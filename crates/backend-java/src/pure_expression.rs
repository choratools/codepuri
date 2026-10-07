//! Fail-closed local expression elision for exact single-use SSA values.
//!
//! A value is eligible only when its sole consumer is reached along an acyclic
//! chain of unique unconditional edges. The chain may cross simple constant
//! definitions only; all other intervening operations, branches, joins, Phi
//! values, and multiple operand uses are deliberately excluded.
use super::{JavaResolver, JavaType, managed_type};
use decomp_core::cfg::EdgeKind;
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    ops::{ArithOp, CompareOp, Constant, OrderingOp},
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Copy)]
struct UseSite {
    count: usize,
    instruction: InsnId,
}

struct PlanContext<'a, R> {
    ir: &'a MethodIr<SsaForm>,
    receiver: Option<ValueId>,
    declarations: &'a HashMap<ValueId, JavaType>,
    exception_scopes_empty: bool,
    resolver: &'a R,
}

/// Find definitions safe to retain as source expressions instead of locals.
/// Throwing array-length expressions are allowed only without any original
/// exception scopes and only when their direct consumer is a return. Instance
/// field reads are limited to a proven `this` receiver and non-volatile fields.
pub(super) fn plan(
    ir: &MethodIr<SsaForm>,
    receiver: Option<ValueId>,
    declarations: &HashMap<ValueId, JavaType>,
    exception_scopes_empty: bool,
    resolver: &impl JavaResolver,
) -> HashSet<ValueId> {
    if !exception_scopes_empty {
        return HashSet::new();
    }
    // Only scan full use/dominance data for bodies that contain one of the
    // initial witness-producing operations. This cheap gate keeps the planner
    // out of the overwhelmingly common methods with no array lengths or
    // Dalvik ordering comparisons.
    let mut relevant_body = false;
    for (_, instruction) in ir.instructions() {
        if matches!(
            instruction,
            ManagedInsn::Invoke { .. }
                | ManagedInsn::FieldPut { .. }
                | ManagedInsn::ArrayPut { .. }
                | ManagedInsn::FillArray { .. }
                | ManagedInsn::NewInstance { .. }
                | ManagedInsn::NewArray { .. }
                | ManagedInsn::NewFilledArray { .. }
                | ManagedInsn::MonitorEnter { .. }
                | ManagedInsn::MonitorExit { .. }
                | ManagedInsn::Throw { .. }
        ) {
            return HashSet::new();
        }
        relevant_body |= matches!(
            instruction,
            ManagedInsn::ArrayLength { .. }
                | ManagedInsn::OrderingCompare {
                    op: OrderingOp::CmpLong,
                    ..
                }
        );
    }
    if !relevant_body || has_exception_edges(ir) || !cfg_is_acyclic(ir) {
        return HashSet::new();
    }

    let mut uses = HashMap::<ValueId, UseSite>::new();
    for (id, instruction) in ir.instructions() {
        visit_reads(instruction, |value| {
            uses.entry(value)
                .and_modify(|site| site.count += 1)
                .or_insert(UseSite {
                    count: 1,
                    instruction: id,
                });
        });
    }

    let mut blocks = HashMap::new();
    for block_id in ir.cfg().block_ids() {
        let Ok(block) = ir.cfg().block(block_id) else {
            return HashSet::new();
        };
        if blocks.insert(*block.payload(), block_id).is_some() {
            return HashSet::new();
        }
    }

    let context = PlanContext {
        ir,
        receiver,
        declarations,
        exception_scopes_empty,
        resolver,
    };
    let mut planned = HashSet::new();
    for (definition, instruction) in ir.instructions() {
        let Some(result) = defined_value(instruction) else {
            continue;
        };
        if (!matches!(instruction, ManagedInsn::Const { .. }) && declarations.contains_key(&result))
            || !ir
                .value(result)
                .is_ok_and(|value| value.def() == definition)
        {
            continue;
        }
        let Some(site) = uses.get(&result).copied().filter(|site| site.count == 1) else {
            continue;
        };
        let available_at_use = if matches!(instruction, ManagedInsn::Const { .. }) {
            constant_dominates(ir, &blocks, definition, site.instruction)
        } else {
            single_straight_line_to_consumer(ir, &blocks, definition, site.instruction)
        };
        if !available_at_use {
            continue;
        }
        let Some(user) = ir.instruction(site.instruction).ok() else {
            continue;
        };
        if producer_is_safe(&context, instruction, user, result) {
            planned.insert(result);
        }
    }
    planned
}

