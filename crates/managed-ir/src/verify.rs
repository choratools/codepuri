//! Structural verification for managed SSA form.

mod definitions;
mod semantics;
mod uses;

use std::collections::HashMap;

use decomp_core::diagnostics::{Diagnostic, Severity};
use decomp_core::ids::BlockId;

use crate::method::{MethodIr, SsaForm};
use crate::value::InsnId;

/// Checks structural SSA invariants without mutating the method.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SsaVerifier {
    allow_conflict_values: bool,
}

impl SsaVerifier {
    /// Creates a strict verifier that rejects uses of conflict-typed values.
    pub const fn new() -> Self {
        Self {
            allow_conflict_values: false,
        }
    }

    /// Configures whether conflict-typed values may be consumed.
    pub const fn with_conflict_values_allowed(mut self, allowed: bool) -> Self {
        self.allow_conflict_values = allowed;
        self
    }

    /// Returns all structural diagnostics and never panics for invalid IR.
    pub fn verify(&self, method: &MethodIr<SsaForm>) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        if let Err(error) = method.cfg().validate() {
            diagnostics.push(error_diagnostic(
                "MIR_CFG_INVALID",
                format!("invalid control-flow graph: {error}"),
            ));
        }
        let layout = Layout::build(method, &mut diagnostics);
        definitions::verify(method, &layout, &mut diagnostics);
        uses::verify(
            method,
            &layout,
            self.allow_conflict_values,
            &mut diagnostics,
        );
        diagnostics
    }
}

#[derive(Debug, Default)]
struct Layout {
    blocks: HashMap<InsnId, BlockId>,
}

impl Layout {
    fn build(method: &MethodIr<SsaForm>, diagnostics: &mut Vec<Diagnostic>) -> Self {
        let mut layout = Self::default();
        for block_id in method.cfg().block_ids() {
            let block = match method.cfg().block(block_id) {
                Ok(block) => block,
                Err(error) => {
                    diagnostics.push(error_diagnostic(
                        "MIR_CFG_INVALID",
                        format!("cannot read block {block_id:?}: {error}"),
                    ));
                    continue;
                }
            };
            let insn_id = *block.payload();
            if method.instructions().get(insn_id).is_none() {
                diagnostics.push(error_diagnostic(
                    "MIR_SSA_MISSING_INSN",
                    format!("block {block_id:?} references missing instruction {insn_id:?}"),
                ));
                continue;
            }
            if let Some(previous) = layout.blocks.insert(insn_id, block_id) {
                diagnostics.push(error_diagnostic(
                    "MIR_SSA_DUPLICATE_PLACEMENT",
                    format!(
                        "instruction {insn_id:?} is placed in blocks {previous:?} and {block_id:?}"
                    ),
                ));
            }
        }
        for (insn_id, _) in method.instructions() {
            if !layout.blocks.contains_key(&insn_id) {
                diagnostics.push(error_diagnostic(
                    "MIR_SSA_UNPLACED_INSN",
                    format!("instruction {insn_id:?} is not placed in the CFG"),
                ));
            }
        }
        layout
    }

    fn block(&self, insn: InsnId) -> Option<BlockId> {
        self.blocks.get(&insn).copied()
    }
}

fn error_diagnostic(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Severity::Error, code, message)
}
