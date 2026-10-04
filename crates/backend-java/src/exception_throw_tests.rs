//! Throw classification agrees with synchronous managed operation semantics.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::may_throw;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::{ArithOp, Constant},
    refs::{StringRef, TypeRef},
    types::ManagedType,
    value::{Value, ValueId},
};
fn fixture(types: &[ManagedType]) -> (MethodIr<SsaForm>, Vec<ValueId>) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let entry = insns.insert(ManagedInsn::Return { value: None });
    let ids = types
        .iter()
        .map(|ty| values.insert(Value::new(entry, ty.clone())))
        .collect();
    (
        MethodIr::<SsaForm>::from_parts(Cfg::new(entry), insns, values),
        ids,
    )
}
#[test]
fn numeric_casts_and_floating_division_have_no_synchronous_handler_edge() {
    for ty in [
        ManagedType::Int,
        ManagedType::Long,
        ManagedType::Float,
        ManagedType::Double,
    ] {
        let (ir, v) = fixture(&[ty.clone(), ty.clone(), ty.clone()]);
        assert!(!may_throw(
            &ir,
            &ManagedInsn::Cast {
                result: v[0],
                value: v[1],
                target: TypeRef::new(1)
            }
        ));
        for op in [ArithOp::Div, ArithOp::Rem] {
            assert_eq!(
                may_throw(
                    &ir,
                    &ManagedInsn::Arithmetic {
                        op,
                        result: v[0],
                        lhs: v[1],
                        rhs: v[2]
                    }
                ),
                matches!(ty, ManagedType::Int | ManagedType::Long)
            );
        }
    }
    let (ir, v) = fixture(&[ManagedType::Float, ManagedType::Zero]);
    assert!(!may_throw(
        &ir,
        &ManagedInsn::Cast {
            result: v[0],
            value: v[1],
            target: TypeRef::new(1)
        }
    ));
}
#[test]
fn resolving_operations_and_declared_opaque_effects_remain_throwing() {
    let (ir, v) = fixture(&[ManagedType::Unknown, ManagedType::Unknown]);
    assert!(may_throw(
        &ir,
        &ManagedInsn::Cast {
            result: v[0],
            value: v[1],
            target: TypeRef::new(1)
        }
    ));
    assert!(may_throw(
        &ir,
        &ManagedInsn::InstanceOf {
            result: v[0],
            value: v[1],
            target: TypeRef::new(1)
        }
    ));
    for value in [
        Constant::String(StringRef::new(1)),
        Constant::ClassLiteral(TypeRef::new(1)),
    ] {
        assert!(may_throw(
            &ir,
            &ManagedInsn::Const {
                result: v[0],
                value
            }
        ));
    }
    for effects in [EffectFlags::NONE, EffectFlags::MAY_THROW] {
        assert_eq!(
            may_throw(
                &ir,
                &ManagedInsn::Opaque {
                    family_tag: 7,
                    reads: vec![],
                    writes: vec![],
                    effects
                }
            ),
            effects == EffectFlags::MAY_THROW
        );
    }
    assert!(!may_throw(
        &ir,
        &ManagedInsn::Move {
            result: v[0],
            value: v[1]
        }
    ));
}