fn has_exception_edges(ir: &MethodIr<SsaForm>) -> bool {
    ir.cfg().edge_ids().any(|edge| {
        ir.cfg()
            .edge(edge)
            .map_or(true, |edge| matches!(edge.kind(), EdgeKind::Exception(_)))
    })
}

fn cfg_is_acyclic(ir: &MethodIr<SsaForm>) -> bool {
    let mut indegree = HashMap::new();
    for block in ir.cfg().block_ids() {
        let Ok(block_ref) = ir.cfg().block(block) else {
            return false;
        };
        indegree.insert(block, block_ref.incoming().len());
    }
    let mut ready: VecDeque<_> = indegree
        .iter()
        .filter_map(|(block, degree)| (*degree == 0).then_some(*block))
        .collect();
    let mut visited = 0usize;
    while let Some(block) = ready.pop_front() {
        visited += 1;
        let Ok(block_ref) = ir.cfg().block(block) else {
            return false;
        };
        for edge_id in block_ref.outgoing() {
            let Ok(edge) = ir.cfg().edge(*edge_id) else {
                return false;
            };
            let Some(degree) = indegree.get_mut(&edge.target()) else {
                return false;
            };
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(edge.target());
            }
        }
    }
    visited == indegree.len()
}

/// A simple literal has no evaluation or exception behavior to move. Its only
/// cross-block substitution is allowed when its defining block dominates its
/// unique consumer; joins and loops are already excluded by the method gate.
fn constant_dominates(
    ir: &MethodIr<SsaForm>,
    blocks: &HashMap<InsnId, decomp_core::ids::BlockId>,
    definition: InsnId,
    consumer: InsnId,
) -> bool {
    let (Some(&definition_block), Some(&consumer_block)) =
        (blocks.get(&definition), blocks.get(&consumer))
    else {
        return false;
    };
    let mut reachable = HashSet::new();
    let mut pending = vec![ir.cfg().entry()];
    while let Some(block) = pending.pop() {
        if !reachable.insert(block) {
            continue;
        }
        let Ok(block_ref) = ir.cfg().block(block) else {
            return false;
        };
        for edge_id in block_ref.outgoing() {
            let Ok(edge) = ir.cfg().edge(*edge_id) else {
                return false;
            };
            pending.push(edge.target());
        }
    }
    if !reachable.contains(&definition_block) || !reachable.contains(&consumer_block) {
        return false;
    }
    let mut without_definition = HashSet::new();
    let mut pending = vec![ir.cfg().entry()];
    while let Some(block) = pending.pop() {
        if block == definition_block || !without_definition.insert(block) {
            continue;
        }
        let Ok(block_ref) = ir.cfg().block(block) else {
            return false;
        };
        for edge_id in block_ref.outgoing() {
            let Ok(edge) = ir.cfg().edge(*edge_id) else {
                return false;
            };
            pending.push(edge.target());
        }
    }
    !without_definition.contains(&consumer_block)
}

