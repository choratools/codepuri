//! Managed value types and their family-neutral lattice.

use crate::refs::ClassRef;

/// The storage category occupied by a managed value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ValueCategory {
    /// A category-1 value occupying one managed slot.
    One,
    /// A category-2 value occupying two managed slots atomically.
    Two,
}

/// A point in the managed type lattice.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ManagedType {
    /// Top: no useful type information is available.
    Unknown,
    /// Ambiguous null, integral zero, or false.
    Zero,
    /// Boolean value.
    Boolean,
    /// Signed byte value.
    Byte,
    /// Signed short value.
    Short,
    /// Unsigned character value.
    Char,
    /// Signed 32-bit integer value.
    Int,
    /// Signed 64-bit integer value.
    Long,
    /// IEEE-754 single-precision value.
    Float,
    /// IEEE-754 double-precision value.
    Double,
    /// A nullable instance of an unresolved class.
    Reference(ClassRef),
    /// An array whose element type is known structurally.
    Array {
        /// The managed element type.
        element: Box<ManagedType>,
        /// Whether the array value may be null.
        nullable: bool,
    },
    /// The null reference type.
    Null,
    /// The receiver before its constructor has initialized it.
    UninitializedThis,
    /// An allocation before its constructor has initialized it.
    UninitializedNew {
        /// Frontend-supplied allocation-site identity.
        origin: u32,
    },
    /// Bottom: mutually inconsistent type information.
    Conflict,
}

impl ManagedType {
    /// Returns whether this is a category-2 value.
    pub const fn is_wide(&self) -> bool {
        matches!(self, Self::Long | Self::Double)
    }

    /// Returns whether this is a category-2 value.
    pub const fn is_category_2(&self) -> bool {
        self.is_wide()
    }

    /// Returns the storage category when the type is usable.
    pub const fn category(&self) -> Option<ValueCategory> {
        match self {
            Self::Unknown | Self::Conflict => None,
            Self::Long | Self::Double => Some(ValueCategory::Two),
            Self::Zero
            | Self::Boolean
            | Self::Byte
            | Self::Short
            | Self::Char
            | Self::Int
            | Self::Float
            | Self::Reference(_)
            | Self::Array { .. }
            | Self::Null
            | Self::UninitializedThis
            | Self::UninitializedNew { .. } => Some(ValueCategory::One),
        }
    }
}

/// Returns the greatest lower bound in the managed type lattice.
///
/// `Unknown` is top, `Conflict` is bottom, `Zero` is the shared lower
/// bound of zero-compatible scalar and nullable reference types, and `Null`
/// is the shared lower bound of nullable reference types.
pub fn meet(left: &ManagedType, right: &ManagedType) -> ManagedType {
    if is_subtype(left, right) {
        return left.clone();
    }
    if is_subtype(right, left) {
        return right.clone();
    }
    if is_nullable_reference(left) && is_nullable_reference(right) {
        return ManagedType::Null;
    }
    if is_zero_compatible(left) && is_zero_compatible(right) {
        return ManagedType::Zero;
    }
    ManagedType::Conflict
}

/// Returns the least upper bound in the managed type lattice.
pub fn join(left: &ManagedType, right: &ManagedType) -> ManagedType {
    if is_subtype(left, right) {
        return right.clone();
    }
    if is_subtype(right, left) {
        return left.clone();
    }
    if is_integral(left) && is_integral(right) {
        return ManagedType::Int;
    }
    if is_nullish(left)
        && let Some(array) = nullable_array(right)
    {
        return array;
    }
    if is_nullish(right)
        && let Some(array) = nullable_array(left)
    {
        return array;
    }
    ManagedType::Unknown
}

fn is_subtype(subtype: &ManagedType, supertype: &ManagedType) -> bool {
    if subtype == supertype || is_conflict(subtype) || is_unknown(supertype) {
        return true;
    }
    if is_zero(subtype) && is_zero_compatible(supertype) {
        return true;
    }
    if is_null(subtype) && is_nullable_reference(supertype) {
        return true;
    }
    if integral_subtype(subtype, supertype) {
        return true;
    }
    if let (
        ManagedType::Array {
            element: sub_element,
            nullable: sub_nullable,
        },
        ManagedType::Array {
            element: super_element,
            nullable: super_nullable,
        },
    ) = (subtype, supertype)
    {
        // Unknown elements describe the array shape, not a distinct concrete
        // component type. Primitive array components remain invariant.
        return (!sub_nullable || *super_nullable)
            && (sub_element == super_element || is_unknown(super_element));
    }
    false
}

