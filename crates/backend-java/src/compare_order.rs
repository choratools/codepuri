//! Java expression contract for Dalvik three-way compare instructions.
//!
//! This module is intentionally standalone: production wiring should import it
//! from the emitter crate and call [`ordering_expr`] from the
//! `ManagedInsn::OrderingCompare` emission arm.

use managed_ir::ops::OrderingOp;

/// Build a Java expression for a Dalvik three-way compare.
///
/// The expression intentionally does not use `Float.compare` or
/// `Double.compare`: Java's compare methods distinguish `-0.0` from `+0.0`,
/// while Dalvik `cmp*` treats them as equal.
pub fn ordering_expr(op: OrderingOp, lhs: &str, rhs: &str) -> String {
    match op {
        OrderingOp::CmpLowFloat | OrderingOp::CmpLowDouble | OrderingOp::CmpLong => {
            format!("(({lhs}) > ({rhs}) ? 1 : (({lhs}) == ({rhs}) ? 0 : -1))")
        }
        OrderingOp::CmpHighFloat | OrderingOp::CmpHighDouble => {
            format!("(({lhs}) < ({rhs}) ? -1 : (({lhs}) == ({rhs}) ? 0 : 1))")
        }
    }
}
