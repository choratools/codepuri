//! Self-contained decoded Dalvik instructions.

use super::opcodes::{Format, Opcode};

/// One decoded instruction or payload pseudo-instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedInsn {
    /// Instruction start in 16-bit code units.
    pub offset: u32,
    /// Supported opcode metadata, an explicit unsupported opcode, or a payload kind.
    pub opcode: Opcode,
    /// Encoded operand format.
    pub format: Format,
    /// Registers in the same left-to-right order as the instruction operands.
    pub registers: Vec<u16>,
    /// Sign-extended or fully materialized literal, when present.
    pub literal: Option<i64>,
    /// Primary string/type/field/method/call-site pool index, when present.
    pub pool_index: Option<u32>,
    /// Secondary prototype pool index used by `45cc` and `4rcc`.
    pub secondary_pool_index: Option<u32>,
    /// Signed code-unit displacement relative to this instruction, when present.
    pub branch_offset: Option<i32>,
    /// Absolute code-unit offset of a referenced payload, when present.
    pub payload_offset: Option<u32>,
}

impl DecodedInsn {
    /// Returns the instruction start in code units.
    #[must_use]
    pub const fn offset(&self) -> u32 {
        self.offset
    }

    /// Returns the decoded opcode metadata.
    #[must_use]
    pub const fn opcode(&self) -> Opcode {
        self.opcode
    }

    /// Returns the operand format.
    #[must_use]
    pub const fn format(&self) -> Format {
        self.format
    }

    /// Returns registers in encoded operand order.
    #[must_use]
    pub fn registers(&self) -> &[u16] {
        &self.registers
    }

    /// Returns the decoded literal.
    #[must_use]
    pub const fn literal(&self) -> Option<i64> {
        self.literal
    }

    /// Returns the primary constant-pool index.
    #[must_use]
    pub const fn pool_index(&self) -> Option<u32> {
        self.pool_index
    }

    /// Returns the secondary prototype pool index.
    #[must_use]
    pub const fn secondary_pool_index(&self) -> Option<u32> {
        self.secondary_pool_index
    }

    /// Returns the relative branch displacement in code units.
    #[must_use]
    pub const fn branch_offset(&self) -> Option<i32> {
        self.branch_offset
    }

    /// Returns the absolute payload offset in code units.
    #[must_use]
    pub const fn payload_offset(&self) -> Option<u32> {
        self.payload_offset
    }
}
