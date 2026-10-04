//! Lowering arms for control transfer, invocation, and arithmetic.

use managed_ir::effects::EffectFlags;
use managed_ir::insn::ManagedInsn;
use managed_ir::ops::Constant;
use managed_ir::refs::MethodRef;
use managed_ir::types::ManagedType;
use managed_ir::value::ValueId;

use super::lowering::{Built, Lowering, PendingFlow};
use super::op_maps::{
    binary_ty, binary_wide, branch_target, invoke_kind, literal_arith, ordering_op, ordering_wide,
    pair_compare, plain_arith, zero_pair_compare,
};

/// Lowers one control-transfer, invocation, or arithmetic instruction.
pub(super) fn lower(state: &mut Lowering<'_>) -> Built {
    let byte = state.byte;
    match byte {
        0x2d..=0x31 => {
            let wide = ordering_wide(byte);
            let lhs = state.values.read_typed(state.reg(1), wide);
            let rhs = state.values.read_typed(state.reg(2), wide);
            Built::Managed(ManagedInsn::OrderingCompare {
                op: ordering_op(byte),
                result: state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Int),
                lhs,
                rhs,
            })
        }
        0x28..=0x2a => Built::Pending(PendingFlow::Goto(branch_target(state.insn))),
        0x2b | 0x2c => {
            let key = state.values.read(state.reg(0));
            Built::Pending(PendingFlow::Switch {
                key,
                payload: state.insn.payload_offset,
            })
        }
        0x32..=0x37 => {
            let lhs = state.values.read_typed(state.reg(0), false);
            let rhs = state.values.read_typed(state.reg(1), false);
            let condition = state.values.compare_value(pair_compare(byte), lhs, rhs);
            Built::Pending(PendingFlow::Branch {
                condition,
                target: branch_target(state.insn),
            })
        }
        0x38..=0x3d => {
            let tested = state.values.read_typed(state.reg(0), false);
            let condition = zero_condition(state, byte, tested);
            Built::Pending(PendingFlow::Branch {
                condition,
                target: branch_target(state.insn),
            })
        }
        0x6e..=0x72 | 0x74..=0x78 | 0xfa | 0xfb => {
            let args: Vec<_> = state
                .insn
                .registers
                .iter()
                .map(|&r| state.values.read_word(r))
                .collect();
            let result = state.define_fused(ManagedType::Unknown);
            Built::Managed(ManagedInsn::Invoke {
                kind: invoke_kind(byte),
                result,
                method: MethodRef::new(state.pool()),
                args,
            })
        }
        0x90..=0xaf => {
            let lhs = state.values.read_typed(state.reg(1), binary_wide(byte));
            let rhs = state.values.read_typed(
                state.reg(2),
                binary_wide(byte) && !matches!(byte, 0xa3..=0xa5),
            );
            Built::Managed(ManagedInsn::Arithmetic {
                op: plain_arith(byte),
                result: state.define_typed(state.reg(0), binary_ty(byte), binary_wide(byte)),
                lhs,
                rhs,
            })
        }
        0xb0..=0xcf => {
            let lhs = state.values.read_typed(state.reg(0), binary_wide(byte));
            let rhs = state.values.read_typed(
                state.reg(1),
                binary_wide(byte) && !matches!(byte, 0xc3..=0xc5),
            );
            Built::Managed(ManagedInsn::Arithmetic {
                op: plain_arith(byte),
                result: state.define_typed(state.reg(0), binary_ty(byte), binary_wide(byte)),
                lhs,
                rhs,
            })
        }
        0xd0..=0xe2 => {
            let source = state.values.read(state.reg(1));
            let literal = state
                .values
                .const_value(Constant::Int(state.lit() as i32), ManagedType::Int);
            let (lhs, rhs) = if matches!(byte, 0xd1 | 0xd9) {
                (literal, source)
            } else {
                (source, literal)
            };
            Built::Managed(ManagedInsn::Arithmetic {
                op: literal_arith(byte),
                result: state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Int),
                lhs,
                rhs,
            })
        }
        0xfc | 0xfd => {
            // `InvokeKind` has no custom-dispatch variant and call-site
            // references are not pool methods, so degrade conservatively.
            let reads: Vec<_> = state
                .insn
                .registers
                .iter()
                .map(|&r| state.values.read_word(r))
                .collect();
            let writes = state
                .fused
                .map(|fused| {
                    vec![
                        state
                            .values
                            .define(state.id, fused.dest, ManagedType::Unknown),
                    ]
                })
                .unwrap_or_default();
            Built::Managed(state.opaque(reads, writes, EffectFlags::MAY_THROW))
        }
        0xfe | 0xff => {
            let writes = vec![
                state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Unknown),
            ];
            Built::Managed(state.opaque(Vec::new(), writes, EffectFlags::MAY_THROW))
        }
        _ => Built::Managed(state.opaque(Vec::new(), Vec::new(), EffectFlags::MAY_THROW)),
    }
}

/// DEX zero tests accept integers or references, not just Booleans. Keep
/// truthiness as an explicit comparison so it cannot constrain the tested
/// register to Boolean during inference.
fn zero_condition(state: &mut Lowering<'_>, byte: u8, tested: ValueId) -> ValueId {
    let zero = state.values.const_value(Constant::Zero, ManagedType::Zero);
    state
        .values
        .compare_value(zero_pair_compare(byte), tested, zero)
}
