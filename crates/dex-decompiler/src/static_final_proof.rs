//! Compact raw-lift evidence for the Java static-final definite-assignment proof.

use std::collections::HashMap;

use decomp_core::cfg::EdgeKind;
use decomp_core::ids::BlockId;
use dex_format::{DexFile, code::CodeItem};
use managed_ir::insn::ManagedInsn;

use crate::lift::LiftedMethod;

/// Instruction facts needed to walk an original static-initializer path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofInstruction {
    /// An instruction that does not directly affect this proof's state.
    Other,
    /// A field write, retaining its field identity and receiver category.
    FieldPut {
        /// Field pool index.
        field: u32,
        /// Whether the raw access has no instance receiver.
        is_static: bool,
    },
    /// A field read, retaining its field identity and receiver category.
    FieldGet {
        /// Field pool index.
        field: u32,
        /// Whether the raw access has no instance receiver.
        is_static: bool,
    },
    /// A void return.
    ReturnVoid,
    /// A throw instruction.
    Throw,
}

/// One normal or exceptional successor in original CFG insertion order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProofEdge {
    target: u32,
    exceptional: bool,
}

impl ProofEdge {
    /// Target block index in this summary.
    pub fn target(self) -> usize {
        self.target as usize
    }

    /// Whether this is an exceptional edge to a handler root.
    pub const fn is_exceptional(self) -> bool {
        self.exceptional
    }
}

/// One original raw field access, in instruction-arena iteration order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProofFieldSite {
    field: u32,
    is_static: bool,
    offset: u32,
}

impl ProofFieldSite {
    /// Field pool index.
    pub const fn field(self) -> u32 {
        self.field
    }

    /// Whether the raw access has no instance receiver.
    pub const fn is_static(self) -> bool {
        self.is_static
    }

    /// Original DEX code-unit offset.
    pub const fn offset(self) -> u32 {
        self.offset
    }
}

#[derive(Clone, Copy, Debug)]
struct ProofBlock {
    instruction: ProofInstruction,
    edge_start: u32,
    edge_count: u32,
}

/// Bounded projection of the original lifted `<clinit>` CFG and field sites.
///
/// This owns no IR values or DEX borrows. It is safe to retain with an SSA
/// cache entry until that export's normal cache-release lifecycle runs.
#[derive(Clone, Debug)]
pub struct StaticFinalProofSummary {
    dex_identity: usize,
    method_index: u32,
    code_identity: usize,
    entry: u32,
    blocks: Vec<ProofBlock>,
    edges: Vec<ProofEdge>,
    field_puts: Vec<ProofFieldSite>,
    field_gets: Vec<ProofFieldSite>,
}

