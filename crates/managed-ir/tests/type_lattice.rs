//! Managed type lattice properties.

use managed_ir::refs::ClassRef;
use managed_ir::types::{ManagedType, ValueCategory, join, meet};
use proptest::prelude::*;

fn leaf_type() -> impl Strategy<Value = ManagedType> {
    prop_oneof![
        Just(ManagedType::Unknown),
        Just(ManagedType::Zero),
        Just(ManagedType::Boolean),
        Just(ManagedType::Byte),
        Just(ManagedType::Short),
        Just(ManagedType::Char),
        Just(ManagedType::Int),
        Just(ManagedType::Long),
        Just(ManagedType::Float),
        Just(ManagedType::Double),
        (0_u32..4).prop_map(|index| ManagedType::Reference(ClassRef::new(index))),
        Just(ManagedType::Null),
        Just(ManagedType::UninitializedThis),
        (0_u32..4).prop_map(|origin| ManagedType::UninitializedNew { origin }),
        Just(ManagedType::Conflict),
    ]
}

fn managed_type() -> impl Strategy<Value = ManagedType> {
    prop_oneof![
        4 => leaf_type(),
        1 => (leaf_type(), any::<bool>()).prop_map(|(element, nullable)| ManagedType::Array {
            element: Box::new(element),
            nullable,
        }),
    ]
}

proptest! {
    #[test]
    fn meet_is_commutative(left in managed_type(), right in managed_type()) {
        prop_assert_eq!(meet(&left, &right), meet(&right, &left));
    }

    #[test]
    fn meet_is_associative(
        first in managed_type(),
        second in managed_type(),
        third in managed_type(),
    ) {
        let left_grouped = meet(&meet(&first, &second), &third);
        let right_grouped = meet(&first, &meet(&second, &third));

        prop_assert_eq!(left_grouped, right_grouped);
    }

    #[test]
    fn meet_is_idempotent(value in managed_type()) {
        prop_assert_eq!(meet(&value, &value), value);
    }
}

#[test]
fn zero_and_null_refine_ambiguous_managed_values() {
    let reference = ManagedType::Reference(ClassRef::new(7));

    assert_eq!(meet(&ManagedType::Int, &reference), ManagedType::Zero);
    assert_eq!(meet(&ManagedType::Null, &reference), ManagedType::Null);
    assert_eq!(join(&ManagedType::Zero, &reference), reference);
}

#[test]
fn width_helpers_identify_category_two_values() {
    assert!(ManagedType::Long.is_wide());
    assert!(ManagedType::Double.is_category_2());
    assert_eq!(ManagedType::Long.category(), Some(ValueCategory::Two));
    assert_eq!(ManagedType::Int.category(), Some(ValueCategory::One));
    assert_eq!(ManagedType::Unknown.category(), None);
}

fn array(element: ManagedType, nullable: bool) -> ManagedType {
    ManagedType::Array {
        element: Box::new(element),
        nullable,
    }
}

#[test]
fn array_shape_and_nullability_crossing_meet_is_associative() {
    let first = array(ManagedType::Long, false);
    let second = array(ManagedType::Unknown, false);
    let third = array(ManagedType::Long, true);
    assert_eq!(meet(&second, &third), first);
    assert_eq!(meet(&meet(&first, &second), &third), first);
    assert_eq!(meet(&first, &meet(&second, &third)), first);
}

#[test]
fn array_unknown_component_and_nullable_axes_intersect_independently() {
    for concrete in [
        ManagedType::Long,
        ManagedType::Int,
        ManagedType::Double,
        ManagedType::Boolean,
        ManagedType::Reference(ClassRef::new(7)),
    ] {
        for left_nullable in [false, true] {
            for right_nullable in [false, true] {
                let shape = array(ManagedType::Unknown, left_nullable);
                let typed = array(concrete.clone(), right_nullable);
                let expected = array(concrete.clone(), left_nullable && right_nullable);
                assert_eq!(meet(&shape, &typed), expected);
                assert_eq!(meet(&typed, &shape), expected);
                assert_eq!(
                    meet(
                        &array(ManagedType::Unknown, left_nullable),
                        &array(ManagedType::Unknown, right_nullable)
                    ),
                    array(ManagedType::Unknown, left_nullable && right_nullable)
                );
            }
        }
    }
}

