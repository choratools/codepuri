//! Family-neutral managed semantic intermediate representation.
//!
//! This crate owns the register-form instruction set, the managed type
//! lattice, SSA value model, and structural verifiers. It does not know any
//! input format. Stages are separate types so mixing pre-SSA and post-SSA IR
//! is a compile error.

pub mod effects;
pub mod error;
pub mod insn;
pub mod method;
pub mod ops;
pub mod refs;
pub mod types;
pub mod value;
pub mod verify;
