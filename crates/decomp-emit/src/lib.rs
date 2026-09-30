//! Language-agnostic code emission infrastructure.
//!
//! Mechanisms only, no language specifics: token stream, indentation-aware
//! document layout, operator-precedence expression writing with minimal
//! parentheses, deterministic identifier allocation, and provenance source
//! maps. Language profiles (operator tables, keyword sets) belong to
//! backend crates, not here.

pub mod document;
pub mod error;
pub mod naming;
pub mod precedence;
pub mod sourcemap;
pub mod token;

#[cfg(test)]
mod tests;
