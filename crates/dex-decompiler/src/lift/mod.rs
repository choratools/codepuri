//! Stage 2.2/2.3: Dalvik-to-managed-IR lifting, normalization, and CFG build.
//!
//! Structural model: every lifted instruction owns exactly one CFG block
//! whose payload is its instruction identity, matching the `MethodIr`
//! contract that each block carries one [`InsnId`]. Entry, branch and
//! switch targets, conditional fallthroughs, handler entries, and try
//! boundaries therefore all coincide with block boundaries. Exceptional
//! edges connect potentially throwing instructions inside a try range to each handler
//! target, typed handlers named by their DEX type index and catch-all
//! handlers by the empty string. Instructions consumed by fusion (a
//! `move-result*` fused into its invoke) and payload pseudo-instructions
//! never become blocks. Guarded destination writes use a synthetic normal-only
//! commit block so handlers retain the pre-instruction register state.

mod cfg_build;
mod data_map;
mod flow_map;
mod insn_map;
mod lower;
mod lowering;
mod op_maps;
mod payloads;
mod values;

pub(crate) use lowering::SYNTHETIC_TYPE_BASE;

use std::collections::HashMap;

use decomp_core::cfg::CfgError;
use dex_format::code::CodeItem;
use thiserror::Error;

use crate::decode::RecoveredMethod;
use managed_ir::method::{MethodIr, RegisterForm};
use managed_ir::value::InsnId;

/// Why a method could not be lifted into managed IR.
#[derive(Debug, Clone, Error, Eq, PartialEq)]
pub enum LiftError {
    /// A filled allocation requires a resolved legal category-one array descriptor.
    #[error("invalid filled-array type {type_index} at {offset}: {reason}")]
    InvalidFilledArray {
        /// DEX instruction offset.
        offset: u32,
        /// DEX type pool index.
        type_index: u32,
        /// Descriptor validation failure.
        reason: String,
    },
    /// The recovering decoder hit its error budget, so the instruction
    /// stream is incomplete beyond recovery and lifting refuses to guess.
    #[error("decode aborted after {budget} budgeted failures")]
    DecodeAborted {
        /// Number of budgeted decode failures before the walk stopped.
        budget: usize,
    },
    /// A control-flow graph mutation or validation failed.
    #[error("cfg construction failed: {0}")]
    Cfg(#[from] CfgError),
}

/// A lifted method: managed register-form IR plus DEX provenance.
///
/// Each CFG block carries exactly one instruction identity; [`Self::offset`]
/// maps that identity back to its DEX code-unit offset. [`Self::try_ranges`]
/// retains the protected ranges so downstream passes can refuse to fuse
/// across a try boundary even when no handler edge was materialized.
#[derive(Debug)]
pub struct LiftedMethod {
    ir: MethodIr<RegisterForm>,
    offsets: HashMap<InsnId, u32>,
    try_ranges: Vec<(u32, u32)>,
    /// Per-value width categories the defining opcodes fixed. Consumed by
    /// SSA construction to keep register lineages width-pure.
    width_claims: HashMap<managed_ir::value::ValueId, managed_ir::types::ValueCategory>,
}

impl LiftedMethod {
    /// Returns the managed register-form IR.
    pub const fn ir(&self) -> &MethodIr<RegisterForm> {
        &self.ir
    }

    /// Consumes the lift result into its managed IR.
    pub fn into_ir(self) -> MethodIr<RegisterForm> {
        self.ir
    }

    /// Returns the DEX code-unit offset of an instruction, if lifted.
    pub fn offset(&self, insn: InsnId) -> Option<u32> {
        self.offsets.get(&insn).copied()
    }

    /// Returns the protected code-unit ranges as `(start, end)` pairs.
    pub fn try_ranges(&self) -> &[(u32, u32)] {
        &self.try_ranges
    }

    /// Consumes the per-value width categories the defining opcodes fixed.
    pub fn into_width_claims(
        self,
    ) -> HashMap<managed_ir::value::ValueId, managed_ir::types::ValueCategory> {
        self.width_claims
    }
}

/// Decodes and lifts one method body.
///
/// # Errors
/// Returns [`LiftError::DecodeAborted`] when recovery decoding exhausts its
/// budget and [`LiftError::Cfg`] when graph construction fails.
pub fn lift_method(code_item: &CodeItem) -> Result<LiftedMethod, LiftError> {
    let recovered = crate::decode_method_recovering(code_item, crate::decode::DEFAULT_ERROR_LIMIT);
    lift_recovered(&recovered, code_item)
}

/// Lifts an already decoded method body.
///
/// # Errors
/// Same contract as [`lift_method`].
pub fn lift_recovered(
    recovered: &RecoveredMethod,
    code_item: &CodeItem,
) -> Result<LiftedMethod, LiftError> {
    lift_recovered_with(recovered, code_item, &crate::infer::NoTypes)
}

pub(crate) fn lift_recovered_with(
    recovered: &RecoveredMethod,
    code_item: &CodeItem,
    ctx: &dyn crate::infer::TypeContext,
) -> Result<LiftedMethod, LiftError> {
    // A stream that exhausted its recovery budget is incomplete beyond
    // trust: the walk may stop at the top of the loop or exit cleanly after
    // the final failure, so both signals are checked.
    if recovered.aborted || recovered.budget_used >= crate::decode::DEFAULT_ERROR_LIMIT {
        return Err(LiftError::DecodeAborted {
            budget: recovered.budget_used,
        });
    }
    lower::lift_stream(recovered, code_item, ctx)
}
