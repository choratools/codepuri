//! Dalvik type-descriptor parsing into the managed lattice.

use managed_ir::refs::ClassRef;
use managed_ir::types::ManagedType;

/// Parses one type descriptor, resolving class descriptors to type-pool
/// indexes through `reverse`.
///
/// `V` yields `None` (a void descriptor names no value type), and any
/// malformed descriptor yields `None` — adversarial string pools are
/// tolerated without panicking.
pub(crate) fn parse(
    descriptor: &str,
    reverse: &dyn Fn(&str) -> Option<u32>,
) -> Option<ManagedType> {
    match descriptor.as_bytes().first()? {
        b'V' => None,
        b'Z' => Some(ManagedType::Boolean),
        b'B' => Some(ManagedType::Byte),
        b'S' => Some(ManagedType::Short),
        b'C' => Some(ManagedType::Char),
        b'I' => Some(ManagedType::Int),
        b'J' => Some(ManagedType::Long),
        b'F' => Some(ManagedType::Float),
        b'D' => Some(ManagedType::Double),
        b'L' => reverse(descriptor).map(|index| ManagedType::Reference(ClassRef::new(index))),
        b'[' => parse(&descriptor[1..], reverse).map(|element| ManagedType::Array {
            element: Box::new(element),
            nullable: true,
        }),
        _ => None,
    }
}

/// Returns the element type of an array type, if any.
pub(crate) fn element_of(ty: &ManagedType) -> Option<&ManagedType> {
    match ty {
        ManagedType::Array { element, .. } => Some(element),
        _ => None,
    }
}

/// Returns the array shape with an unknown element, for constraining
/// array operands.
pub(crate) fn array_shape() -> ManagedType {
    ManagedType::Array {
        element: Box::new(ManagedType::Unknown),
        nullable: true,
    }
}