#[test]
fn concrete_primitive_array_components_remain_invariant() {
    for (left, right) in [
        (ManagedType::Byte, ManagedType::Short),
        (ManagedType::Short, ManagedType::Int),
        (ManagedType::Long, ManagedType::Double),
        (ManagedType::Int, ManagedType::Float),
        (ManagedType::Boolean, ManagedType::Int),
    ] {
        for left_nullable in [false, true] {
            for right_nullable in [false, true] {
                let expected = if left_nullable && right_nullable {
                    ManagedType::Null
                } else {
                    ManagedType::Conflict
                };
                assert_eq!(
                    meet(
                        &array(left.clone(), left_nullable),
                        &array(right.clone(), right_nullable)
                    ),
                    expected
                );
                assert_eq!(
                    meet(
                        &array(right.clone(), right_nullable),
                        &array(left.clone(), left_nullable)
                    ),
                    expected
                );
            }
        }
    }
}

proptest! {
    #[test]
    fn join_is_commutative(left in managed_type(), right in managed_type()) {
        prop_assert_eq!(join(&left, &right), join(&right, &left));
    }
    #[test]
    fn join_is_associative(first in managed_type(), second in managed_type(), third in managed_type()) {
        prop_assert_eq!(join(&join(&first, &second), &third), join(&first, &join(&second, &third)));
    }
    #[test]
    fn join_is_idempotent(value in managed_type()) {
        prop_assert_eq!(join(&value, &value), value);
    }
}

fn flat_array() -> impl Strategy<Value = ManagedType> {
    (
        prop_oneof![
            Just(ManagedType::Unknown),
            Just(ManagedType::Boolean),
            Just(ManagedType::Byte),
            Just(ManagedType::Short),
            Just(ManagedType::Char),
            Just(ManagedType::Int),
            Just(ManagedType::Long),
            Just(ManagedType::Float),
            Just(ManagedType::Double),
            (0_u32..4).prop_map(|index| ManagedType::Reference(ClassRef::new(index)))
        ],
        any::<bool>(),
    )
        .prop_map(|(element, nullable)| array(element, nullable))
}
proptest! {
    #[test]
    fn flat_array_join_and_meet_absorb_each_other(left in flat_array(), right in flat_array()) {
        prop_assert_eq!(meet(&left, &join(&left, &right)), left.clone());
        prop_assert_eq!(join(&left, &meet(&left, &right)), left);
    }
}
#[test]
fn array_join_records_only_common_shape_and_union_nullability() {
    for left_nullable in [false, true] {
        for right_nullable in [false, true] {
            let concrete = array(ManagedType::Long, left_nullable);
            let shape = array(ManagedType::Unknown, right_nullable);
            assert_eq!(
                join(&concrete, &shape),
                array(ManagedType::Unknown, left_nullable || right_nullable)
            );
            assert_eq!(
                join(&shape, &concrete),
                array(ManagedType::Unknown, left_nullable || right_nullable)
            );
            assert_eq!(
                join(&concrete, &array(ManagedType::Long, right_nullable)),
                array(ManagedType::Long, left_nullable || right_nullable)
            );
        }
    }
    let byte = array(ManagedType::Byte, false);
    let short = array(ManagedType::Short, false);
    let shape = array(ManagedType::Unknown, false);
    // The scalar LUB is Short; invariant array components must not inherit it.
    assert_eq!(
        join(&ManagedType::Byte, &ManagedType::Short),
        ManagedType::Short
    );
    assert_eq!(join(&byte, &short), shape);
    assert_eq!(
        join(&join(&byte, &short), &shape),
        join(&byte, &join(&short, &shape))
    );
    assert_eq!(meet(&byte, &short), ManagedType::Conflict);
}
