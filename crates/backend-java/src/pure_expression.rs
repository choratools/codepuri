//! Fail-closed local expression elision for exact single-use SSA values.
//!
//! A value is eligible only when its sole consumer is reached along an acyclic
//! chain of unique unconditional edges. The chain may cross inert literals and
//! exact primitive operations, but never effects, throwing operations, branches,
//! joins, Phi values, or multiple operand uses.
use super::{JavaResolver, JavaType, managed_type};
use decomp_core::cfg::EdgeKind;
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    ops::{ArithOp, CompareOp, Constant, OrderingOp},
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet};

const MAX_INLINE_EXPRESSION_NODES: usize = 128;
const MAX_INLINE_EXPRESSION_DEPTH: usize = 64;

#[derive(Clone, Copy, Default)]
struct ExpressionMetrics {
    nodes: usize,
    depth: usize,
}

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
    // Only scan full use/dominance data for bodies that contain a supported
    // pure-expression candidate. This cheap gate keeps the planner out of
    // methods that cannot benefit from local expression elision.
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
            // Effects elsewhere in a body do not prohibit a local arithmetic
            // chain. The per-producer path check below rejects crossing them.
            continue;
        }
        relevant_body |= match instruction {
            ManagedInsn::Move { .. } => true,
            ManagedInsn::Arithmetic { op, .. } => !matches!(op, ArithOp::Div | ArithOp::Rem),
            ManagedInsn::ArrayLength { .. }
            | ManagedInsn::Compare { .. }
            | ManagedInsn::Neg { .. }
            | ManagedInsn::Not { .. }
            | ManagedInsn::OrderingCompare {
                op: OrderingOp::CmpLong,
                ..
            } => true,
            _ => false,
        };
    }
    // Loop lowering has its own hoisted value environment. Keep this broad
    // guard until expression planning can prove compatibility with that plan;
    // a local linear use path alone does not establish loop-slot availability.
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
    let dominance = std::cell::OnceCell::new();
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
            let tree = dominance.get_or_init(|| decomp_core::analysis::dominators(ir.cfg()));
            blocks
                .get(&definition)
                .zip(blocks.get(&site.instruction))
                .is_some_and(|(&definition, &consumer)| tree.dominates(definition, consumer))
        } else {
            single_straight_line_to_consumer(
                &context,
                instruction,
                &blocks,
                definition,
                site.instruction,
            )
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
    bound_expression_complexity(ir, &mut planned);
    planned
}

/// Bound each emitted expression, not just the longest path through it. A
/// balanced SSA tree can otherwise be exponentially larger than its depth.
/// When a rendered expression exceeds either budget, materialize its largest
/// planned operand and repeat; this also bounds initializer lines of cut nodes.
fn bound_expression_complexity(ir: &MethodIr<SsaForm>, planned: &mut HashSet<ValueId>) {
    loop {
        let metrics = planned_expression_metrics(ir, planned);
        let mut remove = HashSet::new();
        for (_, instruction) in ir.instructions() {
            let (own_nodes, own_depth) = expression_shape(instruction);
            let mut inputs = Vec::new();
            visit_reads(instruction, |input| {
                if let Some(metric) = metrics.get(&input) {
                    inputs.push((input, *metric));
                }
            });
            let nodes = inputs.iter().fold(own_nodes, |nodes, (_, metric)| {
                nodes.saturating_add(metric.nodes)
            });
            let depth = own_depth.saturating_add(
                inputs
                    .iter()
                    .map(|(_, metric)| metric.depth)
                    .max()
                    .unwrap_or(0),
            );
            if nodes <= MAX_INLINE_EXPRESSION_NODES && depth <= MAX_INLINE_EXPRESSION_DEPTH {
                continue;
            }
            let selected = inputs.iter().max_by_key(|(_, metric)| {
                if nodes > MAX_INLINE_EXPRESSION_NODES && depth > MAX_INLINE_EXPRESSION_DEPTH {
                    (metric.nodes, metric.depth)
                } else if nodes > MAX_INLINE_EXPRESSION_NODES {
                    (metric.nodes, 0)
                } else {
                    (metric.depth, metric.nodes)
                }
            });
            if let Some((value, _)) = selected {
                remove.insert(*value);
            }
        }
        if remove.is_empty() {
            break;
        }
        planned.retain(|value| !remove.contains(value));
    }
}