impl StaticFinalProofSummary {
    pub(crate) fn capture(
        lifted: &LiftedMethod,
        dex_identity: usize,
        method_index: u32,
        code: &CodeItem,
    ) -> Option<Self> {
        let ir = lifted.ir();
        let cfg = ir.cfg();
        let block_ids: Vec<_> = cfg.block_ids().collect();
        let mut indexes = HashMap::<BlockId, u32>::with_capacity(block_ids.len());
        for (index, block) in block_ids.iter().copied().enumerate() {
            let index = u32::try_from(index).ok()?;
            if indexes.insert(block, index).is_some() {
                return None;
            }
        }
        let entry = *indexes.get(&cfg.entry())?;
        let mut blocks = Vec::with_capacity(block_ids.len());
        let mut edges = Vec::new();
        for block_id in block_ids {
            let block = cfg.block(block_id).ok()?;
            let instruction = ir.instruction(*block.payload()).ok()?;
            let proof_instruction = match instruction {
                ManagedInsn::FieldPut { field, object, .. } => ProofInstruction::FieldPut {
                    field: field.index(),
                    is_static: object.is_none(),
                },
                ManagedInsn::FieldGet { field, object, .. } => ProofInstruction::FieldGet {
                    field: field.index(),
                    is_static: object.is_none(),
                },
                ManagedInsn::Return { value: None } => ProofInstruction::ReturnVoid,
                ManagedInsn::Throw { .. } => ProofInstruction::Throw,
                _ => ProofInstruction::Other,
            };
            let edge_start = u32::try_from(edges.len()).ok()?;
            for edge_id in block.outgoing() {
                let edge = cfg.edge(*edge_id).ok()?;
                edges.push(ProofEdge {
                    target: *indexes.get(&edge.target())?,
                    exceptional: matches!(edge.kind(), EdgeKind::Exception(_)),
                });
            }
            let edge_count = u32::try_from(edges.len().checked_sub(edge_start as usize)?).ok()?;
            blocks.push(ProofBlock {
                instruction: proof_instruction,
                edge_start,
                edge_count,
            });
        }

        let mut field_puts = Vec::new();
        let mut field_gets = Vec::new();
        for (id, instruction) in ir.instructions() {
            match instruction {
                ManagedInsn::FieldPut { field, object, .. } => {
                    field_puts.push(ProofFieldSite {
                        field: field.index(),
                        is_static: object.is_none(),
                        offset: lifted.offset(id)?,
                    });
                }
                ManagedInsn::FieldGet { field, object, .. } => {
                    field_gets.push(ProofFieldSite {
                        field: field.index(),
                        is_static: object.is_none(),
                        offset: lifted.offset(id)?,
                    });
                }
                _ => {}
            }
        }

        Some(Self {
            dex_identity,
            method_index,
            code_identity: std::ptr::from_ref(code) as usize,
            entry,
            blocks,
            edges,
            field_puts,
            field_gets,
        })
    }

    /// Whether this summary belongs to the exact original DEX method body.
    pub fn matches_original(&self, dex: &DexFile, method_index: u32, code: &CodeItem) -> bool {
        self.dex_identity == std::ptr::from_ref(dex) as usize
            && self.method_index == method_index
            && self.code_identity == std::ptr::from_ref(code) as usize
    }

    /// Entry block index.
    pub fn entry(&self) -> usize {
        self.entry as usize
    }

    /// Raw `FieldPut` sites in original instruction iteration order.
    pub fn field_puts(&self) -> &[ProofFieldSite] {
        &self.field_puts
    }

    /// Raw `FieldGet` sites and offsets in original instruction iteration order.
    pub fn field_gets(&self) -> &[ProofFieldSite] {
        &self.field_gets
    }

    /// Instruction and ordered successors for a summary block.
    pub fn block(&self, index: usize) -> Option<(ProofInstruction, &[ProofEdge])> {
        let record = *self.blocks.get(index)?;
        let start = usize::try_from(record.edge_start).ok()?;
        let end = start.checked_add(usize::try_from(record.edge_count).ok()?)?;
        Some((record.instruction, self.edges.get(start..end)?))
    }

    /// Estimated retained bytes, including vector capacities and this value's
    /// inline storage; excludes the owning `SsaMethod`'s optional-field tag.
    pub fn estimated_size_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.blocks.capacity() * std::mem::size_of::<ProofBlock>()
            + self.edges.capacity() * std::mem::size_of::<ProofEdge>()
            + self.field_puts.capacity() * std::mem::size_of::<ProofFieldSite>()
            + self.field_gets.capacity() * std::mem::size_of::<ProofFieldSite>()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{DexPasses, TypeContext};
    use decomp_core::ids::BlockId;
    use dex_format::{RawSpan, code::EncodedCatchHandler, code::TryItem};
    use managed_ir::types::ManagedType;

    struct StaticInitializerContext;

    impl TypeContext for StaticInitializerContext {
        fn original_static_initializer_identity(&self) -> Option<(usize, u32)> {
            Some((0x1234, 7))
        }

        fn initial_register_type(&self, _: u16, _: bool) -> Option<ManagedType> {
            None
        }
    }

