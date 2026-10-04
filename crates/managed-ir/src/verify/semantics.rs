use decomp_core::ids::EdgeId;

use crate::insn::ManagedInsn;
use crate::value::ValueId;

#[derive(Clone, Copy)]
pub(super) enum UseLocation {
    Instruction,
    PhiEdge(EdgeId),
}

pub(super) fn for_each_definition(insn: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    match insn {
        ManagedInsn::Const { result, value: _ }
        | ManagedInsn::Move { result, value: _ }
        | ManagedInsn::Phi { result, inputs: _ }
        | ManagedInsn::Neg { result, value: _ }
        | ManagedInsn::Not { result, value: _ }
        | ManagedInsn::FieldGet {
            result,
            object: _,
            field: _,
        }
        | ManagedInsn::ArrayGet {
            result,
            array: _,
            index: _,
        }
        | ManagedInsn::ArrayLength { result, array: _ }
        | ManagedInsn::NewInstance { result, class: _ }
        | ManagedInsn::NewArray {
            result,
            element: _,
            length: _,
        }
        | ManagedInsn::Cast {
            result,
            value: _,
            target: _,
        }
        | ManagedInsn::InstanceOf {
            result,
            value: _,
            target: _,
        }
        | ManagedInsn::Arithmetic {
            op: _,
            result,
            lhs: _,
            rhs: _,
        }
        | ManagedInsn::Compare {
            op: _,
            result,
            lhs: _,
            rhs: _,
        }
        | ManagedInsn::OrderingCompare {
            op: _,
            result,
            lhs: _,
            rhs: _,
        } => visit(*result),
        ManagedInsn::Invoke { result, .. } | ManagedInsn::NewFilledArray { result, .. } => {
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

pub(super) fn for_each_use(insn: &ManagedInsn, mut visit: impl FnMut(ValueId, UseLocation)) {
    let mut ordinary = |value| visit(value, UseLocation::Instruction);
    match insn {
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => {}
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => ordinary(*value),
        ManagedInsn::Phi { inputs, .. } => {
            for (edge, value) in inputs {
                visit(*value, UseLocation::PhiEdge(*edge));
            }
        }
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => {
            ordinary(*lhs);
            ordinary(*rhs);
        }
        ManagedInsn::FieldGet { object, .. } => visit_optional(*object, &mut ordinary),
        ManagedInsn::FieldPut { object, value, .. } => {
            visit_optional(*object, &mut ordinary);
            ordinary(*value);
        }
        ManagedInsn::ArrayGet { array, index, .. } => {
            ordinary(*array);
            ordinary(*index);
        }
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => {
            ordinary(*array);
            ordinary(*index);
            ordinary(*value);
        }
        ManagedInsn::ArrayLength { array, .. }
        | ManagedInsn::NewArray { length: array, .. }
        | ManagedInsn::MonitorEnter { object: array }
        | ManagedInsn::MonitorExit { object: array }
        | ManagedInsn::Switch { key: array, .. } => ordinary(*array),
        ManagedInsn::FillArray { array, values } => {
            ordinary(*array);
            for value in values {
                ordinary(*value);
            }
        }
        ManagedInsn::NewFilledArray { elements: args, .. }
        | ManagedInsn::Invoke { args, .. }
        | ManagedInsn::Opaque { reads: args, .. } => {
            for value in args {
                ordinary(*value);
            }
        }
        ManagedInsn::Branch { condition, .. } | ManagedInsn::Return { value: condition } => {
            visit_optional(*condition, &mut ordinary);
        }
    }
}

fn visit_optional(value: Option<ValueId>, visit: &mut impl FnMut(ValueId)) {
    if let Some(value) = value {
        visit(value);
    }
}

pub(super) fn phi_inputs(insn: &ManagedInsn) -> Option<&[(EdgeId, ValueId)]> {
    match insn {
        ManagedInsn::Phi { inputs, .. } => Some(inputs),
        ManagedInsn::Const { .. }
        | ManagedInsn::Move { .. }
        | ManagedInsn::Arithmetic { .. }
        | ManagedInsn::Compare { .. }
        | ManagedInsn::OrderingCompare { .. }
        | ManagedInsn::Neg { .. }
        | ManagedInsn::Not { .. }
        | ManagedInsn::FieldGet { .. }
        | ManagedInsn::FieldPut { .. }
        | ManagedInsn::ArrayGet { .. }
        | ManagedInsn::ArrayPut { .. }
        | ManagedInsn::ArrayLength { .. }
        | ManagedInsn::NewInstance { .. }
        | ManagedInsn::NewArray { .. }
        | ManagedInsn::NewFilledArray { .. }
        | ManagedInsn::FillArray { .. }
        | ManagedInsn::Invoke { .. }
        | ManagedInsn::Cast { .. }
        | ManagedInsn::InstanceOf { .. }
        | ManagedInsn::MonitorEnter { .. }
        | ManagedInsn::MonitorExit { .. }
        | ManagedInsn::Branch { .. }
        | ManagedInsn::Switch { .. }
        | ManagedInsn::Return { .. }
        | ManagedInsn::Throw { .. }
        | ManagedInsn::Opaque { .. } => None,
    }
}

pub(super) fn for_each_equal_width(insn: &ManagedInsn, mut visit: impl FnMut(ValueId, ValueId)) {
    match insn {
        ManagedInsn::Move { result, value }
        | ManagedInsn::Neg { result, value }
        | ManagedInsn::Not { result, value } => visit(*result, *value),
        ManagedInsn::Phi { result, inputs } => {
            for (_, value) in inputs {
                visit(*result, *value);
            }
        }
        ManagedInsn::Arithmetic {
            op,
            result,
            lhs,
            rhs,
        } => {
            visit(*result, *lhs);
            if !matches!(
                op,
                crate::ops::ArithOp::ShiftLeft
                    | crate::ops::ArithOp::ShiftRight
                    | crate::ops::ArithOp::UnsignedShiftRight
            ) {
                visit(*result, *rhs);
            }
        }
        ManagedInsn::Compare { lhs, rhs, .. } | ManagedInsn::OrderingCompare { lhs, rhs, .. } => {
            visit(*lhs, *rhs);
        }
        ManagedInsn::Const { .. }
        | ManagedInsn::FieldGet { .. }
        | ManagedInsn::FieldPut { .. }
        | ManagedInsn::ArrayGet { .. }
        | ManagedInsn::ArrayPut { .. }
        | ManagedInsn::ArrayLength { .. }
        | ManagedInsn::NewInstance { .. }
        | ManagedInsn::NewArray { .. }
        | ManagedInsn::NewFilledArray { .. }
        | ManagedInsn::FillArray { .. }
        | ManagedInsn::Invoke { .. }
        | ManagedInsn::Cast { .. }
        | ManagedInsn::InstanceOf { .. }
        | ManagedInsn::MonitorEnter { .. }
        | ManagedInsn::MonitorExit { .. }
        | ManagedInsn::Branch { .. }
        | ManagedInsn::Switch { .. }
        | ManagedInsn::Return { .. }
        | ManagedInsn::Throw { .. }
        | ManagedInsn::Opaque { .. } => {}
    }
}

pub(super) fn for_each_category_one(insn: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    match insn {
        ManagedInsn::NewFilledArray {
            result, elements, ..
        } => {
            visit_optional(*result, &mut visit);
            for value in elements {
                visit(*value);
            }
        }
        ManagedInsn::Arithmetic {
            op:
                crate::ops::ArithOp::ShiftLeft
                | crate::ops::ArithOp::ShiftRight
                | crate::ops::ArithOp::UnsignedShiftRight,
            rhs,
            ..
        } => visit(*rhs),
        ManagedInsn::Compare { result, .. }
        | ManagedInsn::OrderingCompare { result, .. }
        | ManagedInsn::NewInstance { result, .. } => visit(*result),
        ManagedInsn::FieldGet { object, .. } | ManagedInsn::FieldPut { object, .. } => {
            visit_optional(*object, &mut visit);
        }
        ManagedInsn::ArrayGet { array, index, .. } | ManagedInsn::ArrayPut { array, index, .. } => {
            visit(*array);
            visit(*index);
        }
        ManagedInsn::ArrayLength { result, array } => {
            visit(*result);
            visit(*array);
        }
        ManagedInsn::FillArray { array, .. }
        | ManagedInsn::MonitorEnter { object: array }
        | ManagedInsn::MonitorExit { object: array }
        | ManagedInsn::Switch { key: array, .. }
        | ManagedInsn::Throw { value: array } => visit(*array),
        ManagedInsn::NewArray { result, length, .. } => {
            visit(*result);
            visit(*length);
        }
        ManagedInsn::InstanceOf { result, value, .. } => {
            visit(*result);
            visit(*value);
        }
        ManagedInsn::Branch { condition, .. } => visit_optional(*condition, &mut visit),
        ManagedInsn::Const { .. }
        | ManagedInsn::Move { .. }
        | ManagedInsn::Phi { .. }
        | ManagedInsn::Arithmetic { .. }
        | ManagedInsn::Neg { .. }
        | ManagedInsn::Not { .. }
        | ManagedInsn::Invoke { .. }
        | ManagedInsn::Cast { .. }
        | ManagedInsn::Return { .. }
        | ManagedInsn::Opaque { .. } => {}
    }
}