fn single_straight_line_to_consumer(
    ir: &MethodIr<SsaForm>,
    blocks: &HashMap<InsnId, decomp_core::ids::BlockId>,
    definition: InsnId,
    consumer: InsnId,
) -> bool {
    let (Some(&source), Some(&target)) = (blocks.get(&definition), blocks.get(&consumer)) else {
        return false;
    };
    let mut current = source;
    let mut previous_edge = None;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        let Ok(block) = ir.cfg().block(current) else {
            return false;
        };
        if let Some(edge) = previous_edge
            && block.incoming() != [edge]
        {
            return false;
        }
        let [edge_id] = block.outgoing() else {
            return false;
        };
        let Ok(edge) = ir.cfg().edge(*edge_id) else {
            return false;
        };
        if *edge.kind() != EdgeKind::Unconditional {
            return false;
        }
        let next = edge.target();
        if next == target {
            let Ok(target_block) = ir.cfg().block(target) else {
                return false;
            };
            return target_block.incoming() == [*edge_id];
        }
        let Ok(next_block) = ir.cfg().block(next) else {
            return false;
        };
        if !matches!(
            ir.instruction(*next_block.payload()),
            Ok(ManagedInsn::Const {
                value: Constant::Zero | Constant::Boolean(_) | Constant::Int(_) | Constant::Long(_),
                ..
            })
        ) {
            return false;
        }
        previous_edge = Some(*edge_id);
        current = next;
    }
    false
}

fn producer_is_safe<R: JavaResolver>(
    context: &PlanContext<'_, R>,
    producer: &ManagedInsn,
    consumer: &ManagedInsn,
    result: ValueId,
) -> bool {
    let ir = context.ir;
    let receiver = context.receiver;
    let exception_scopes_empty = context.exception_scopes_empty;
    let declarations = context.declarations;
    let resolver = context.resolver;
    let Some(result_type) = java_value_type(ir, result, declarations, resolver) else {
        return false;
    };
    if !consumer_is_supported(producer, consumer, result) {
        return false;
    }
    match producer {
        ManagedInsn::Const { value, .. } => {
            matches!(
                value,
                Constant::Zero | Constant::Boolean(_) | Constant::Int(_) | Constant::Long(_)
            ) && result_type != JavaType::Void
        }
        ManagedInsn::Arithmetic { op, lhs, rhs, .. } => {
            if matches!(op, ArithOp::Div | ArithOp::Rem) {
                return false;
            }
            let Some(left) = java_value_type(ir, *lhs, declarations, resolver) else {
                return false;
            };
            let Some(right) = java_value_type(ir, *rhs, declarations, resolver) else {
                return false;
            };
            match (op, &result_type) {
                (
                    ArithOp::ShiftLeft | ArithOp::ShiftRight | ArithOp::UnsignedShiftRight,
                    JavaType::Int | JavaType::Long,
                ) => left == result_type && right == JavaType::Int,
                (_, _)
                    if matches!(
                        result_type,
                        JavaType::Int | JavaType::Long | JavaType::Float | JavaType::Double
                    ) =>
                {
                    left == result_type && right == result_type
                }
                _ => false,
            }
        }
        ManagedInsn::Compare { lhs, rhs, .. } => {
            let (Some(left), Some(right)) = (
                java_value_type(ir, *lhs, declarations, resolver),
                java_value_type(ir, *rhs, declarations, resolver),
            ) else {
                return false;
            };
            result_type == JavaType::Boolean
                && left == right
                && matches!(
                    left,
                    JavaType::Boolean
                        | JavaType::Byte
                        | JavaType::Short
                        | JavaType::Char
                        | JavaType::Int
                        | JavaType::Long
                        | JavaType::Float
                        | JavaType::Double
                )
        }
        ManagedInsn::OrderingCompare { op, lhs, rhs, .. } => {
            let (Some(left), Some(right)) = (
                java_value_type(ir, *lhs, declarations, resolver),
                java_value_type(ir, *rhs, declarations, resolver),
            ) else {
                return false;
            };
            result_type == JavaType::Int
                && match op {
                    OrderingOp::CmpLong => left == JavaType::Long && right == JavaType::Long,
                    OrderingOp::CmpLowFloat | OrderingOp::CmpHighFloat => {
                        left == JavaType::Float && right == JavaType::Float
                    }
                    OrderingOp::CmpLowDouble | OrderingOp::CmpHighDouble => {
                        left == JavaType::Double && right == JavaType::Double
                    }
                }
        }
        ManagedInsn::Neg { value, .. } => java_value_type(ir, *value, declarations, resolver)
            .is_some_and(|source| {
                source == result_type
                    && matches!(
                        result_type,
                        JavaType::Int | JavaType::Long | JavaType::Float | JavaType::Double
                    )
            }),
        ManagedInsn::Not { value, .. } => java_value_type(ir, *value, declarations, resolver)
            .is_some_and(|source| {
                source == result_type
                    && matches!(
                        result_type,
                        JavaType::Int | JavaType::Long | JavaType::Boolean
                    )
            }),
        ManagedInsn::ArrayLength { array, .. } => {
            exception_scopes_empty
                && matches!(consumer, ManagedInsn::Return { value: Some(v) } if *v == result)
                && result_type == JavaType::Int
                && matches!(
                    java_value_type(ir, *array, declarations, resolver),
                    Some(JavaType::Array(_))
                )
        }
        ManagedInsn::FieldGet {
            object: Some(object),
            field,
            ..
        } => {
            if !exception_scopes_empty
                || receiver != Some(*object)
                || !matches!(
                    consumer,
                    ManagedInsn::ArrayLength { array, .. } if *array == result
                ) && !matches!(consumer, ManagedInsn::Return { value: Some(v) } if *v == result)
            {
                return false;
            }
            let Some(resolved) = resolver.field(*field) else {
                return false;
            };
            let Some(flags) = resolver.field_access(*field) else {
                return false;
            };
            !resolved.is_static && flags & 0x0040 == 0 && result_type == resolved.ty
        }
        _ => false,
    }
}

