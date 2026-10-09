use std::collections::{HashMap, HashSet};

use decomp_core::analysis::dominators;
use decomp_core::diagnostics::Diagnostic;
use decomp_core::ids::BlockId;

use super::semantics::UseLocation;
use super::{Layout, error_diagnostic, semantics};
use crate::method::{MethodIr, SsaForm};
use crate::types::{ManagedType, ValueCategory};
use crate::value::ValueId;

/// Consecutive phis form one simultaneous join. Only a unique unconditional
/// edge from a phi with a unique successor can extend its cluster: branches
/// and additional entries must start a new join.
fn phi_predecessor(method: &MethodIr<SsaForm>, block: BlockId) -> Option<BlockId> {
    let record = method.cfg().block(block).ok()?;
    let [incoming] = record.incoming() else {
        return None;
    };
    let edge = method.cfg().edge(*incoming).ok()?;
    if edge.kind() != &decomp_core::cfg::EdgeKind::Unconditional {
        return None;
    }
    let predecessor = method.cfg().block(edge.source()).ok()?;
    if predecessor.outgoing().len() != 1
        || !method
            .instructions()
            .get(*predecessor.payload())
            .is_some_and(|insn| matches!(insn, crate::insn::ManagedInsn::Phi { .. }))
    {
        return None;
    }
    Some(edge.source())
}

/// Memoize the functional predecessor graph. A closed phi cycle retains the
/// old walk's start-relative head: every cycle member is its own head.
fn phi_heads(method: &MethodIr<SsaForm>) -> HashMap<BlockId, BlockId> {
    let mut heads = HashMap::new();
    let mut path = Vec::new();
    let mut positions = HashMap::new();
    for start in method.cfg().block_ids() {
        if heads.contains_key(&start) {
            continue;
        }
        path.clear();
        positions.clear();
        let mut block = start;
        let head = loop {
            if let Some(&head) = heads.get(&block) {
                break head;
            }
            if let Some(&position) = positions.get(&block) {
                for &member in &path[position..] {
                    heads.insert(member, member);
                }
                path.truncate(position);
                break block;
            }
            positions.insert(block, path.len());
            path.push(block);
            match phi_predecessor(method, block) {
                Some(parent) => block = parent,
                None => break block,
            }
        };
        for &member in &path {
            heads.insert(member, head);
        }
    }
    heads
}

/// The shared core tree already provides interval queries. Retain one analysis
/// rather than constructing a second verifier-specific interval tree.
struct DominanceIndex {
    tree: decomp_core::analysis::Dominators,
}

impl DominanceIndex {
    fn build(method: &MethodIr<SsaForm>) -> Self {
        Self {
            tree: dominators(method.cfg()),
        }
    }

    fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        self.tree.dominates(dominator, block)
    }
}

pub(super) fn verify(
    method: &MethodIr<SsaForm>,
    layout: &Layout,
    allow_conflict_values: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let heads = phi_heads(method);
    verify_phi_shapes(method, layout, &heads, diagnostics);
    verify_uses(method, layout, &heads, allow_conflict_values, diagnostics);
    verify_widths(method, diagnostics);
    verify_register_widths(method, diagnostics);
}

fn verify_phi_shapes(
    method: &MethodIr<SsaForm>,
    layout: &Layout,
    heads: &HashMap<BlockId, BlockId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut expected_edges = HashMap::<BlockId, HashSet<_>>::new();
    let mut actual = HashSet::new();
    for (insn_id, insn) in method.instructions() {
        let Some(inputs) = semantics::phi_inputs(insn) else {
            continue;
        };
        let Some(block_id) = layout.block(insn_id) else {
            continue;
        };
        let head = heads.get(&block_id).copied().unwrap_or(block_id);
        let block = match method.cfg().block(head) {
            Ok(block) => block,
            Err(error) => {
                diagnostics.push(error_diagnostic(
                    "MIR_CFG_INVALID",
                    format!("cannot read phi block {block_id:?}: {error}"),
                ));
                continue;
            }
        };
        if inputs.len() != block.incoming().len() {
            diagnostics.push(error_diagnostic(
                "MIR_SSA_PHI_ARITY",
                format!(
                    "phi {insn_id:?} has {} inputs for {} predecessor edges",
                    inputs.len(),
                    block.incoming().len()
                ),
            ));
        }
        let expected = expected_edges
            .entry(head)
            .or_insert_with(|| block.incoming().iter().copied().collect());
        actual.clear();
        actual.extend(inputs.iter().map(|(edge, _)| *edge));
        if *expected != actual || actual.len() != inputs.len() {
            diagnostics.push(error_diagnostic(
                "MIR_SSA_PHI_EDGES",
                format!("phi {insn_id:?} inputs do not match incoming edge identities"),
            ));
        }
    }
}

