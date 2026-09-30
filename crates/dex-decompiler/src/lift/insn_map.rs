//! Per-opcode dispatch from decoded Dalvik instructions to managed
//! instructions.
//!
//! The dispatcher splits by opcode family: data movement, constants, field
//! and array access, and conversions lower in [`data_map`]; control
//! transfer, invocation, and arithmetic lower in [`flow_map`]. Shared state
//! and constructors live in [`lowering`], and pure opcode-byte tables in
//! [`op_maps`].

use managed_ir::value::InsnId;

use super::lowering::Lowering;
use super::values::ValueTable;

pub(super) use super::lowering::{Built, Context};

/// Lowers one decoded instruction into its managed form.
pub(crate) fn build(values: &mut ValueTable, id: InsnId, context: Context<'_>) -> Built {
    let Context {
        insn,
        fused,
        handler_input,
        fill_values,
    } = context;
    let byte = super::op_maps::info_or_opcode(insn);
    let mut state = Lowering {
        values,
        id,
        insn,
        byte,
        fused,
        handler_input,
        fill_values,
    };
    match byte {
        0x28..=0x3d | 0x6e..=0x72 | 0x74..=0x78 | 0x90..=0xe2 | 0xfa..=0xff => {
            super::flow_map::lower(&mut state)
        }
        _ => super::data_map::lower(&mut state),
    }
}