fn planned_expression_metrics(
    ir: &MethodIr<SsaForm>,
    planned: &HashSet<ValueId>,
) -> HashMap<ValueId, ExpressionMetrics> {
    let mut metrics = HashMap::new();
    let mut active = HashSet::new();
    for seed in planned.iter().copied() {
        let mut stack = vec![(seed, false)];
        while let Some((value, expanded)) = stack.pop() {
            if expanded {
                if metrics.contains_key(&value) {
                    active.remove(&value);
                    continue;
                }
                let instruction = ir
                    .value(value)
                    .ok()
                    .and_then(|record| ir.instruction(record.def()).ok());
                let Some(instruction) = instruction else {
                    metrics.insert(value, oversized_expression());
                    active.remove(&value);
                    continue;
                };
                let mut inputs = Vec::new();
                visit_reads(instruction, |input| {
                    if planned.contains(&input) {
                        inputs.push(input);
                    }
                });
                let (own_nodes, own_depth) = expression_shape(instruction);
                let nodes = inputs.iter().fold(own_nodes, |nodes, input| {
                    nodes.saturating_add(
                        metrics
                            .get(input)
                            .copied()
                            .unwrap_or_else(oversized_expression)
                            .nodes,
                    )
                });
                let depth = own_depth.saturating_add(
                    inputs
                        .iter()
                        .map(|input| {
                            metrics
                                .get(input)
                                .copied()
                                .unwrap_or_else(oversized_expression)
                                .depth
                        })
                        .max()
                        .unwrap_or(0),
                );
                metrics.insert(
                    value,
                    ExpressionMetrics {
                        nodes: nodes.min(MAX_INLINE_EXPRESSION_NODES + 1),
                        depth: depth.min(MAX_INLINE_EXPRESSION_DEPTH + 1),
                    },
                );
                active.remove(&value);
            } else {
                match metrics.entry(value) {
                    std::collections::hash_map::Entry::Occupied(_) => continue,
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        if !active.insert(value) {
                            entry.insert(oversized_expression());
                            continue;
                        }
                    }
                }
                let instruction = ir
                    .value(value)
                    .ok()
                    .and_then(|record| ir.instruction(record.def()).ok());
                let mut inputs = Vec::new();
                if let Some(instruction) = instruction {
                    visit_reads(instruction, |input| {
                        if planned.contains(&input) {
                            inputs.push(input);
                        }
                    });
                }
                stack.push((value, true));
                stack.extend(inputs.into_iter().rev().map(|input| (input, false)));
            }
        }
    }
    metrics
}

fn oversized_expression() -> ExpressionMetrics {
    ExpressionMetrics {
        nodes: MAX_INLINE_EXPRESSION_NODES + 1,
        depth: MAX_INLINE_EXPRESSION_DEPTH + 1,
    }
}