fn verify_uses(
    method: &MethodIr<SsaForm>,
    layout: &Layout,
    heads: &HashMap<BlockId, BlockId>,
    allow_conflict_values: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let dominator_tree = DominanceIndex::build(method);
    for (insn_id, insn) in method.instructions() {
        let use_block = layout.block(insn_id);
        semantics::for_each_use(insn, |value_id, location| {
            let Some(value) = method.values().get(value_id) else {
                diagnostics.push(error_diagnostic(
                    "MIR_SSA_MISSING_VALUE",
                    format!("instruction {insn_id:?} uses missing value {value_id:?}"),
                ));
                return;
            };
            if !allow_conflict_values && value.ty() == &ManagedType::Conflict {
                diagnostics.push(error_diagnostic(
                    "MIR_SSA_CONFLICT_USE",
                    format!("instruction {insn_id:?} uses conflict value {value_id:?}"),
                ));
            }
            let (Some(def_block), Some(use_block)) = (layout.block(value.def()), use_block) else {
                return;
            };
            let dominated = match location {
                UseLocation::Instruction => {
                    def_block != use_block && dominator_tree.dominates(def_block, use_block)
                }
                UseLocation::PhiEdge(edge_id) => match method.cfg().edge(edge_id) {
                    Ok(edge)
                        if edge.target() == heads.get(&use_block).copied().unwrap_or(use_block) =>
                    {
                        let source = edge.source();
                        def_block == source || dominator_tree.dominates(def_block, source)
                    }
                    Ok(_) | Err(_) => false,
                },
            };
            if !dominated {
                diagnostics.push(error_diagnostic(
                    "MIR_SSA_DOMINANCE",
                    format!("definition of {value_id:?} does not dominate its use in {insn_id:?}"),
                ));
            }
        });
    }
}

fn verify_widths(method: &MethodIr<SsaForm>, diagnostics: &mut Vec<Diagnostic>) {
    for (insn_id, insn) in method.instructions() {
        semantics::for_each_equal_width(insn, |left, right| {
            let categories = categories(method, left, right);
            if let Some((left_category, right_category)) = categories
                && left_category != right_category
            {
                diagnostics.push(wide_diagnostic(insn_id, left, right));
            }
        });
        semantics::for_each_category_one(insn, |value_id| {
            if category(method, value_id) == Some(ValueCategory::Two) {
                diagnostics.push(wide_diagnostic(insn_id, value_id, value_id));
            }
        });
    }
}

fn verify_register_widths(method: &MethodIr<SsaForm>, diagnostics: &mut Vec<Diagnostic>) {
    let mut registers = HashMap::<ValueId, HashSet<u16>>::new();
    for (_, slot) in method.register_slots() {
        for value in slot.values() {
            registers.entry(*value).or_default().insert(slot.index());
        }
    }
    for (value_id, indices) in registers {
        if category(method, value_id) != Some(ValueCategory::Two) {
            continue;
        }
        let mut indices: Vec<_> = indices.into_iter().collect();
        indices.sort_unstable();
        let intact = matches!(indices.as_slice(), [low, high] if low.checked_add(1) == Some(*high));
        if !intact {
            diagnostics.push(error_diagnostic(
                "MIR_SSA_WIDE_SPLIT",
                format!("wide value {value_id:?} is not represented by one adjacent register pair"),
            ));
        }
    }
}

