//! Static opcode-byte to semantic mappings shared by the lowering arms.
//!
//! These tables are pure functions of the Dalvik opcode byte and mirror the
//! opcode metadata in [`crate::decode`].

use managed_ir::insn::{ArithOp, CompareOp, InvokeKind, OrderingOp};
use managed_ir::types::ManagedType;

use crate::decode::DecodedInsn;

/// Returns the opcode byte of any decoded instruction kind.
pub(crate) fn info_or_opcode(insn: &DecodedInsn) -> u8 {
    match insn.opcode {
        crate::decode::Opcode::Supported(info) => info.opcode,
        crate::decode::Opcode::Unsupported { opcode } => opcode,
        crate::decode::Opcode::Payload { ident } => ident as u8,
    }
}

/// Returns the managed array-lattice type for object array results.
pub(crate) fn array_type() -> ManagedType {
    ManagedType::Array {
        element: Box::new(ManagedType::Unknown),
        nullable: true,
    }
}

/// Resolves an absolute branch target; [`u32::MAX`] is an unresolvable
/// sentinel that forces opaque degradation downstream.
pub(crate) fn branch_target(insn: &DecodedInsn) -> u32 {
    let relative = i64::from(insn.branch_offset.unwrap_or(0));
    let target = i64::from(insn.offset) + relative;
    u32::try_from(target).unwrap_or(u32::MAX)
}

/// Maps an invoke opcode byte to its dispatch semantics.
pub(crate) fn invoke_kind(byte: u8) -> InvokeKind {
    match byte {
        0x6e | 0x74 => InvokeKind::Virtual,
        0x6f | 0x75 => InvokeKind::Super,
        0x70 | 0x76 => InvokeKind::Special,
        0x71 | 0x77 => InvokeKind::Static,
        0x72 | 0x78 => InvokeKind::Interface,
        _ => InvokeKind::Polymorphic,
    }
}

/// Maps a unary negation or bitwise-complement opcode to its result type.
pub(crate) fn unary_ty(byte: u8) -> (ManagedType, bool) {
    match byte {
        0x7d | 0x7e => (ManagedType::Long, true),
        0x7f => (ManagedType::Float, false),
        0x80 => (ManagedType::Double, true),
        _ => (ManagedType::Int, false),
    }
}

/// Maps a numeric conversion opcode to its result type and width.
pub(crate) fn conversion_ty(byte: u8) -> (ManagedType, bool) {
    let ty = match byte {
        0x81 | 0x88 | 0x8b => ManagedType::Long,
        0x82 | 0x85 | 0x8c => ManagedType::Float,
        0x83 | 0x86 | 0x89 => ManagedType::Double,
        0x8d => ManagedType::Byte,
        0x8e => ManagedType::Char,
        _ => ManagedType::Int,
    };
    let wide = matches!(byte, 0x81 | 0x83 | 0x86 | 0x88 | 0x89 | 0x8b);
    (ty, wide)
}

/// Maps a binary arithmetic opcode to its result type.
pub(crate) fn binary_ty(byte: u8) -> ManagedType {
    match byte {
        0x9b..=0xa5 | 0xbb..=0xc5 => ManagedType::Long,
        0xa6..=0xaa | 0xc6..=0xca => ManagedType::Float,
        0xab..=0xaf | 0xcb..=0xcf => ManagedType::Double,
        _ => ManagedType::Int,
    }
}

/// Returns whether a binary arithmetic opcode defines a category-2 value.
pub(crate) fn binary_wide(byte: u8) -> bool {
    matches!(byte, 0x9b..=0xa5 | 0xab..=0xaf | 0xbb..=0xc5 | 0xcb..=0xcf)
}

/// Maps a register-register arithmetic opcode to its operator.
pub(crate) fn plain_arith(byte: u8) -> ArithOp {
    let index = match byte {
        0x90..=0x9a => byte - 0x90,
        0x9b..=0xa5 => byte - 0x9b,
        0xa6..=0xaa => byte - 0xa6,
        0xab..=0xaf => byte - 0xab,
        0xb0..=0xba => byte - 0xb0,
        0xbb..=0xc5 => byte - 0xbb,
        0xc6..=0xca => byte - 0xc6,
        _ => byte - 0xcb,
    };
    arith_from_index(index)
}

/// Maps a literal-operand arithmetic opcode to its operator.
pub(crate) fn literal_arith(byte: u8) -> ArithOp {
    arith_from_index(match byte {
        0xd0..=0xd7 => byte - 0xd0,
        _ => byte - 0xd8,
    })
}

fn arith_from_index(index: u8) -> ArithOp {
    match index {
        0 => ArithOp::Add,
        1 => ArithOp::Sub,
        2 => ArithOp::Mul,
        3 => ArithOp::Div,
        4 => ArithOp::Rem,
        5 => ArithOp::And,
        6 => ArithOp::Or,
        7 => ArithOp::Xor,
        8 => ArithOp::ShiftLeft,
        9 => ArithOp::ShiftRight,
        _ => ArithOp::UnsignedShiftRight,
    }
}

/// Maps a two-register conditional branch to its comparison.
pub(crate) fn pair_compare(byte: u8) -> CompareOp {
    match byte {
        0x32 => CompareOp::Equal,
        0x33 => CompareOp::NotEqual,
        0x34 => CompareOp::LessThan,
        0x35 => CompareOp::GreaterEqual,
        0x36 => CompareOp::GreaterThan,
        _ => CompareOp::LessEqual,
    }
}

/// Maps a compare-against-zero branch to its comparison against zero.
pub(crate) fn zero_pair_compare(byte: u8) -> CompareOp {
    match byte {
        0x38 => CompareOp::Equal,
        0x39 => CompareOp::NotEqual,
        0x3a => CompareOp::LessThan,
        0x3b => CompareOp::GreaterEqual,
        0x3c => CompareOp::GreaterThan,
        _ => CompareOp::LessEqual,
    }
}

/// Maps Dalvik three-way compare opcodes to ordering semantics.
pub(crate) fn ordering_op(byte: u8) -> OrderingOp {
    match byte {
        0x2d => OrderingOp::CmpLowFloat,
        0x2e => OrderingOp::CmpHighFloat,
        0x2f => OrderingOp::CmpLowDouble,
        0x30 => OrderingOp::CmpHighDouble,
        _ => OrderingOp::CmpLong,
    }
}

/// Returns whether a three-way compare opcode consumes category-2 operands.
pub(crate) fn ordering_wide(byte: u8) -> bool {
    matches!(byte, 0x2f..=0x31)
}

#[cfg(test)]
mod invoke_provenance_tests {
    use super::*;
    #[test]
    fn super_and_direct_opcodes_remain_distinct() {
        for opcode in [0x6f, 0x75] {
            assert_eq!(invoke_kind(opcode), InvokeKind::Super);
        }
        for opcode in [0x70, 0x76] {
            assert_eq!(invoke_kind(opcode), InvokeKind::Special);
        }
        for opcode in [0x6e, 0x74] {
            assert_eq!(invoke_kind(opcode), InvokeKind::Virtual);
        }
    }
}
