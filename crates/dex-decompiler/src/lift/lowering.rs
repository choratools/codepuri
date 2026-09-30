//! Shared lowering context threading the value table through opcode
//! mapping.
//!
//! Wide register pairs are folded into one atomic category-2 value by the
//! [`ValueTable`]; `move-result*` has already been fused by the stream
//! walker, so a surviving `move-result*` opcode is an orphan and degrades to
//! a conservative opaque. Three-way compares (`cmpl-*`, `cmpg-*`,
//! `cmp-long`) have no Boolean-faithful managed form and also degrade.

use managed_ir::effects::EffectFlags;
use managed_ir::insn::ManagedInsn;
use managed_ir::types::ManagedType;
use managed_ir::value::{InsnId, ValueId};

use super::values::ValueTable;
use crate::decode::DecodedInsn;

/// Frontend-owned synthetic `TypeRef` space for primitive conversions, which
/// carry no pool index; real type-pool indices never reach this range.
pub(crate) const SYNTHETIC_TYPE_BASE: u32 = 0x8000_0000;

/// The lowered form of one decoded instruction.
pub(crate) enum Built {
    /// A final managed instruction.
    Managed(ManagedInsn),
    /// A control-transfer instruction finalized after edge allocation.
    Pending(PendingFlow),
}

/// Per-instruction lowering context supplied by the stream walker.
pub(crate) struct Context<'a> {
    /// Decoded instruction being lowered.
    pub(crate) insn: &'a DecodedInsn,
    /// Fused `move-result` destination and whether the result move was the
    /// wide variant, if any.
    pub(crate) fused: Option<FusedResult>,
    /// Handler exception value for `move-exception`, if at a handler entry.
    pub(crate) handler_input: Option<ValueId>,
    /// Folded `fill-array-data` constants, if the payload resolved.
    pub(crate) fill_values: Option<Vec<ValueId>>,
}

/// The `move-result*` fused into a preceding invoke or array fill.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FusedResult {
    /// Destination register.
    pub(crate) dest: u16,
    /// Whether the result move was `move-result-wide`.
    pub(crate) wide: bool,
}

/// Stateful view over one instruction under lowering.
pub(crate) struct Lowering<'a> {
    /// Register-file view and arenas.
    pub(crate) values: &'a mut ValueTable,
    /// Identity of the reserved instruction slot to patch.
    pub(crate) id: InsnId,
    /// Decoded instruction being lowered.
    pub(crate) insn: &'a DecodedInsn,
    /// Dalvik opcode byte.
    pub(crate) byte: u8,
    /// Fused `move-result` destination and width, if any.
    pub(crate) fused: Option<FusedResult>,
    /// Handler exception value for `move-exception`, if any.
    pub(crate) handler_input: Option<ValueId>,
    /// Folded `fill-array-data` constants, if any.
    pub(crate) fill_values: Option<Vec<ValueId>>,
}

impl Lowering<'_> {
    /// Returns the `index`th encoded register, or zero for truncated
    /// adversarial operands.
    pub(crate) fn reg(&self, index: usize) -> u16 {
        self.insn.registers.get(index).copied().unwrap_or(0)
    }

    /// Returns the primary pool index, or zero when absent.
    pub(crate) fn pool(&self) -> u32 {
        self.insn.pool_index.unwrap_or(0)
    }

    /// Returns the decoded literal, or zero when absent.
    pub(crate) fn lit(&self) -> i64 {
        self.insn.literal.unwrap_or(0)
    }

    /// Defines a value in a register pair atomically when `wide`.
    pub(crate) fn define_typed(&mut self, reg: u16, ty: ManagedType, wide: bool) -> ValueId {
        if wide {
            self.values.define_wide(self.id, reg, ty)
        } else {
            self.values.define(self.id, reg, ty)
        }
    }

    /// Defines the fused result value of this instruction, binding the full
    /// register pair for `move-result-wide` and recording the width claim
    /// the result move's opcode fixes.
    pub(crate) fn define_fused(&mut self, ty: ManagedType) -> Option<ValueId> {
        let fused = self.fused?;
        Some(if fused.wide {
            self.values.define_wide(self.id, fused.dest, ty)
        } else {
            self.values.define(self.id, fused.dest, ty)
        })
    }

    /// Builds a conservative opaque with the given dataflow contract.
    pub(crate) fn opaque(
        &self,
        reads: Vec<ValueId>,
        writes: Vec<ValueId>,
        effects: EffectFlags,
    ) -> ManagedInsn {
        ManagedInsn::Opaque {
            family_tag: u16::from(self.byte),
            reads,
            writes,
            effects,
        }
    }

    /// Degrades an orphan result move to a conservative opaque. The result
    /// move's opcode still fixes the width: `move-result-wide` (0x0b) binds
    /// the destination pair.
    pub(crate) fn orphan_result(&mut self, reg: u16) -> Built {
        let writes = if self.byte == 0x0b {
            vec![self
                .values
                .define_wide(self.id, reg, ManagedType::Unknown)]
        } else {
            vec![self.values.define(self.id, reg, ManagedType::Unknown)]
        };
        Built::Managed(self.opaque(Vec::new(), writes, EffectFlags::MAY_THROW))
    }
}
/// A control transfer whose edges are allocated after block creation.
pub(crate) enum PendingFlow {
    /// `return` / `throw`: no normal successors.
    Terminal,
    /// Unconditional transfer to a target offset.
    Goto(u32),
    /// Conditional transfer: predicate, taken target, plus fallthrough.
    Branch {
        /// Synthetic or direct branch predicate.
        condition: ValueId,
        /// Offset targeted when the predicate holds.
        target: u32,
    },
    /// Multi-way dispatch resolved from a switch payload.
    Switch {
        /// Integral selector.
        key: ValueId,
        /// Absolute payload offset, when the decoder validated one.
        payload: Option<u32>,
    },
}

/// One lifted instruction awaiting CFG wiring.
pub(crate) struct LiftedInsn {
    /// DEX code-unit offset.
    pub(crate) offset: u32,
    /// Dalvik opcode byte, retained for opaque degradation.
    pub(crate) opcode: u8,
    /// Arena identity of the reserved managed instruction.
    pub(crate) insn: InsnId,
    /// Control transfer to finalize, if any.
    pub(crate) pending: Option<PendingFlow>,
}

impl LiftedInsn {
    pub(crate) fn new(offset: u32, opcode: u8, insn: InsnId) -> Self {
        Self {
            offset,
            opcode,
            insn,
            pending: None,
        }
    }
}