    fn code(with_handler: bool) -> CodeItem {
        let units: &[u16] = if with_handler {
            &[0x0060, 0, 0x0067, 0, 0x000e, 0x000d, 0x000e]
        } else {
            &[0x0060, 0, 0x0067, 0, 0x000e]
        };
        CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            debug_info_off: 0,
            insns_size: units.len() as u32,
            insns: units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
            tries: with_handler
                .then_some(TryItem {
                    start_addr: 0,
                    insn_count: 2,
                    handler_off: 0,
                    span: RawSpan { offset: 0, size: 8 },
                })
                .into_iter()
                .collect(),
            handlers: with_handler
                .then_some(EncodedCatchHandler {
                    relative_offset: 0,
                    handlers: vec![],
                    catch_all_addr: Some(5),
                    span: RawSpan { offset: 0, size: 0 },
                })
                .into_iter()
                .collect(),
            span: RawSpan {
                offset: 0,
                size: (units.len() * 2) as u32,
            },
        }
    }

    fn verify_summary_matches_raw_lift(code: &CodeItem) {
        let ssa = DexPasses::run_ssa_with(code, &StaticInitializerContext).unwrap();
        let summary = ssa.original_static_final_proof().unwrap();
        let raw = DexPasses::run(code).unwrap();
        let cfg = raw.ir().cfg();
        let block_ids: Vec<BlockId> = cfg.block_ids().collect();
        let indexes: HashMap<_, _> = block_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        assert_eq!(summary.entry(), indexes[&cfg.entry()]);
        assert_eq!(summary.block(block_ids.len()), None);

        for (index, block_id) in block_ids.iter().copied().enumerate() {
            let block = cfg.block(block_id).unwrap();
            let insn = raw.ir().instruction(*block.payload()).unwrap();
            let expected = match insn {
                ManagedInsn::FieldPut { field, object, .. } => ProofInstruction::FieldPut {
                    field: field.index(),
                    is_static: object.is_none(),
                },
                ManagedInsn::FieldGet { field, object, .. } => ProofInstruction::FieldGet {
                    field: field.index(),
                    is_static: object.is_none(),
                },
                ManagedInsn::Return { value: None } => ProofInstruction::ReturnVoid,
                ManagedInsn::Throw { .. } => ProofInstruction::Throw,
                _ => ProofInstruction::Other,
            };
            let (actual, edges) = summary.block(index).unwrap();
            assert_eq!(actual, expected);
            let expected_edges: Vec<_> = block
                .outgoing()
                .iter()
                .map(|edge_id| {
                    let edge = cfg.edge(*edge_id).unwrap();
                    (
                        indexes[&edge.target()],
                        matches!(edge.kind(), EdgeKind::Exception(_)),
                    )
                })
                .collect();
            let actual_edges: Vec<_> = edges
                .iter()
                .map(|edge| (edge.target(), edge.is_exceptional()))
                .collect();
            assert_eq!(actual_edges, expected_edges);
        }

        let mut expected_puts = Vec::new();
        let mut expected_gets = Vec::new();
        for (id, insn) in raw.ir().instructions() {
            match insn {
                ManagedInsn::FieldPut { field, object, .. } => {
                    expected_puts.push((field.index(), object.is_none(), raw.offset(id).unwrap()))
                }
                ManagedInsn::FieldGet { field, object, .. } => {
                    expected_gets.push((field.index(), object.is_none(), raw.offset(id).unwrap()))
                }
                _ => {}
            }
        }
        let actual_puts: Vec<_> = summary
            .field_puts()
            .iter()
            .map(|site| (site.field(), site.is_static(), site.offset()))
            .collect();
        let actual_gets: Vec<_> = summary
            .field_gets()
            .iter()
            .map(|site| (site.field(), site.is_static(), site.offset()))
            .collect();
        assert_eq!(actual_puts, expected_puts);
        assert_eq!(actual_gets, expected_gets);
        assert!(summary.estimated_size_bytes() >= std::mem::size_of_val(summary));
    }

    #[test]
    fn summary_preserves_normal_cfg_and_field_provenance() {
        verify_summary_matches_raw_lift(&code(false));
    }

    #[test]
    fn summary_preserves_exception_edges_and_handler_roots() {
        verify_summary_matches_raw_lift(&code(true));
    }
}
