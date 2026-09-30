//! Stage 2.4a: fusion of lifted one-instruction blocks into straight-line
//! chains.
//!
//! The lifter documents a strict managed-ir contract: every `MethodIr` block
//! carries exactly one [`InsnId`], so multiple instructions can never share a
//! block in a `Cfg<InsnId>`. Fusing chains therefore cannot extend the
//! published IR shape. This module instead builds an *internal*
//! dex-decompiler representation — a `Cfg<Vec<InsnId>>` whose payload is the
//! ordered instruction chain of one fused basic block — which feeds the SSA
//! construction. The published `MethodIr<SsaForm>` re-materializes one block
//! per SSA instruction as managed-ir requires; the chain graph is the
//! analysis substrate and the block-count metric reported by the sweep.
//!
//! Fusion rule: chain `A` absorbs successor chain `B` when
//! - `A` ends in a non-terminating instruction with exactly one outgoing
//!   edge, and that edge is unconditional;
//! - `B` has exactly one incoming edge (so nothing else branches to it and
//!   no exceptional edge enters it);
//! - `A` and `B` lie in the same set of protected try ranges, so no try
//!   boundary falls between them;
//! - the two are not already the same chain (self-loops stay intact).
//!
//! A chain ending in a pure `goto` (unconditional `Branch` with no
//! condition) absorbs its successor by dropping the redundant `goto`
//! instruction; its edge becomes chain-internal adjacency. Every other
//! terminator (`Switch`, conditional `Branch`, `Return`, `Throw`, opaque
//! with `TERMINATES`) ends its chain. Instruction semantics, edge kinds, and
//! branch targets are preserved; only chain-internal edges disappear.
//!
//! Entry-unreachable chains are dead code and are reported separately; they
//! are dropped before SSA construction.

mod fuse;

use std::collections::HashMap;

use decomp_core::cfg::Cfg;
use decomp_core::ids::{BlockId, EdgeId};
use managed_ir::effects::EffectFlags;
use managed_ir::insn::ManagedInsn;
use managed_ir::method::{InsnArena, MethodIr, RegisterForm, RegisterSlotArena, ValueArena};
use managed_ir::value::InsnId;

use crate::lift::LiftedMethod;

