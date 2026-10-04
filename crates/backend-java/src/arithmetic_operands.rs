//! Validate arithmetic operands without conflating int promotion with bit reinterpretation.
use crate::{JavaType, Unsupported};
use managed_ir::ops::ArithOp;

/// Validate operands for an attested arithmetic result type.
///
/// DEX int arithmetic accepts byte, short and char values after Java's ordinary
/// unary/binary int promotion. This helper does not change expressions, invent
/// narrowing casts or widen mixed long/floating operations. Boolean is accepted
/// only for homogeneous Boolean bitwise operations.
pub fn validate(
    operation: ArithOp,
    result: &JavaType,
    left: &JavaType,
    right: &JavaType,
) -> Result<(), Unsupported> {
    let int_promotable = |t: &JavaType| {
        matches!(
            t,
            JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int
        )
    };
    let shift = matches!(
        operation,
        ArithOp::ShiftLeft | ArithOp::ShiftRight | ArithOp::UnsignedShiftRight
    );
    let bitwise = matches!(operation, ArithOp::And | ArithOp::Or | ArithOp::Xor);
    let valid = match result {
        JavaType::Int => int_promotable(left) && int_promotable(right),
        JavaType::Long => {
            left == result
                && if shift {
                    int_promotable(right)
                } else {
                    right == result
                }
        }
        JavaType::Float | JavaType::Double => {
            !shift && !bitwise && left == result && right == result
        }
        JavaType::Boolean => bitwise && left == result && right == result,
        // Arithmetic evaluates as int before any explicit DEX narrowing cast.
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Unsupported::Type("arithmetic operand widths/types".into()))
    }
}