fn integral_subtype(subtype: &ManagedType, supertype: &ManagedType) -> bool {
    let subtype = classify(subtype);
    let supertype = classify(supertype);
    (subtype == TypeClass::Byte && (supertype == TypeClass::Short || supertype == TypeClass::Int))
        || ((subtype == TypeClass::Short || subtype == TypeClass::Char)
            && supertype == TypeClass::Int)
}

fn is_integral(ty: &ManagedType) -> bool {
    let class = classify(ty);
    class == TypeClass::Byte
        || class == TypeClass::Short
        || class == TypeClass::Char
        || class == TypeClass::Int
}

fn is_zero_compatible(ty: &ManagedType) -> bool {
    let class = classify(ty);
    class == TypeClass::Zero
        || class == TypeClass::Boolean
        || class == TypeClass::Byte
        || class == TypeClass::Short
        || class == TypeClass::Char
        || class == TypeClass::Int
        || class == TypeClass::Reference
        || class == TypeClass::Array(true)
        || class == TypeClass::Null
}

fn is_nullable_reference(ty: &ManagedType) -> bool {
    let class = classify(ty);
    class == TypeClass::Reference || class == TypeClass::Array(true) || class == TypeClass::Null
}

fn is_unknown(ty: &ManagedType) -> bool {
    classify(ty) == TypeClass::Unknown
}

fn is_conflict(ty: &ManagedType) -> bool {
    classify(ty) == TypeClass::Conflict
}

fn is_zero(ty: &ManagedType) -> bool {
    classify(ty) == TypeClass::Zero
}

fn is_null(ty: &ManagedType) -> bool {
    classify(ty) == TypeClass::Null
}

fn is_nullish(ty: &ManagedType) -> bool {
    classify(ty) == TypeClass::Zero || classify(ty) == TypeClass::Null
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TypeClass {
    Unknown,
    Zero,
    Boolean,
    Byte,
    Short,
    Char,
    Int,
    Long,
    Float,
    Double,
    Reference,
    Array(bool),
    Null,
    UninitializedThis,
    UninitializedNew,
    Conflict,
}

fn classify(ty: &ManagedType) -> TypeClass {
    match ty {
        ManagedType::Unknown => TypeClass::Unknown,
        ManagedType::Zero => TypeClass::Zero,
        ManagedType::Boolean => TypeClass::Boolean,
        ManagedType::Byte => TypeClass::Byte,
        ManagedType::Short => TypeClass::Short,
        ManagedType::Char => TypeClass::Char,
        ManagedType::Int => TypeClass::Int,
        ManagedType::Long => TypeClass::Long,
        ManagedType::Float => TypeClass::Float,
        ManagedType::Double => TypeClass::Double,
        ManagedType::Reference(_class) => TypeClass::Reference,
        ManagedType::Array {
            element: _element,
            nullable,
        } => TypeClass::Array(*nullable),
        ManagedType::Null => TypeClass::Null,
        ManagedType::UninitializedThis => TypeClass::UninitializedThis,
        ManagedType::UninitializedNew { origin: _origin } => TypeClass::UninitializedNew,
        ManagedType::Conflict => TypeClass::Conflict,
    }
}

fn nullable_array(ty: &ManagedType) -> Option<ManagedType> {
    match ty {
        ManagedType::Array {
            element,
            nullable: _nullable,
        } => Some(ManagedType::Array {
            element: element.clone(),
            nullable: true,
        }),
        ManagedType::Unknown
        | ManagedType::Zero
        | ManagedType::Boolean
        | ManagedType::Byte
        | ManagedType::Short
        | ManagedType::Char
        | ManagedType::Int
        | ManagedType::Long
        | ManagedType::Float
        | ManagedType::Double
        | ManagedType::Reference(_)
        | ManagedType::Null
        | ManagedType::UninitializedThis
        | ManagedType::UninitializedNew { .. }
        | ManagedType::Conflict => None,
    }
}
