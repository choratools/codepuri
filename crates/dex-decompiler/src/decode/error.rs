//! Typed failures produced while decoding Dalvik instructions.

use thiserror::Error;

/// Why a Dalvik instruction stream could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeReason {
    /// The raw instruction bytes do not match `insns_size` or end mid-instruction.
    #[error("truncated instruction stream: needed {needed} code units, {remaining} remain")]
    Truncated {
        /// Number of code units required by the current operation.
        needed: u32,
        /// Number of code units still available.
        remaining: u32,
    },
    /// An opcode is reserved or not part of portable DEX 035-041.
    #[error("unsupported opcode 0x{opcode:02x}")]
    UnsupportedOpcode {
        /// Reserved opcode byte.
        opcode: u8,
    },
    /// A decoded register is outside the method register file.
    #[error("register v{register} is outside registers_size {registers_size}")]
    RegisterOutOfRange {
        /// Invalid register index.
        register: u16,
        /// Register count declared by the code item.
        registers_size: u16,
    },
    /// A branch or switch case resolves outside the instruction array.
    #[error("branch target {target} is outside {insns_size} code units")]
    BranchTargetOutOfRange {
        /// Signed computed target, retained to describe underflow as well as overflow.
        target: i64,
        /// Instruction-array length in code units.
        insns_size: u32,
    },
    /// A `31t` instruction points at the wrong payload pseudo-instruction.
    #[error("bad payload ident: expected 0x{expected:04x}, found 0x{found:04x}")]
    BadPayloadIdent {
        /// Identifier required by the referring opcode.
        expected: u16,
        /// Identifier present at the payload target.
        found: u16,
    },
    /// A payload count, element width, or encoded extent is invalid.
    #[error("malformed payload: {message}")]
    MalformedPayload {
        /// Static explanation of the violated payload rule.
        message: &'static str,
    },
    /// A packed-register invoke encodes more than five words.
    #[error("invalid packed argument word count {count}")]
    InvalidArgumentCount {
        /// Encoded argument word count.
        count: u8,
    },
}

/// A decode failure annotated with the instruction offset and opcode byte.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("decode error at code-unit {offset} (opcode 0x{opcode:02x}): {reason}")]
pub struct DecodeError {
    /// Offset of the instruction that exposed the failure, in 16-bit code units.
    pub offset: u32,
    /// Opcode byte of the instruction that exposed the failure.
    pub opcode: u8,
    /// Typed failure reason.
    pub reason: DecodeReason,
}

impl DecodeError {
    pub(crate) const fn new(offset: u32, opcode: u8, reason: DecodeReason) -> Self {
        Self {
            offset,
            opcode,
            reason,
        }
    }
}
