//! Read and rewrite helpers for value definitions and uses.
//!
//! These mirror managed-ir's private walkers so the SSA transform can
//! renumber operands without mutating `managed-ir`. Phi instructions are
//! ordinary arena instructions here (they are patched in before inference);
//! the renaming pass tracks phi inputs separately while phis live in their
//! own per-block tables.

use managed_ir::insn::ManagedInsn;
use managed_ir::value::ValueId;

/// Visits every value defined by `insn`.
pub(crate) fn for_each_definition(insn: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    match insn {
        ManagedInsn::Const { result, .. }
        | ManagedInsn::Move { result, .. }
        | ManagedInsn::Phi { result, .. }
        | ManagedInsn::Neg { result, .. }
        | ManagedInsn::Not { result, .. }
        | ManagedInsn::FieldGet { result, .. }
        | ManagedInsn::ArrayGet { result, .. }
        | ManagedInsn::ArrayLength { result, .. }
        | ManagedInsn::NewInstance { result, .. }
        | ManagedInsn::NewArray { result, .. }
        | ManagedInsn::Cast { result, .. }
        | ManagedInsn::InstanceOf { result, .. }
        | ManagedInsn::Arithmetic { result, .. }
        | ManagedInsn::Compare { result, .. } => visit(*result),
        ManagedInsn::Invoke { result, .. } => {
            if let Some(value) = result {
                visit(*value);
            }
        }
        ManagedInsn::Opaque { writes, .. } => {
            for value in writes {
                visit(*value);
            }
        }
        ManagedInsn::FieldPut { .. }
        | ManagedInsn::ArrayPut { .. }
        | ManagedInsn::FillArray { .. }
        | ManagedInsn::MonitorEnter { .. }
        | ManagedInsn::MonitorExit { .. }
        | ManagedInsn::Branch { .. }
        | ManagedInsn::Switch { .. }
        | ManagedInsn::Return { .. }
        | ManagedInsn::Throw { .. } => {}
    }
}

/// Visits every value used by `insn`, including phi inputs keyed by edge.
pub(crate) fn for_each_use(insn: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    match insn {
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => {}
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => visit(*value),
        ManagedInsn::Phi { inputs, .. } => {
            for (_, value) in inputs {
                visit(*value);
            }
        }
        ManagedInsn::Arithmetic { lhs, rhs, .. } | ManagedInsn::Compare { lhs, rhs, .. } => {
            visit(*lhs);
            visit(*rhs);
        }
        ManagedInsn::FieldGet { object, .. } => visit_optional(*object, &mut visit),
        ManagedInsn::FieldPut { object, value, .. } => {
            visit_optional(*object, &mut visit);
            visit(*value);
        }
        ManagedInsn::ArrayGet { array, index, .. } => {
            visit(*array);
            visit(*index);
        }
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => {
            visit(*array);
            visit(*index);
            visit(*value);
        }
        ManagedInsn::ArrayLength { array, .. }
        | ManagedInsn::NewArray { length: array, .. }
        | ManagedInsn::MonitorEnter { object: array }
        | ManagedInsn::MonitorExit { object: array }
        | ManagedInsn::Switch { key: array, .. } => visit(*array),
        ManagedInsn::FillArray { array, values } => {
            visit(*array);
            for value in values {
                visit(*value);
            }
        }
        ManagedInsn::Invoke { args, .. } | ManagedInsn::Opaque { reads: args, .. } => {
            for value in args {
                visit(*value);
            }
        }
        ManagedInsn::Branch { condition, .. } | ManagedInsn::Return { value: condition } => {
            visit_optional(*condition, &mut visit);
        }
    }
}

/// Rewrites every value used by `insn` in place, including phi inputs.
pub(crate) fn for_each_use_mut(insn: &mut ManagedInsn, mut visit: impl FnMut(&mut ValueId)) {
    match insn {
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => {}
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => visit(value),
        ManagedInsn::Phi { inputs, .. } => {
            for (_, value) in inputs {
                visit(value);
            }
        }
        ManagedInsn::Arithmetic { lhs, rhs, .. } | ManagedInsn::Compare { lhs, rhs, .. } => {
            visit(lhs);
            visit(rhs);
        }
        ManagedInsn::FieldGet { object, .. } => visit_optional_mut(object.as_mut(), &mut visit),
        ManagedInsn::FieldPut { object, value, .. } => {
            visit_optional_mut(object.as_mut(), &mut visit);
            visit(value);
        }
        ManagedInsn::ArrayGet { array, index, .. } => {
            visit(array);
            visit(index);
        }
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => {
            visit(array);
            visit(index);
            visit(value);
        }
        ManagedInsn::ArrayLength { array, .. }
        | ManagedInsn::NewArray { length: array, .. }
        | ManagedInsn::MonitorEnter { object: array }
        | ManagedInsn::MonitorExit { object: array }
        | ManagedInsn::Switch { key: array, .. } => visit(array),
        ManagedInsn::FillArray { array, values } => {
            visit(array);
            for value in values {
                visit(value);
            }
        }
        ManagedInsn::Invoke { args, .. } | ManagedInsn::Opaque { reads: args, .. } => {
            for value in args {
                visit(value);
            }
        }
        ManagedInsn::Branch { condition, .. } | ManagedInsn::Return { value: condition } => {
            visit_optional_mut(condition.as_mut(), &mut visit);
        }
    }
}

fn visit_optional(value: Option<ValueId>, visit: &mut impl FnMut(ValueId)) {
    if let Some(value) = value {
        visit(value);
    }
}

fn visit_optional_mut(value: Option<&mut ValueId>, visit: &mut impl FnMut(&mut ValueId)) {
    if let Some(value) = value {
        visit(value);
    }
}
