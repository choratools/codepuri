//! DEX frontend: decodes Dalvik bytecode into decoded instructions and lifts
//! them into the managed IR (`managed-ir`), then runs CFG/SSA passes over
//! `decomp-core`.
//!
//! Stage ownership: raw DEX model comes from `dex-format`; the Dalvik
//! decoder and DEX-specific normalization (move-result fusion, payload
//! fusion, wide-register pairing) live here; everything downstream of
//! lifting is family-neutral.

/// Dalvik opcode decoding and validation.
pub mod decode;

/// Stage 2.2/2.3: lifting decoded Dalvik instructions into managed
/// register-form IR with normalization, and construction of a validated CFG.
pub mod lift;

/// Stage 2.4a: fusion of lifted one-instruction blocks into straight-line
/// chains feeding SSA construction.
pub mod merge;

/// Stage 2.4b: pruned Cytron SSA construction, renaming, and verification.
pub mod ssa;

/// Stage 2.4c: worklist type inference over the managed type lattice.
pub mod infer;

/// Explicit ordered runner for the DEX frontend stages.
pub mod passes;

/// Compact raw-lift evidence retained for original static-final proofs.
pub mod static_final_proof;

pub use decode::{
    DecodeDiagnostic, DecodeError, DecodeReason, DecodedInsn, Format, Opcode, OpcodeInfo,
    RecoveredMethod, SemanticClass, decode_method, decode_method_recovering, opcode,
};
pub use infer::{DexTypes, NoTypes, TypeContext};
pub use lift::{LiftError, LiftedMethod, lift_method, lift_recovered};
pub use merge::{MergeError, MergedMethod, merge_method};
pub use passes::{DexPasses, StageError};
pub use ssa::{SsaError, SsaMethod, SsaStats, to_ssa, to_ssa_with};
