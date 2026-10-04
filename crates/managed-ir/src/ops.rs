//! Operators and constants used by managed instructions.

use crate::refs::{StringRef, TypeRef};

/// A family-neutral constant payload.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Constant {
    /// The ambiguous managed zero literal.
    Zero,
    /// A Boolean literal.
    Boolean(bool),
    /// A signed 32-bit integer literal.
    Int(i32),
    /// A signed 64-bit integer literal.
    Long(i64),
    /// IEEE-754 single-precision bits.
    FloatBits(u32),
    /// IEEE-754 double-precision bits.
    DoubleBits(u64),
    /// A null reference literal.
    Null,
    /// An unresolved string-pool literal.
    String(StringRef),
    /// Resolves a type-pool entry to its runtime class object, including arrays
    /// and primitive types. Resolution may throw; this is not the referenced
    /// type itself as a value.
    ClassLiteral(TypeRef),
}

/// A binary arithmetic or bitwise operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ArithOp {
    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Mul,
    /// Division.
    Div,
    /// Remainder.
    Rem,
    /// Bitwise conjunction.
    And,
    /// Bitwise disjunction.
    Or,
    /// Bitwise exclusive disjunction.
    Xor,
    /// Left shift.
    ShiftLeft,
    /// Signed right shift.
    ShiftRight,
    /// Unsigned right shift.
    UnsignedShiftRight,
}

/// A comparison operation producing a Boolean value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CompareOp {
    /// Equality.
    Equal,
    /// Inequality.
    NotEqual,
    /// Strictly less than.
    LessThan,
    /// Less than or equal.
    LessEqual,
    /// Strictly greater than.
    GreaterThan,
    /// Greater than or equal.
    GreaterEqual,
}

/// A Dalvik three-way comparison that defines an `int` ordering value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OrderingOp {
    /// Single-precision compare where NaN orders as `-1` (`cmpl-float`).
    CmpLowFloat,
    /// Single-precision compare where NaN orders as `1` (`cmpg-float`).
    CmpHighFloat,
    /// Double-precision compare where NaN orders as `-1` (`cmpl-double`).
    CmpLowDouble,
    /// Double-precision compare where NaN orders as `1` (`cmpg-double`).
    CmpHighDouble,
    /// Integer long compare.
    CmpLong,
}

/// Dispatch semantics for a managed invocation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InvokeKind {
    /// Direct static dispatch.
    Static,
    /// Virtual receiver dispatch.
    Virtual,
    /// Constructor or private direct dispatch.
    Special,
    /// Exact superclass dispatch, retaining invoke-super opcode provenance.
    Super,
    /// Interface receiver dispatch.
    Interface,
    /// Runtime-polymorphic signature dispatch.
    Polymorphic,
}
