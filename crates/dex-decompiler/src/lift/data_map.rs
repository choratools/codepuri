//! Lowering arms for data-movement, constant, field, array, and conversion
//! instructions.

use managed_ir::effects::EffectFlags;
use managed_ir::insn::ManagedInsn;
use managed_ir::ops::Constant;
use managed_ir::refs::{ClassRef, FieldRef, StringRef, TypeRef};
use managed_ir::types::ManagedType;

use super::lowering::{Built, Lowering, SYNTHETIC_TYPE_BASE};
use super::op_maps::{array_type, conversion_ty, unary_ty};

/// Lowers one data instruction; unreachable dispatches degrade to a
/// conservative opaque.
pub(super) fn lower(state: &mut Lowering<'_>) -> Built {
    let byte = state.byte;
    match byte {
        0x01..=0x09 => {
            let wide = matches!(byte, 0x04..=0x06);
            let value = state.values.read_typed(state.reg(1), wide);
            let ty = ManagedType::Unknown;
            // The opcode fixes the destination width, independently of the
            // source definition encountered last in bytecode file order.
            let result = if wide {
                state.values.define_wide(state.id, state.reg(0), ty)
            } else {
                state.values.define(state.id, state.reg(0), ty)
            };
            Built::Managed(ManagedInsn::Move { result, value })
        }
        0x0a..=0x0c => state.orphan_result(state.reg(0)),
        0x0d => match state.handler_input {
            Some(input) => Built::Managed(ManagedInsn::Move {
                result: state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Unknown),
                value: input,
            }),
            None => state.orphan_result(state.reg(0)),
        },
        0x0e => Built::Managed(ManagedInsn::Return { value: None }),
        0x0f..=0x11 => {
            let value = state.values.read_typed(state.reg(0), byte == 0x10);
            Built::Managed(ManagedInsn::Return { value: Some(value) })
        }
        0x12..=0x15 => {
            let (constant, ty) = if state.lit() == 0 {
                (Constant::Zero, ManagedType::Zero)
            } else {
                (Constant::Int(state.lit() as i32), ManagedType::Int)
            };
            Built::Managed(ManagedInsn::Const {
                result: state.values.define(state.id, state.reg(0), ty),
                value: constant,
            })
        }
        0x16..=0x19 => Built::Managed(ManagedInsn::Const {
            result: state.define_typed(state.reg(0), ManagedType::Long, true),
            value: Constant::Long(state.lit()),
        }),
        0x1a | 0x1b => Built::Managed(ManagedInsn::Const {
            result: state
                .values
                .define(state.id, state.reg(0), ManagedType::Unknown),
            value: Constant::String(StringRef::new(state.pool())),
        }),
        0x1c => Built::Managed(ManagedInsn::Const {
            result: state
                .values
                .define(state.id, state.reg(0), ManagedType::Unknown),
            value: Constant::ClassLiteral(TypeRef::new(state.pool())),
        }),
        0x1d => Built::Managed(ManagedInsn::MonitorEnter {
            object: state.values.read(state.reg(0)),
        }),
        0x1e => Built::Managed(ManagedInsn::MonitorExit {
            object: state.values.read(state.reg(0)),
        }),
        0x1f => {
            let value = state.values.read(state.reg(0));
            // The pool operand may name an array descriptor. Inference resolves
            // its structural type; a nominal seed would meet that array as Null.
            let ty = ManagedType::Unknown;
            Built::Managed(ManagedInsn::Cast {
                result: state.values.define(state.id, state.reg(0), ty),
                value,
                target: TypeRef::new(state.pool()),
            })
        }
        0x20 => {
            let value = state.values.read(state.reg(1));
            Built::Managed(ManagedInsn::InstanceOf {
                result: state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Boolean),
                value,
                target: TypeRef::new(state.pool()),
            })
        }
        0x21 => {
            let array = state.values.read(state.reg(1));
            Built::Managed(ManagedInsn::ArrayLength {
                result: state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Int),
                array,
            })
        }
        0x22 => Built::Managed(ManagedInsn::NewInstance {
            result: state.values.define(
                state.id,
                state.reg(0),
                ManagedType::Reference(ClassRef::new(state.pool())),
            ),
            class: ClassRef::new(state.pool()),
        }),
        0x23 => {
            let length = state.values.read(state.reg(1));
            Built::Managed(ManagedInsn::NewArray {
                result: state.values.define(state.id, state.reg(0), array_type()),
                element: TypeRef::new(state.pool()),
                length,
            })
        }
        0x24 | 0x25 => {
            let reads: Vec<_> = state
                .insn
                .registers
                .iter()
                .map(|&r| state.values.read(r))
                .collect();
            let result = state.fused.map(|fused| {
                state
                    .values
                    .define(state.id, fused.dest, ManagedType::Unknown)
            });
            Built::Managed(ManagedInsn::NewFilledArray {
                result,
                target: TypeRef::new(state.pool()),
                elements: reads,
            })
        }
        0x26 => match state.fill_values.take() {
            Some(elements) => Built::Managed(ManagedInsn::FillArray {
                array: state.values.read(state.reg(0)),
                values: elements,
            }),
            None => {
                let array = state.values.read(state.reg(0));
                Built::Managed(state.opaque(
                    vec![array],
                    Vec::new(),
                    EffectFlags::WRITES_MEMORY | EffectFlags::MAY_THROW,
                ))
            }
        },
        0x27 => Built::Managed(ManagedInsn::Throw {
            value: state.values.read(state.reg(0)),
        }),
        0x2d..=0x31 => {
            // Three-way compares have no Boolean-faithful managed form.
            let lhs = state.values.read(state.reg(1));
            let rhs = state.values.read(state.reg(2));
            let result = state
                .values
                .define(state.id, state.reg(0), ManagedType::Int);
            Built::Managed(state.opaque(vec![lhs, rhs], vec![result], EffectFlags::NONE))
        }
        0x44..=0x4a => {
            let array = state.values.read(state.reg(1));
            let index = state.values.read(state.reg(2));
            // `aget-wide` loads a category-two element into the register
            // pair; its element type stays unknown until inference, so the
            // opcode's width promise is recorded as a claim.
            let result = if byte == 0x45 {
                state
                    .values
                    .define_wide(state.id, state.reg(0), ManagedType::Unknown)
            } else {
                state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Unknown)
            };
            Built::Managed(ManagedInsn::ArrayGet {
                result,
                array,
                index,
            })
        }
        0x4b..=0x51 => {
            let value = state.values.read_typed(state.reg(0), byte == 0x4c);
            let array = state.values.read(state.reg(1));
            let index = state.values.read(state.reg(2));
            Built::Managed(ManagedInsn::ArrayPut {
                array,
                index,
                value,
            })
        }
        0x52..=0x58 => {
            let object = state.values.read(state.reg(1));
            let result = if byte == 0x53 {
                // `iget-wide` fills the destination pair atomically.
                state
                    .values
                    .define_wide(state.id, state.reg(0), ManagedType::Unknown)
            } else {
                state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Unknown)
            };
            Built::Managed(ManagedInsn::FieldGet {
                result,
                object: Some(object),
                field: FieldRef::new(state.pool()),
            })
        }
        0x59..=0x5f => {
            let value = state.values.read_typed(state.reg(0), byte == 0x5a);
            let object = state.values.read(state.reg(1));
            Built::Managed(ManagedInsn::FieldPut {
                object: Some(object),
                field: FieldRef::new(state.pool()),
                value,
            })
        }
        0x60..=0x66 => {
            let result = if byte == 0x61 {
                // `sget-wide` fills the destination pair atomically.
                state
                    .values
                    .define_wide(state.id, state.reg(0), ManagedType::Unknown)
            } else {
                state
                    .values
                    .define(state.id, state.reg(0), ManagedType::Unknown)
            };
            Built::Managed(ManagedInsn::FieldGet {
                result,
                object: None,
                field: FieldRef::new(state.pool()),
            })
        }
        0x67..=0x6d => {
            let value = state.values.read_typed(state.reg(0), byte == 0x68);
            Built::Managed(ManagedInsn::FieldPut {
                object: None,
                field: FieldRef::new(state.pool()),
                value,
            })
        }
        0x7b | 0x7d | 0x7f | 0x80 => {
            let (ty, wide) = unary_ty(byte);
            let value = state.values.read_typed(state.reg(1), wide);
            Built::Managed(ManagedInsn::Neg {
                result: state.define_typed(state.reg(0), ty, wide),
                value,
            })
        }
        0x7c | 0x7e => {
            let (ty, wide) = unary_ty(byte);
            let value = state.values.read_typed(state.reg(1), wide);
            Built::Managed(ManagedInsn::Not {
                result: state.define_typed(state.reg(0), ty, wide),
                value,
            })
        }
        0x81..=0x8f => {
            let value = state
                .values
                .read_typed(state.reg(1), matches!(byte, 0x84..=0x86 | 0x8a..=0x8c));
            let (ty, wide) = conversion_ty(byte);
            Built::Managed(ManagedInsn::Cast {
                result: state.define_typed(state.reg(0), ty, wide),
                value,
                target: TypeRef::new(SYNTHETIC_TYPE_BASE | u32::from(byte)),
            })
        }
        _ => Built::Managed(state.opaque(Vec::new(), Vec::new(), EffectFlags::MAY_THROW)),
    }
}
