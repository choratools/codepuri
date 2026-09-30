//! Explicit ordered runner for the DEX frontend stages.
//!
//! [`decomp_core::pipeline::Registry`] only schedules declarative
//! [`Pass`](decomp_core::pipeline::Pass) metadata; the trait carries no
//! execution method, so until core grows an execution harness the frontend
//! owns explicit orchestration here. Stage order is fixed:
//! 2.2 lift + normalize into managed register form, 2.3 CFG construction
//! and validation (both inside [`lift_method`]), 2.4a chain fusion,
//! 2.4b SSA construction + verification, and 2.4c type inference (inside
//! [`to_ssa_with`], re-verified after inference).

use dex_format::code::CodeItem;

use crate::TypeContext;
use crate::lift::{LiftError, LiftedMethod};
use crate::ssa::{SsaError, SsaMethod, to_ssa_with};

/// Ordered executor for the DEX frontend stages over one method body.
#[derive(Debug, Clone, Copy, Default)]
pub struct DexPasses;

impl DexPasses {
    /// Runs lift (2.2), normalization, and CFG construction + validation
    /// (2.3) for one method.
    ///
    /// # Errors
    /// Returns the first stage failure: decode-abort or CFG error.
    pub fn run(code_item: &CodeItem) -> Result<LiftedMethod, LiftError> {
        let lifted = crate::lift_method(code_item)?;
        lifted.ir().cfg().validate()?;
        Ok(lifted)
    }

    /// Runs lift (2.2/2.3), chain fusion (2.4a), SSA construction and
    /// verification (2.4b) for one method.
    ///
    /// # Errors
    /// Returns the first failing stage: decode-abort, CFG error, merge
    /// failure, structural failure, or verifier rejection.
    pub fn run_merged(code_item: &CodeItem) -> Result<SsaMethod, StageError> {
        Self::run_ssa_with(code_item, &crate::infer::NoTypes)
    }

    /// Runs the full 2.4 pipeline with a descriptor context, so type
    /// inference sees field, method, and return descriptors.
    ///
    /// # Errors
    /// Same contract as [`Self::run_merged`]; inference diagnostics are
    /// attached to the returned [`SsaMethod`] instead of failing it.
    pub fn run_ssa_with(
        code_item: &CodeItem,
        ctx: &dyn TypeContext,
    ) -> Result<SsaMethod, StageError> {
        let recovered =
            crate::decode_method_recovering(code_item, crate::decode::DEFAULT_ERROR_LIMIT);
        let lifted = crate::lift::lift_recovered_with(&recovered, code_item, ctx)?;
        let merged = crate::merge_method(lifted)?;
        let ssa = to_ssa_with(merged, ctx)?;
        Ok(ssa)
    }
}

/// Any failure across the 2.2–2.4 frontend stages.
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum StageError {
    /// Lifting failed (decode abort or CFG construction).
    #[error("lift failed: {0}")]
    Lift(#[from] LiftError),
    /// Chain fusion failed.
    #[error("merge failed: {0}")]
    Merge(#[from] crate::merge::MergeError),
    /// SSA construction or verification failed.
    #[error("ssa failed: {0}")]
    Ssa(#[from] SsaError),
}