fn java_value_type(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    declarations: &HashMap<ValueId, JavaType>,
    resolver: &impl JavaResolver,
) -> Option<JavaType> {
    declarations.get(&value).cloned().or_else(|| {
        ir.value(value)
            .ok()
            .and_then(|value| managed_type(value.ty(), resolver).ok())
    })
}

fn consumer_is_supported(producer: &ManagedInsn, consumer: &ManagedInsn, result: ValueId) -> bool {
    match producer {
        ManagedInsn::Const { .. } => matches!(
            consumer,
            ManagedInsn::Compare { .. }
                | ManagedInsn::OrderingCompare { .. }
                | ManagedInsn::Return { value: Some(_) }
        ),
        ManagedInsn::Arithmetic { .. } => matches!(
            consumer,
            ManagedInsn::Arithmetic { .. } | ManagedInsn::Compare { .. }
        ),
        ManagedInsn::Compare { .. } => {
            matches!(
                consumer,
                ManagedInsn::Branch {
                    condition: Some(value),
                    ..
                } if *value == result
            ) || matches!(consumer, ManagedInsn::Return { value: Some(value) } if *value == result)
        }
        ManagedInsn::OrderingCompare { .. } => matches!(
            consumer,
            ManagedInsn::Compare { .. } | ManagedInsn::Return { value: Some(_) }
        ),
        ManagedInsn::ArrayLength { .. } => {
            matches!(consumer, ManagedInsn::Return { value: Some(value) } if *value == result)
        }
        ManagedInsn::FieldGet { .. } => {
            matches!(consumer, ManagedInsn::ArrayLength { array, .. } if *array == result)
                || matches!(consumer, ManagedInsn::Return { value: Some(value) } if *value == result)
        }
        _ => false,
    }
}

fn defined_value(instruction: &ManagedInsn) -> Option<ValueId> {
    match instruction {
        ManagedInsn::Const { result, .. }
        | ManagedInsn::Move { result, .. }
        | ManagedInsn::Arithmetic { result, .. }
        | ManagedInsn::Compare { result, .. }
        | ManagedInsn::OrderingCompare { result, .. }
        | ManagedInsn::Neg { result, .. }
        | ManagedInsn::Not { result, .. }
        | ManagedInsn::FieldGet { result, .. }
        | ManagedInsn::ArrayLength { result, .. } => Some(*result),
        _ => None,
    }
}

