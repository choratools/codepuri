//! Numeric comparison constraints must preserve the domains of producers.
#![allow(clippy::unwrap_used)]

use super::*;
use managed_ir::ops::CompareOp;
use managed_ir::refs::ClassRef;
use slotmap::SlotMap;

#[test]
fn shared_byte_bound_does_not_narrow_array_length_or_copy() {
    let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
    let array = values.insert(());
    let length = values.insert(());
    let copy = values.insert(());
    let byte = values.insert(());
    let bound = values.insert(());
    let byte_test = values.insert(());
    let length_test = values.insert(());
    let mut insns = InsnArena::with_key();
    insns.insert(ManagedInsn::ArrayLength {
        result: length,
        array,
    });
    insns.insert(ManagedInsn::Move {
        result: copy,
        value: length,
    });
    insns.insert(ManagedInsn::Const {
        result: bound,
        value: Constant::Int(1),
    });
    insns.insert(ManagedInsn::Compare {
        op: CompareOp::GreaterThan,
        result: byte_test,
        lhs: byte,
        rhs: bound,
    });
    insns.insert(ManagedInsn::Compare {
        op: CompareOp::NotEqual,
        result: length_test,
        lhs: copy,
        rhs: bound,
    });
    let seeds = HashMap::from([
        (
            array,
            ManagedType::Array {
                element: Box::new(ManagedType::Byte),
                nullable: false,
            },
        ),
        (length, ManagedType::Int),
        (copy, ManagedType::Unknown),
        (byte, ManagedType::Byte),
        (bound, ManagedType::Int),
        (byte_test, ManagedType::Unknown),
        (length_test, ManagedType::Unknown),
    ]);
    let (actual, diagnostics) = super::tests::solve(&insns, seeds, false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(actual[&length], ManagedType::Int);
    assert_eq!(actual[&copy], ManagedType::Int);
    assert_eq!(actual[&byte], ManagedType::Byte);
    assert_eq!(actual[&bound], ManagedType::Int);
    assert_eq!(actual[&byte_test], ManagedType::Boolean);
    assert_eq!(actual[&length_test], ManagedType::Boolean);
}

fn comparison(
    left: ManagedType,
    right: ManagedType,
) -> (ManagedType, ManagedType, Vec<Diagnostic>) {
    let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
    let lhs = values.insert(());
    let rhs = values.insert(());
    let result = values.insert(());
    let mut insns = InsnArena::with_key();
    insns.insert(ManagedInsn::Compare {
        op: CompareOp::Equal,
        result,
        lhs,
        rhs,
    });
    let (types, diagnostics) = super::tests::solve(
        &insns,
        HashMap::from([(lhs, left), (rhs, right), (result, ManagedType::Unknown)]),
        false,
    );
    assert_eq!(types[&result], ManagedType::Boolean);
    (types[&lhs].clone(), types[&rhs].clone(), diagnostics)
}

#[test]
fn all_integral_comparison_pairs_keep_their_producer_domains() {
    let types = [
        ManagedType::Byte,
        ManagedType::Short,
        ManagedType::Char,
        ManagedType::Int,
    ];
    for left in &types {
        for right in &types {
            let (actual_left, actual_right, diagnostics) = comparison(left.clone(), right.clone());
            assert_eq!(&actual_left, left);
            assert_eq!(&actual_right, right);
            assert!(
                diagnostics.is_empty(),
                "{left:?}/{right:?}: {diagnostics:?}"
            );
        }
    }
}

#[test]
fn incompatible_boolean_reference_comparisons_still_diagnose() {
    let reference = ManagedType::Reference(ClassRef::new(1));
    for (left, right) in [
        (ManagedType::Boolean, reference.clone()),
        (reference, ManagedType::Boolean),
    ] {
        let (actual_left, actual_right, diagnostics) = comparison(left.clone(), right.clone());
        assert_eq!(actual_left, left);
        assert_eq!(actual_right, right);
        assert!(!diagnostics.is_empty());
    }
}

#[test]
fn ordering_and_shift_opcode_domains_remain_exact() {
    let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
    let lhs = values.insert(());
    let rhs = values.insert(());
    let result = values.insert(());
    for right in [ManagedType::Long, ManagedType::Double] {
        let mut insns = InsnArena::with_key();
        insns.insert(ManagedInsn::OrderingCompare {
            op: OrderingOp::CmpLong,
            result,
            lhs,
            rhs,
        });
        let (types, diagnostics) = super::tests::solve(
            &insns,
            HashMap::from([
                (lhs, ManagedType::Long),
                (rhs, right.clone()),
                (result, ManagedType::Unknown),
            ]),
            false,
        );
        assert_eq!(types[&lhs], ManagedType::Long);
        assert_eq!(types[&rhs], right);
        assert_eq!(types[&result], ManagedType::Int);
        assert_eq!(diagnostics.is_empty(), right == ManagedType::Long);
    }
    let mut insns = InsnArena::with_key();
    insns.insert(ManagedInsn::Arithmetic {
        op: ArithOp::ShiftLeft,
        result,
        lhs,
        rhs,
    });
    let (types, diagnostics) = super::tests::solve(
        &insns,
        HashMap::from([
            (lhs, ManagedType::Long),
            (rhs, ManagedType::Int),
            (result, ManagedType::Long),
        ]),
        false,
    );
    assert!(diagnostics.is_empty());
    assert_eq!(types[&lhs], ManagedType::Long);
    assert_eq!(types[&rhs], ManagedType::Int);
    assert_eq!(types[&result], ManagedType::Long);
}
