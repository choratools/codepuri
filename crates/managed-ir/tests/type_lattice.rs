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