/// Why chain fusion could not complete.
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum MergeError {
    /// A live edge referenced by the lifted CFG disappeared.
    #[error("edge {0:?} vanished during chain fusion")]
    MissingEdge(EdgeId),
    /// A live block referenced by the lifted CFG disappeared.
    #[error("block {0:?} vanished during chain fusion")]
    MissingBlock(BlockId),
    /// A control-flow graph mutation failed.
    #[error("cfg mutation failed: {0}")]
    Cfg(#[from] decomp_core::cfg::CfgError),
}

/// The fused-chain control-flow graph over lifted instructions.
///
/// This is an internal dex-decompiler representation feeding SSA
/// construction; see the [module docs](self) for why it is not a
/// `MethodIr` block shape.
#[derive(Debug)]
pub struct MergedMethod {
    pub(crate) cfg: Cfg<Vec<InsnId>>,
    pub(crate) insns: InsnArena,
    pub(crate) values: ValueArena,
    pub(crate) reg_slots: RegisterSlotArena,
    pub(crate) offsets: HashMap<InsnId, u32>,
    /// Per-value width categories the lifter's opcodes fixed; see
    /// [`crate::lift::LiftedMethod::into_width_claims`].
    pub(crate) width_claims: HashMap<managed_ir::value::ValueId, managed_ir::types::ValueCategory>,
    pub(crate) lifted_blocks: usize,
    pub(crate) dropped_gotos: usize,
    pub(crate) dead_blocks: usize,
    pub(crate) dead_insns: usize,
}

impl MergedMethod {
    /// Returns the fused-chain control-flow graph.
    pub const fn cfg(&self) -> &Cfg<Vec<InsnId>> {
        &self.cfg
    }

    /// Returns the lifted instruction arena shared by all chains.
    pub const fn instructions(&self) -> &InsnArena {
        &self.insns
    }

    /// Returns the one-instruction block count before fusion.
    pub const fn lifted_blocks(&self) -> usize {
        self.lifted_blocks
    }

    /// Returns the number of redundant `goto` instructions removed by
    /// fusion.
    pub const fn dropped_gotos(&self) -> usize {
        self.dropped_gotos
    }

    /// Returns the number of entry-unreachable chains removed before SSA.
    pub const fn dead_blocks(&self) -> usize {
        self.dead_blocks
    }

    /// Returns the number of instructions inside removed dead chains.
    pub const fn dead_insns(&self) -> usize {
        self.dead_insns
    }
}

/// Fuses the lifted one-instruction blocks of a method into chains.
///
/// # Errors
/// Returns [`MergeError`] when the lifted graph references a missing
/// block or edge; fusion itself is best-effort and never fails.
pub fn merge_method(lifted: LiftedMethod) -> Result<MergedMethod, MergeError> {
    fuse::fuse(lifted)
}

/// Returns whether an instruction forces its chain to end.
///
/// A pure unconditional `goto` ([`ManagedInsn::Branch`] with no condition)
/// is deliberately *not* hard: fusion may absorb across it and drop the
/// instruction, because its transfer becomes chain-internal adjacency.
pub(crate) fn is_hard_terminator(insn: &ManagedInsn) -> bool {
    match insn {
        ManagedInsn::Branch {
            condition: Some(_), ..
        }
        | ManagedInsn::Switch { .. }
        | ManagedInsn::Return { .. }
        | ManagedInsn::Throw { .. } => true,
        ManagedInsn::Opaque { effects, .. } => effects.contains(EffectFlags::TERMINATES),
        ManagedInsn::Branch {
            condition: None, ..
        }
        | ManagedInsn::Const { .. }
        | ManagedInsn::Move { .. }
        | ManagedInsn::Phi { .. }
        | ManagedInsn::Arithmetic { .. }
        | ManagedInsn::Compare { .. }
        | ManagedInsn::Neg { .. }
        | ManagedInsn::Not { .. }
        | ManagedInsn::FieldGet { .. }
        | ManagedInsn::FieldPut { .. }
        | ManagedInsn::ArrayGet { .. }
        | ManagedInsn::ArrayPut { .. }
        | ManagedInsn::ArrayLength { .. }
        | ManagedInsn::NewInstance { .. }
        | ManagedInsn::NewArray { .. }
        | ManagedInsn::FillArray { .. }
        | ManagedInsn::Invoke { .. }
        | ManagedInsn::Cast { .. }
        | ManagedInsn::InstanceOf { .. }
        | ManagedInsn::MonitorEnter { .. }
        | ManagedInsn::MonitorExit { .. } => false,
    }
}

/// Returns whether an instruction is a pure unconditional `goto`, which
/// fusion may drop when it transfers into its own chain successor.
pub(crate) fn is_pure_goto(insn: &ManagedInsn) -> bool {
    matches!(
        insn,
        ManagedInsn::Branch {
            condition: None,
            ..
        }
    )
}

/// Computes, per lifted block, the bitmask of try ranges containing it.
///
/// The mask identity of two adjacent blocks is the try-boundary test used by
/// fusion; overlapping or malformed ranges degrade to stricter separation,
/// never looser.
pub(crate) fn try_masks(
    lifted: &LiftedMethod,
    block_offset: &HashMap<BlockId, u32>,
) -> HashMap<BlockId, u64> {
    let ranges = lifted.try_ranges();
    let mut masks = HashMap::with_capacity(block_offset.len());
    for (block, &offset) in block_offset {
        let mut mask = 0_u64;
        for (index, &(start, end)) in ranges.iter().enumerate().take(u64::BITS as usize) {
            if offset >= start && offset < end {
                mask |= 1_u64 << index;
            }
        }
        masks.insert(*block, mask);
    }
    masks
}

/// Copies the lifted arenas by value, preserving every identity.
pub(crate) fn clone_arenas(lifted: &MethodIr<RegisterForm>) -> (InsnArena, ValueArena) {
    (lifted.instructions().clone(), lifted.values().clone())
}

/// Copies register-slot histories, preserving every identity.
pub(crate) fn clone_reg_slots(lifted: &MethodIr<RegisterForm>) -> RegisterSlotArena {
    lifted.register_slots().clone()
}
