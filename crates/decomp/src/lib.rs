//! Decompiler facade and backend-independent source output infrastructure.
//!
//! Output is delivered through callbacks or an explicitly configured directory;
//! the library does not write to stdout.

pub mod dex_java;
mod java_access;
pub mod java_aliases;
mod java_array_clone;
#[cfg(test)]
mod java_auth_input_copy_scope_actual;
mod java_checked_bound_targets;
pub mod java_class_kinds;
pub mod java_descriptors;
mod java_exception_regions;
#[cfg(test)]
mod java_exception_scopes;
pub mod java_incoming_registers;
pub mod java_initial_parameters;
mod java_integration;
pub mod java_members;
pub mod java_session;
pub mod library_evidence;
pub mod library_fingerprint;
pub use java_members::external_metadata as java_external_metadata;
pub mod java_hierarchy;
pub use backend_java::type_compatibility;
pub mod java_method_flags;
/// Exact inherited class-super dispatch proofs.
pub mod java_super_resolution;
pub mod output;

#[cfg(test)]
mod output_tests;