fn expression_shape(instruction: &ManagedInsn) -> (usize, usize) {
    match instruction {
        ManagedInsn::Const { .. }
        | ManagedInsn::Move { .. }
        | ManagedInsn::Return { .. }
        | ManagedInsn::Branch { .. }
        | ManagedInsn::Switch { .. } => (0, 0),
        // Ordering comparisons render a nested conditional expression.
        ManagedInsn::OrderingCompare { .. } => (5, 3),
        _ => (1, 1),
    }
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
    let mut ready: std::collections::VecDeque<_> = indegree
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
#[cfg(test)]
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

/// Prove a unique unconditional definition-to-use path. Primitive producers
/// may commute only with other exact non-throwing primitive work; heap reads
/// and copies of references retain the older literal-only gap rule.
fn single_straight_line_to_consumer<R: JavaResolver>(
    context: &PlanContext<'_, R>,
    producer: &ManagedInsn,
    blocks: &HashMap<InsnId, decomp_core::ids::BlockId>,
    definition: InsnId,
    consumer: InsnId,
) -> bool {
    let ir = context.ir;
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
        let Ok(intervening) = ir.instruction(*next_block.payload()) else {
            return false;
        };
        if !reorderable_primitive_operation(context, intervening)
            || (!primitive_expression_producer(context, producer)
                && !matches!(intervening, ManagedInsn::Const { .. }))
        {
            return false;
        }
        previous_edge = Some(*edge_id);
        current = next;
    }
    false
}

fn reorderable_primitive_operation<R: JavaResolver>(
    context: &PlanContext<'_, R>,
    instruction: &ManagedInsn,
) -> bool {
    match instruction {
        ManagedInsn::Const { result, value } => {
            java_value_type(context.ir, *result, context.declarations, context.resolver)
                .is_some_and(|ty| {
                    ty != JavaType::Void
                        && constant_is_exact_primitive(value, &ty)
                        && matches!(
                            value,
                            Constant::Zero
                                | Constant::Boolean(_)
                                | Constant::Int(_)
                                | Constant::Long(_)
                        )
                })
        }
        ManagedInsn::Move { result, value } => matches!(
            (
                java_value_type(
                    context.ir,
                    *result,
                    context.declarations,
                    context.resolver,
                ),
                java_value_type(
                    context.ir,
                    *value,
                    context.declarations,
                    context.resolver,
                ),
            ),
            (Some(result), Some(source)) if result == source && is_primitive(&result)
        ),
        ManagedInsn::Arithmetic { .. }
        | ManagedInsn::Compare { .. }
        | ManagedInsn::OrderingCompare { .. }
        | ManagedInsn::Neg { .. }
        | ManagedInsn::Not { .. } => pure_primitive_operation_is_safe(context, instruction),
        _ => false,
    }
}