fn categories(
    method: &MethodIr<SsaForm>,
    left: ValueId,
    right: ValueId,
) -> Option<(ValueCategory, ValueCategory)> {
    Some((category(method, left)?, category(method, right)?))
}

fn category(method: &MethodIr<SsaForm>, value: ValueId) -> Option<ValueCategory> {
    method
        .values()
        .get(value)
        .and_then(|value| value.ty().category())
}

fn wide_diagnostic(insn: crate::value::InsnId, left: ValueId, right: ValueId) -> Diagnostic {
    error_diagnostic(
        "MIR_SSA_WIDE_SPLIT",
        format!("instruction {insn:?} splits value widths between {left:?} and {right:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insn::ManagedInsn;
    use crate::method::{InsnArena, ValueArena};
    use crate::value::Value;
    use decomp_core::cfg::{Cfg, EdgeKind};

    fn must<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("fixture error: {error:?}"),
        }
    }

    fn phi(insns: &mut InsnArena, values: &mut ValueArena) -> crate::value::InsnId {
        let insn = insns.insert(ManagedInsn::Opaque {
            family_tag: 0,
            reads: Vec::new(),
            writes: Vec::new(),
            effects: crate::effects::EffectFlags::NONE,
        });
        let result = values.insert(Value::new(insn, ManagedType::Int));
        insns[insn] = ManagedInsn::Phi {
            result,
            inputs: Vec::new(),
        };
        insn
    }

    #[test]
    fn interval_queries_match_parent_walks_and_count_chain_work() {
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let mut cfg = Cfg::new(phi(&mut insns, &mut values));
        let mut blocks = vec![cfg.entry()];
        for _ in 1..512 {
            let block = cfg.add_block(phi(&mut insns, &mut values));
            must(cfg.add_edge(blocks[blocks.len() - 1], block, EdgeKind::Unconditional));
            blocks.push(block);
        }
        let unreachable = cfg.add_block(phi(&mut insns, &mut values));
        let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let reference = dominators(method.cfg());
        let index = DominanceIndex::build(&method);
        for &left in blocks
            .iter()
            .step_by(32)
            .chain(std::iter::once(&unreachable))
        {
            for &right in blocks.iter().chain(std::iter::once(&unreachable)) {
                assert_eq!(
                    index.dominates(left, right),
                    reference.dominates(left, right)
                );
            }
        }
        let mut parent_steps = 0;
        for &block in &blocks[1..] {
            let mut cursor = block;
            while let Some(parent) = reference.immediate_dominator(cursor) {
                parent_steps += 1;
                cursor = parent;
            }
        }
        assert_eq!(parent_steps, 130_816);
        assert_eq!(
            method
                .cfg()
                .block_ids()
                .filter(|&block| index.dominates(method.cfg().entry(), block))
                .count(),
            512
        );
        eprintln!(
            "512-block chain: {parent_steps} old parent steps; 512 indexed nodes, 511 constant-time queries"
        );
    }

    #[test]
    fn cached_phi_heads_match_walks_for_clusters_and_cycles() {
        for cycle in [false, true] {
            let mut insns = InsnArena::with_key();
            let mut values = ValueArena::with_key();
            let mut cfg = Cfg::new(phi(&mut insns, &mut values));
            let mut blocks = vec![cfg.entry()];
            for i in 1..128 {
                let block = cfg.add_block(phi(&mut insns, &mut values));
                let kind = if !cycle && i == 64 {
                    EdgeKind::BranchTrue
                } else {
                    EdgeKind::Unconditional
                };
                must(cfg.add_edge(blocks[i - 1], block, kind));
                blocks.push(block);
            }
            if cycle {
                must(cfg.add_edge(blocks[127], blocks[0], EdgeKind::Unconditional));
            }
            let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
            let heads = phi_heads(&method);
            for start in blocks {
                let mut seen = HashSet::new();
                let mut block = start;
                while seen.insert(block) {
                    match phi_predecessor(&method, block) {
                        Some(parent) => block = parent,
                        None => break,
                    }
                }
                assert_eq!(heads[&start], block);
            }
        }
    }
}
