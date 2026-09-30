use std::collections::HashMap;

use decomp_core::diagnostics::Diagnostic;

use super::{Layout, error_diagnostic, semantics};
use crate::method::{MethodIr, SsaForm};
use crate::value::ValueId;

pub(super) fn verify(
    method: &MethodIr<SsaForm>,
    layout: &Layout,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut counts = HashMap::<ValueId, usize>::new();
    for (insn_id, insn) in method.instructions() {
        semantics::for_each_definition(insn, |value_id| {
            *counts.entry(value_id).or_default() += 1;
            match method.values().get(value_id) {
                Some(value) if value.def() != insn_id => {
                    diagnostics.push(error_diagnostic(
                        "MIR_SSA_DEF_MISMATCH",
                        format!(
                            "value {value_id:?} records {:?} but is defined by {insn_id:?}",
                            value.def()
                        ),
                    ));
                }
                Some(_) => {}
                None => diagnostics.push(error_diagnostic(
                    "MIR_SSA_MISSING_VALUE",
                    format!("instruction {insn_id:?} defines missing value {value_id:?}"),
                )),
            }
        });
    }
    for (value_id, value) in method.values() {
        let count = counts.get(&value_id).copied().unwrap_or(0);
        if count != 1 {
            diagnostics.push(error_diagnostic(
                "MIR_SSA_DEFINITION_COUNT",
                format!("value {value_id:?} has {count} definitions; expected exactly one"),
            ));
        }
        if method.instructions().get(value.def()).is_none() {
            diagnostics.push(error_diagnostic(
                "MIR_SSA_MISSING_INSN",
                format!(
                    "value {value_id:?} records missing defining instruction {:?}",
                    value.def()
                ),
            ));
        } else if layout.block(value.def()).is_none() {
            diagnostics.push(error_diagnostic(
                "MIR_SSA_UNPLACED_DEF",
                format!("value {value_id:?} is defined by an unplaced instruction"),
            ));
        }
    }
}
