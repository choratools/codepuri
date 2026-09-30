//! Errors produced while configuring or rendering emitted text.

use thiserror::Error;

/// A failure at an emission boundary.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EmitError {
    /// An operator key was registered more than once.
    #[error("operator key `{0}` is already registered")]
    DuplicateOperator(String),
    /// Operators at one precedence level disagreed about associativity.
    #[error("precedence level {precedence} has inconsistent associativity")]
    InconsistentAssociativity {
        /// The conflicting precedence level.
        precedence: u16,
    },
    /// An expression referenced an operator absent from its table.
    #[error("operator key `{0}` is not registered")]
    UnknownOperator(String),
    /// A text node contained a line break instead of a layout node.
    #[error("text nodes cannot contain newline characters")]
    EmbeddedNewline,
    /// A rendered coordinate exceeded the platform's address space.
    #[error("rendered position overflowed")]
    PositionOverflow,
}