fn is_primitive(ty: &JavaType) -> bool {
    matches!(
        ty,
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

fn primitive_expression_producer<R: JavaResolver>(
    context: &PlanContext<'_, R>,
    producer: &ManagedInsn,
) -> bool {
    match producer {
        ManagedInsn::Arithmetic { .. }
        | ManagedInsn::Compare { .. }
        | ManagedInsn::OrderingCompare { .. }
        | ManagedInsn::Neg { .. }
        | ManagedInsn::Not { .. } => pure_primitive_operation_is_safe(context, producer),
        ManagedInsn::Move { result, value } => matches!(
            (
                java_value_type(
                    context.ir,
                    *result,
                    context.declarations,
                    context.resolver,
                ),
                java_value_type(
                    context.ir,
                    *value,
                    context.declarations,
                    context.resolver,
                ),
            ),
            (Some(result), Some(source)) if result == source && is_primitive(&result)
        ),
        _ => false,
    }
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
    if matches!(
        consumer,
        ManagedInsn::Branch {
            condition: Some(value),
            ..
        } if *value == result
    ) && result_type != JavaType::Boolean
    {
        return false;
    }
    if matches!(
        consumer,
        ManagedInsn::Arithmetic { .. }
            | ManagedInsn::Compare { .. }
            | ManagedInsn::Neg { .. }
            | ManagedInsn::Not { .. }
    ) && !pure_primitive_operation_is_safe(context, consumer)
    {
        return false;
    }
    match producer {
        ManagedInsn::Move { value, .. } => java_value_type(ir, *value, declarations, resolver)
            .is_some_and(|source| source == result_type && result_type != JavaType::Void),
        ManagedInsn::Const { value, .. } => {
            matches!(
                value,
                Constant::Zero | Constant::Boolean(_) | Constant::Int(_) | Constant::Long(_)
            ) && constant_is_exact_primitive(value, &result_type)
                && result_type != JavaType::Void
                && (!matches!(
                    consumer,
                    ManagedInsn::Arithmetic { .. }
                        | ManagedInsn::Neg { .. }
                        | ManagedInsn::Not { .. }
                ) || constant_is_safe_expression_operand(
                    ir,
                    declarations,
                    resolver,
                    consumer,
                    result,
                    value,
                    &result_type,
                ))
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
            arithmetic_types_are_safe(op, &result_type, &left, &right)
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

fn pure_primitive_operation_is_safe<R: JavaResolver>(
    context: &PlanContext<'_, R>,
    instruction: &ManagedInsn,
) -> bool {
    let ir = context.ir;
    let declarations = context.declarations;
    let resolver = context.resolver;
    match instruction {
        ManagedInsn::Arithmetic {
            op,
            result,
            lhs,
            rhs,
        } => {
            let (Some(result), Some(left), Some(right)) = (
                java_value_type(ir, *result, declarations, resolver),
                java_value_type(ir, *lhs, declarations, resolver),
                java_value_type(ir, *rhs, declarations, resolver),
            ) else {
                return false;
            };
            arithmetic_types_are_safe(op, &result, &left, &right)
        }
        ManagedInsn::Compare {
            result, lhs, rhs, ..
        } => {
            let (Some(result), Some(left), Some(right)) = (
                java_value_type(ir, *result, declarations, resolver),
                java_value_type(ir, *lhs, declarations, resolver),
                java_value_type(ir, *rhs, declarations, resolver),
            ) else {
                return false;
            };
            result == JavaType::Boolean
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
        ManagedInsn::OrderingCompare {
            op,
            result,
            lhs,
            rhs,
        } => {
            let (Some(result), Some(left), Some(right)) = (
                java_value_type(ir, *result, declarations, resolver),
                java_value_type(ir, *lhs, declarations, resolver),
                java_value_type(ir, *rhs, declarations, resolver),
            ) else {
                return false;
            };
            result == JavaType::Int
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
        ManagedInsn::Neg { result, value } => {
            let (Some(result), Some(source)) = (
                java_value_type(ir, *result, declarations, resolver),
                java_value_type(ir, *value, declarations, resolver),
            ) else {
                return false;
            };
            result == source
                && matches!(
                    result,
                    JavaType::Int | JavaType::Long | JavaType::Float | JavaType::Double
                )
        }
        ManagedInsn::Not { result, value } => {
            let (Some(result), Some(source)) = (
                java_value_type(ir, *result, declarations, resolver),
                java_value_type(ir, *value, declarations, resolver),
            ) else {
                return false;
            };
            result == source && matches!(result, JavaType::Int | JavaType::Long | JavaType::Boolean)
        }
        _ => false,
    }
}

fn constant_is_exact_primitive(value: &Constant, ty: &JavaType) -> bool {
    match value {
        Constant::Zero => matches!(
            ty,
            JavaType::Boolean
                | JavaType::Byte
                | JavaType::Short
                | JavaType::Char
                | JavaType::Int
                | JavaType::Long
                | JavaType::Float
                | JavaType::Double
        ),
        Constant::Boolean(_) => *ty == JavaType::Boolean,
        Constant::Int(value) => {
            *ty == JavaType::Int
                || match ty {
                    JavaType::Byte => i8::try_from(*value).is_ok(),
                    JavaType::Short => i16::try_from(*value).is_ok(),
                    JavaType::Char => u16::try_from(*value).is_ok(),
                    _ => false,
                }
        }
        Constant::Long(_) => *ty == JavaType::Long,
        _ => false,
    }
}

fn constant_is_safe_expression_operand<R: JavaResolver>(
    ir: &MethodIr<SsaForm>,
    declarations: &HashMap<ValueId, JavaType>,
    resolver: &R,
    consumer: &ManagedInsn,
    value: ValueId,
    constant: &Constant,
    ty: &JavaType,
) -> bool {
    if !constant_is_exact_primitive(constant, ty) {
        return false;
    }
    match consumer {
        ManagedInsn::Arithmetic {
            op,
            result,
            lhs,
            rhs,
        } => {
            let (used_as_left, used_as_right) = (*lhs == value, *rhs == value);
            if used_as_left == used_as_right || matches!(op, ArithOp::Div | ArithOp::Rem) {
                return false;
            }
            let Some(operation_type) = java_value_type(ir, *result, declarations, resolver) else {
                return false;
            };
            let (Some(left), Some(right)) = (
                java_value_type(ir, *lhs, declarations, resolver),
                java_value_type(ir, *rhs, declarations, resolver),
            ) else {
                return false;
            };
            if !arithmetic_types_are_safe(op, &operation_type, &left, &right) {
                return false;
            }
            let expected = if matches!(
                op,
                ArithOp::ShiftLeft | ArithOp::ShiftRight | ArithOp::UnsignedShiftRight
            ) && used_as_right
            {
                JavaType::Int
            } else {
                operation_type
            };
            *ty == expected
        }
        ManagedInsn::Neg {
            result,
            value: operand,
        } if *operand == value => {
            java_value_type(ir, *result, declarations, resolver).as_ref() == Some(ty)
                && matches!(
                    ty,
                    JavaType::Int | JavaType::Long | JavaType::Float | JavaType::Double
                )
        }
        ManagedInsn::Not {
            result,
            value: operand,
        } if *operand == value => {
            java_value_type(ir, *result, declarations, resolver).as_ref() == Some(ty)
                && matches!(ty, JavaType::Int | JavaType::Long | JavaType::Boolean)
        }
        _ => false,
    }
}

fn arithmetic_types_are_safe(
    op: &ArithOp,
    result: &JavaType,
    left: &JavaType,
    right: &JavaType,
) -> bool {
    if matches!(op, ArithOp::Div | ArithOp::Rem) {
        return false;
    }
    match (op, result) {
        (
            ArithOp::ShiftLeft | ArithOp::ShiftRight | ArithOp::UnsignedShiftRight,
            JavaType::Int | JavaType::Long,
        ) => left == result && *right == JavaType::Int,
        (ArithOp::And | ArithOp::Or | ArithOp::Xor, JavaType::Boolean) => {
            *left == JavaType::Boolean && *right == JavaType::Boolean
        }
        (_, JavaType::Int | JavaType::Long | JavaType::Float | JavaType::Double) => {
            left == result && right == result
        }
        _ => false,
    }
}

fn consumer_is_supported(producer: &ManagedInsn, consumer: &ManagedInsn, result: ValueId) -> bool {
    let branch_consumer = matches!(
        consumer,
        ManagedInsn::Branch {
            condition: Some(value),
            ..
        } if *value == result
    );
    match producer {
        ManagedInsn::Const { .. } => {
            branch_consumer
                || matches!(
                    consumer,
                    ManagedInsn::Compare { .. }
                        | ManagedInsn::OrderingCompare { .. }
                        | ManagedInsn::Arithmetic { .. }
                        | ManagedInsn::Neg { .. }
                        | ManagedInsn::Not { .. }
                        | ManagedInsn::Return { value: Some(_) }
                        | ManagedInsn::Move { .. }
                )
        }
        ManagedInsn::Arithmetic { .. } => {
            branch_consumer
                || matches!(
                    consumer,
                    ManagedInsn::Arithmetic { .. }
                        | ManagedInsn::Compare { .. }
                        | ManagedInsn::Neg { .. }
                        | ManagedInsn::Not { .. }
                        | ManagedInsn::Return { value: Some(_) }
                        | ManagedInsn::Move { .. }
                )
        }
        ManagedInsn::Compare { .. } => {
            matches!(
                consumer,
                ManagedInsn::Branch {
                    condition: Some(value),
                    ..
                } if *value == result
            ) || matches!(consumer, ManagedInsn::Return { value: Some(value) } if *value == result)
                || matches!(consumer, ManagedInsn::Compare { .. })
                || matches!(consumer, ManagedInsn::Not { value, .. } if *value == result)
                || matches!(consumer, ManagedInsn::Move { value, .. } if *value == result)
        }
        ManagedInsn::OrderingCompare { .. } => {
            matches!(
                consumer,
                ManagedInsn::Compare { .. } | ManagedInsn::Return { value: Some(_) }
            ) || matches!(consumer, ManagedInsn::Move { value, .. } if *value == result)
        }
        ManagedInsn::ArrayLength { .. } => {
            matches!(consumer, ManagedInsn::Return { value: Some(value) } if *value == result)
        }
        ManagedInsn::Neg { .. } => {
            matches!(
                consumer,
                ManagedInsn::Arithmetic { .. }
                    | ManagedInsn::Compare { .. }
                    | ManagedInsn::Neg { .. }
                    | ManagedInsn::Not { .. }
                    | ManagedInsn::Return { value: Some(_) }
            ) || matches!(consumer, ManagedInsn::Move { value, .. } if *value == result)
        }
        ManagedInsn::Not { .. } => {
            branch_consumer
                || matches!(
                    consumer,
                    ManagedInsn::Arithmetic { .. }
                        | ManagedInsn::Compare { .. }
                        | ManagedInsn::Neg { .. }
                        | ManagedInsn::Not { .. }
                        | ManagedInsn::Return { value: Some(_) }
                )
                || matches!(consumer, ManagedInsn::Move { value, .. } if *value == result)
        }
        ManagedInsn::FieldGet { .. } => {
            matches!(consumer, ManagedInsn::ArrayLength { array, .. } if *array == result)
                || matches!(consumer, ManagedInsn::Return { value: Some(value) } if *value == result)
        }
        ManagedInsn::Move { .. } => {
            branch_consumer
                || matches!(
                    consumer,
                    ManagedInsn::Move { .. }
                        | ManagedInsn::Arithmetic { .. }
                        | ManagedInsn::Compare { .. }
                        | ManagedInsn::Neg { .. }
                        | ManagedInsn::Not { .. }
                        | ManagedInsn::Return { value: Some(_) }
                )
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

#[cfg(test)]
mod reference_tests {
    use super::*;
    use decomp_core::cfg::Cfg;
    use managed_ir::method::{InsnArena, ValueArena};

    #[test]
    fn shared_dominance_matches_constant_cut_reference_in_loops_and_dead_regions() {
        let mut instructions = InsnArena::with_key();
        let ids: Vec<_> = (0..5)
            .map(|_| instructions.insert(ManagedInsn::Return { value: None }))
            .collect();
        let mut cfg = Cfg::new(ids[0]);
        let nodes = [
            cfg.entry(),
            cfg.add_block(ids[1]),
            cfg.add_block(ids[2]),
            cfg.add_block(ids[3]),
            cfg.add_block(ids[4]),
        ];
        for (a, b) in [(0, 1), (1, 2), (2, 1), (2, 3)] {
            assert!(
                cfg.add_edge(nodes[a], nodes[b], EdgeKind::Unconditional)
                    .is_ok()
            );
        }
        let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, ValueArena::with_key());
        assert!(!cfg_is_acyclic(&ir));
        let blocks = ids
            .iter()
            .zip(nodes)
            .map(|(id, block)| (*id, block))
            .collect();
        let dom = decomp_core::analysis::dominators(ir.cfg());
        for (left, &a) in ids.iter().zip(nodes.iter()) {
            for (right, &b) in ids.iter().zip(nodes.iter()) {
                assert_eq!(
                    constant_dominates(&ir, &blocks, *left, *right),
                    dom.dominates(a, b)
                );
            }
        }
    }
}