fn visit_reads(instruction: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    match instruction {
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
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => {
            visit(*lhs);
            visit(*rhs);
        }
        ManagedInsn::FieldGet { object, .. } => {
            if let Some(value) = object {
                visit(*value);
            }
        }
        ManagedInsn::FieldPut { object, value, .. } => {
            if let Some(object) = object {
                visit(*object);
            }
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
        | ManagedInsn::MonitorEnter { object: array }
        | ManagedInsn::MonitorExit { object: array } => visit(*array),
        ManagedInsn::NewArray { length, .. } => visit(*length),
        ManagedInsn::NewFilledArray { elements, .. }
        | ManagedInsn::Invoke { args: elements, .. }
        | ManagedInsn::Opaque {
            reads: elements, ..
        } => {
            for value in elements {
                visit(*value);
            }
        }
        ManagedInsn::FillArray { array, values } => {
            visit(*array);
            for value in values {
                visit(*value);
            }
        }
        ManagedInsn::Branch { condition, .. } | ManagedInsn::Return { value: condition } => {
            if let Some(value) = condition {
                visit(*value);
            }
        }
        ManagedInsn::Switch { key, .. } => visit(*key),
    }
}

/// Render a long-ordering value compared against an exact int zero as its
/// equivalent relational expression. Long comparisons have no NaN corner case.
pub(super) fn long_ordering_predicate(
    ir: &MethodIr<SsaForm>,
    planned: &HashSet<ValueId>,
    op: CompareOp,
    lhs: ValueId,
    rhs: ValueId,
    get: impl Fn(ValueId) -> Option<(String, JavaType)>,
) -> Option<String> {
    let (ordering, zero, reverse) = if is_zero_constant(ir, rhs) {
        (lhs, rhs, false)
    } else if is_zero_constant(ir, lhs) {
        (rhs, lhs, true)
    } else {
        return None;
    };
    let ManagedInsn::OrderingCompare {
        op: OrderingOp::CmpLong,
        result,
        lhs: left,
        rhs: right,
    } = ir
        .value(ordering)
        .ok()
        .and_then(|value| ir.instruction(value.def()).ok())?
    else {
        return None;
    };
    if *result != ordering || !planned.contains(&ordering) || !planned.contains(&zero) {
        return None;
    }
    let (_, zero_type) = get(zero)?;
    if zero_type != JavaType::Int {
        return None;
    }
    let (left, left_type) = get(*left)?;
    let (right, right_type) = get(*right)?;
    if left_type != JavaType::Long || right_type != JavaType::Long {
        return None;
    }
    let op = if reverse {
        match op {
            CompareOp::Equal => CompareOp::Equal,
            CompareOp::NotEqual => CompareOp::NotEqual,
            CompareOp::LessThan => CompareOp::GreaterThan,
            CompareOp::LessEqual => CompareOp::GreaterEqual,
            CompareOp::GreaterThan => CompareOp::LessThan,
            CompareOp::GreaterEqual => CompareOp::LessEqual,
        }
    } else {
        op
    };
    let symbol = match op {
        CompareOp::Equal => "==",
        CompareOp::NotEqual => "!=",
        CompareOp::LessThan => "<",
        CompareOp::LessEqual => "<=",
        CompareOp::GreaterThan => ">",
        CompareOp::GreaterEqual => ">=",
    };
    Some(format!("({left} {symbol} {right})"))
}

fn is_zero_constant(ir: &MethodIr<SsaForm>, value: ValueId) -> bool {
    matches!(
        ir.value(value)
            .ok()
            .and_then(|value| ir.instruction(value.def()).ok()),
        Some(ManagedInsn::Const {
            result,
            value: Constant::Zero | Constant::Int(0),
        }) if *result == value
    )
}
