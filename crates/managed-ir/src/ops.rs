//! Operators and constants used by managed instructions.

use crate::refs::StringRef;

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

/// Dispatch semantics for a managed invocation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InvokeKind {
    /// Direct static dispatch.
    Static,
    /// Virtual receiver dispatch.
    Virtual,
    /// Constructor, private, or superclass dispatch.
    Special,
    /// Interface receiver dispatch.
    Interface,
    /// Runtime-polymorphic signature dispatch.
    Polymorphic,
}
