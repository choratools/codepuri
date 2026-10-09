//! Use-site constant typing and checked phi declarations, without IR mutation.
//! Parent wiring: export this module, consult `expression` before nominal `ty`,
//! and validate every phi with `phi_plan` before emitting its edge copies.
use crate::type_compatibility::{
    Coercion, Compatibility, Hierarchy, UnsupportedType, ValueType, check_local_plan, expected_type,
};
use crate::{JavaResolver, JavaType};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    ops::{Constant, OrderingOp},
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet};

#[path = "producer_reference_locals.rs"]
mod producer_reference_locals;

/// Immutable full-arena relationships shared by all declaration candidates.
/// Adjacency vectors retain arena order; consumer positions are deduplicated
/// per instruction, while operand duplicates remain in the instruction itself.
pub(crate) struct MethodIndex {
    instructions: Vec<InsnId>,
    copies: HashMap<ValueId, Vec<ValueId>>,
    downstream: HashMap<ValueId, Vec<ValueId>>,
    consumers: HashMap<ValueId, Vec<usize>>,
    definitions: HashMap<ValueId, usize>,
    phi_members: HashSet<ValueId>,
    thrown: HashSet<ValueId>,
    caught: HashMap<ValueId, JavaType>,
}

impl MethodIndex {
    pub(crate) fn new(ir: &MethodIr<SsaForm>) -> Self {
        let mut index = Self {
            instructions: ir.instructions().keys().collect(),
            copies: HashMap::new(),
            downstream: HashMap::new(),
            consumers: HashMap::new(),
            definitions: HashMap::new(),
            phi_members: HashSet::new(),
            thrown: HashSet::new(),
            caught: HashMap::new(),
        };
        for (position, (_, instruction)) in ir.instructions().iter().enumerate() {
            instruction_reads(instruction, |value| {
                let positions = index.consumers.entry(value).or_default();
                if positions.last() != Some(&position) {
                    positions.push(position);
                }
            });
            instruction_writes(instruction, |value| {
                *index.definitions.entry(value).or_default() += 1;
            });
            match instruction {
                ManagedInsn::Move { result, value } => {
                    index.copies.entry(*value).or_default().push(*result);
                    index.downstream.entry(*value).or_default().push(*result);
                }
                ManagedInsn::Phi { result, inputs } => {
                    index.phi_members.insert(*result);
                    let mut seen_inputs = HashSet::new();
                    for (_, value) in inputs {
                        index.phi_members.insert(*value);
                        // The original scan pushes one result per Phi, even
                        // when several incoming edges contain the same value.
                        if seen_inputs.insert(*value) {
                            index.downstream.entry(*value).or_default().push(*result);
                        }
                    }
                }
                ManagedInsn::Throw { value } => {
                    index.thrown.insert(*value);
                }
                _ => {}
            }
        }
        index
    }

    fn copy_closure(&self, locals: &[ValueId]) -> HashSet<ValueId> {
        let mut seen: HashSet<_> = locals.iter().copied().collect();
        let mut pending = locals.to_vec();
        while let Some(value) = pending.pop() {
            for copied in self.copies.get(&value).into_iter().flatten() {
                if seen.insert(*copied) {
                    pending.push(*copied);
                }
            }
        }
        seen
    }

    fn consumer_positions(&self, locals: &HashSet<ValueId>) -> Vec<usize> {
        let mut positions: Vec<_> = locals
            .iter()
            .flat_map(|v| self.consumers.get(v).into_iter().flatten().copied())
            .collect();
        positions.sort_unstable();
        positions.dedup();
        positions
    }
}

fn instruction_reads(instruction: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    use ManagedInsn::*;
    match instruction {
        Const { .. } | NewInstance { .. } => {}
        Move { value, .. }
        | Neg { value, .. }
        | Not { value, .. }
        | Cast { value, .. }
        | InstanceOf { value, .. }
        | Throw { value } => visit(*value),
        Phi { inputs, .. } => {
            for (_, value) in inputs {
                visit(*value);
            }
        }
        Arithmetic { lhs, rhs, .. }
        | Compare { lhs, rhs, .. }
        | OrderingCompare { lhs, rhs, .. } => {
            visit(*lhs);
            visit(*rhs);
        }
        FieldGet { object, .. } => {
            if let Some(value) = object {
                visit(*value);
            }
        }
        FieldPut { object, value, .. } => {
            if let Some(value) = object {
                visit(*value);
            }
            visit(*value);
        }
        ArrayGet { array, index, .. } => {
            visit(*array);
            visit(*index);
        }
        ArrayPut {
            array,
            index,
            value,
        } => {
            visit(*array);
            visit(*index);
            visit(*value);
        }
        ArrayLength { array, .. }
        | MonitorEnter { object: array }
        | MonitorExit { object: array } => visit(*array),
        NewArray { length, .. } => visit(*length),
        NewFilledArray { elements, .. }
        | Invoke { args: elements, .. }
        | Opaque {
            reads: elements, ..
        } => {
            for value in elements {
                visit(*value);
            }
        }
        FillArray { array, values } => {
            visit(*array);
            for value in values {
                visit(*value);
            }
        }
        Branch { condition, .. } | Return { value: condition } => {
            if let Some(value) = condition {
                visit(*value);
            }
        }
        Switch { key, .. } => visit(*key),
    }
}

fn instruction_writes(instruction: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    use ManagedInsn::*;
    match instruction {
        Const { result, .. }
        | Move { result, .. }
        | Phi { result, .. }
        | Arithmetic { result, .. }
        | Compare { result, .. }
        | OrderingCompare { result, .. }
        | Neg { result, .. }
        | Not { result, .. }
        | FieldGet { result, .. }
        | ArrayGet { result, .. }
        | ArrayLength { result, .. }
        | NewInstance { result, .. }
        | NewArray { result, .. }
        | Cast { result, .. }
        | InstanceOf { result, .. } => visit(*result),
        Invoke { result, .. } | NewFilledArray { result, .. } => {
            if let Some(value) = result {
                visit(*value);
            }
        }
        Opaque { writes, .. } => {
            for value in writes {
                visit(*value);
            }
        }
        _ => {}
    }
}

fn unknown() -> Compatibility {
    Compatibility::Indeterminate(UnsupportedType::UnknownValue)
}

/// Exact operand constraints over the entire instruction arena, including
/// instructions outside the current emission block. Unmodeled reads fail closed.
pub fn consumer_constraints(
    ir: &MethodIr<SsaForm>,
    locals: &[ValueId],
    return_type: &JavaType,
    resolver: &impl JavaResolver,
) -> Result<Vec<JavaType>, Compatibility> {
    consumer_constraints_with_declarations(
        ir,
        locals,
        return_type,
        resolver,
        &std::collections::HashMap::new(),
    )
}

/// Concrete declarations already justified by an atomic family proof.
pub(crate) fn consumer_constraints_with_declarations(
    ir: &MethodIr<SsaForm>,
    locals: &[ValueId],
    return_type: &JavaType,
    resolver: &impl JavaResolver,
    declarations: &std::collections::HashMap<ValueId, JavaType>,
) -> Result<Vec<JavaType>, Compatibility> {
    consumer_constraints_indexed(
        ir,
        &MethodIndex::new(ir),
        locals,
        return_type,
        resolver,
        declarations,
    )
}

pub(crate) fn consumer_constraints_indexed(
    ir: &MethodIr<SsaForm>,
    index: &MethodIndex,
    locals: &[ValueId],
    return_type: &JavaType,
    resolver: &impl JavaResolver,
    declarations: &HashMap<ValueId, JavaType>,
) -> Result<Vec<JavaType>, Compatibility> {
    let locals = index.copy_closure(locals);
    let positions = index.consumer_positions(&locals);
    consumer_constraints_at_positions(
        ir,
        index,
        &locals,
        &positions,
        return_type,
        resolver,
        declarations,
    )
}

fn consumer_constraints_at_positions(
    ir: &MethodIr<SsaForm>,
    index: &MethodIndex,
    locals: &HashSet<ValueId>,
    positions: &[usize],
    return_type: &JavaType,
    resolver: &impl JavaResolver,
    declarations: &HashMap<ValueId, JavaType>,
) -> Result<Vec<JavaType>, Compatibility> {
    use managed_ir::ops::InvokeKind;
    let has = |v: &ValueId| locals.contains(v);
    let ty = |v: ValueId| {
        if let Some(ty) = declarations.get(&v) {
            return Ok(ty.clone());
        }
        nominal(ir.value(v).map_err(|_| unknown())?.ty(), resolver).ok_or_else(unknown)
    };
    let mut constraints = Vec::new();
    for position in positions {
        let insn = ir
            .instruction(index.instructions[*position])
            .map_err(|_| unknown())?;
        let mut require = |v: ValueId, expected: JavaType| {
            if has(&v) {
                constraints.push(expected);
            }
        };
        match insn {
            ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => {}
            ManagedInsn::Move { result, value } => {
                if has(value) && !has(result) {
                    // A preserved producer may copy into the independently
                    // checked selected family. Its destination must be in
                    // that exact declaration plan; an arbitrary downstream
                    // copy still cannot hide unexamined consumers.
                    let expected = declarations.get(result).ok_or_else(unknown)?;
                    require(*value, expected.clone());
                }
            }
            ManagedInsn::Phi { result, inputs } => {
                if !has(result) {
                    for (_, v) in inputs {
                        if has(v) {
                            require(*v, ty(*result)?);
                        }
                    }
                }
            }
            ManagedInsn::Return { value: Some(v) } => require(*v, return_type.clone()),
            ManagedInsn::Return { value: None } => {}
            ManagedInsn::Invoke {
                kind, method, args, ..
            } if args.iter().any(has) => {
                let target = resolver.method(*method).ok_or_else(unknown)?;
                let offset = usize::from(*kind != InvokeKind::Static);
                if target.is_static != (*kind == InvokeKind::Static) {
                    return Err(Compatibility::Incompatible);
                }
                let mut operands = Vec::new();
                if args.len() == target.parameters.len() + offset {
                    operands.extend(args.iter().copied());
                } else {
                    let mut word = offset;
                    operands.extend(args.iter().take(offset).copied());
                    for t in &target.parameters {
                        let v = *args.get(word).ok_or_else(unknown)?;
                        operands.push(v);
                        if matches!(t, JavaType::Long | JavaType::Double) {
                            if args.get(word + 1) != Some(&v) {
                                return Err(Compatibility::Incompatible);
                            }
                            word += 2;
                        } else {
                            word += 1;
                        }
                    }
                    if word != args.len() {
                        return Err(Compatibility::Incompatible);
                    }
                }
                let args = operands;
                if args.len() != target.parameters.len() + offset {
                    return Err(Compatibility::Incompatible);
                }
                for (i, v) in args.iter().enumerate() {
                    require(
                        *v,
                        if i < offset {
                            JavaType::Class(target.owner.clone())
                        } else {
                            target.parameters[i - offset].clone()
                        },
                    );
                }
            }
            ManagedInsn::Invoke { .. } => {}
            ManagedInsn::FieldGet { object, field, .. }
            | ManagedInsn::FieldPut { object, field, .. }
                if object.as_ref().is_some_and(has)
                    || matches!(insn, ManagedInsn::FieldPut { value, .. } if has(value)) =>
            {
                let field = resolver.field(*field).ok_or_else(unknown)?;
                if let Some(v) = object {
                    require(*v, JavaType::Class(field.owner));
                }
                if let ManagedInsn::FieldPut { value, .. } = insn {
                    require(*value, field.ty);
                }
            }
            ManagedInsn::FieldGet { .. } | ManagedInsn::FieldPut { .. } => {}
            ManagedInsn::ArrayGet { array, index, .. }
            | ManagedInsn::ArrayPut { array, index, .. } => {
                require(*index, JavaType::Int);
                if has(array) || matches!(insn, ManagedInsn::ArrayPut { value, .. } if has(value)) {
                    let array_ty = ty(*array)?;
                    let JavaType::Array(component) = &array_ty else {
                        return Err(Compatibility::Incompatible);
                    };
                    if let ManagedInsn::ArrayPut { value, .. } = insn {
                        require(*value, *component.clone());
                    }
                    require(*array, array_ty);
                }
            }
            ManagedInsn::NewFilledArray {
                target, elements, ..
            } if elements.iter().any(has) => {
                let JavaType::Array(component) =
                    resolver.instruction_type(*target).ok_or_else(unknown)?
                else {
                    return Err(Compatibility::Incompatible);
                };
                for v in elements {
                    require(*v, *component.clone());
                }
            }
            ManagedInsn::NewFilledArray { .. } => {}
            ManagedInsn::FillArray { array, values } if has(array) || values.iter().any(has) => {
                let array_ty = ty(*array)?;
                let JavaType::Array(component) = &array_ty else {
                    return Err(Compatibility::Incompatible);
                };
                for v in values {
                    require(*v, *component.clone());
                }
                require(*array, array_ty);
            }
            ManagedInsn::FillArray { .. } => {}
            ManagedInsn::ArrayLength { array, .. } if has(array) => {
                let t = ty(*array)?;
                if !matches!(t, JavaType::Array(_)) {
                    return Err(Compatibility::Incompatible);
                }
                require(*array, t);
            }
            ManagedInsn::ArrayLength { .. } => {}
            ManagedInsn::NewArray { length, .. } => require(*length, JavaType::Int),
            ManagedInsn::Arithmetic {
                result,
                lhs,
                rhs,
                op,
            } if has(lhs) || has(rhs) => {
                let t = ty(*result)?;
                if !matches!(
                    t,
                    JavaType::Int | JavaType::Long | JavaType::Float | JavaType::Double
                ) {
                    return Err(Compatibility::Incompatible);
                }
                require(*lhs, t.clone());
                require(
                    *rhs,
                    if matches!(
                        op,
                        managed_ir::ops::ArithOp::ShiftLeft
                            | managed_ir::ops::ArithOp::ShiftRight
                            | managed_ir::ops::ArithOp::UnsignedShiftRight
                    ) {
                        JavaType::Int
                    } else {
                        t
                    },
                );
            }
            ManagedInsn::Arithmetic { .. } => {}
            ManagedInsn::Neg { result, value } | ManagedInsn::Not { result, value }
                if has(value) =>
            {
                require(*value, ty(*result)?)
            }
            ManagedInsn::Neg { .. } | ManagedInsn::Not { .. } => {}
            ManagedInsn::Branch {
                condition: Some(v), ..
            } => require(*v, JavaType::Boolean),
            ManagedInsn::Branch {
                condition: None, ..
            } => {}
            ManagedInsn::Switch { key, .. } => require(*key, JavaType::Int),
            ManagedInsn::Throw { value } => {
                require(*value, JavaType::Class("java.lang.Throwable".into()))
            }
            // These operand roles require additional opcode/reference evidence.
            // An annotation alone cannot establish the expected operand type.
            ManagedInsn::Compare { op, lhs, rhs, .. } if has(lhs) || has(rhs) => {
                let expected_peer = |local: ValueId, peer: ValueId| {
                    if let Ok(peer_type) = ty(peer) {
                        // DEX comparisons retain a Boolean peer's exact 0/1
                        // bits when the selected local has a wider int domain.
                        // This proves conversion of the peer, never arbitrary
                        // int-to-Boolean coercion of the selected family.
                        if declarations.get(&local) == Some(&JavaType::Int)
                            && peer_type == JavaType::Boolean
                            && category_one_expression(
                                ir,
                                peer,
                                &peer_type,
                                &JavaType::Int,
                                resolver,
                            )
                            .is_some()
                        {
                            return Ok(JavaType::Int);
                        }
                        return Ok(peer_type);
                    }
                    let declared = declarations.get(&local).ok_or_else(unknown)?;
                    let equality = matches!(
                        op,
                        managed_ir::ops::CompareOp::Equal | managed_ir::ops::CompareOp::NotEqual
                    );
                    let comparable = matches!(
                        declared,
                        JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int
                    ) || equality
                        && (declared.reference() || *declared == JavaType::Boolean);
                    if !comparable
                        || !matches!(
                            evidence(ir, peer, resolver)?,
                            ValueType::RawZero | ValueType::Null
                        )
                    {
                        return Err(unknown());
                    }
                    // A definition-proven literal can be null/zero at this
                    // category-one comparison use. Never reinterpret an arbitrary Unknown or
                    // selected Phi from its annotation; the family declaration
                    // still must satisfy every other consumer and definition.
                    expression(ir, peer, declared, &Hierarchy::default(), resolver)?;
                    Ok(declared.clone())
                };
                if has(lhs) {
                    require(*lhs, expected_peer(*lhs, *rhs)?);
                }
                if has(rhs) {
                    require(*rhs, expected_peer(*rhs, *lhs)?);
                }
            }
            ManagedInsn::Compare { .. } => {}
            ManagedInsn::OrderingCompare { op, lhs, rhs, .. } if has(lhs) || has(rhs) => {
                let expected = ordering_type(*op);
                if has(lhs) {
                    require(*lhs, expected.clone());
                }
                if has(rhs) {
                    require(*rhs, expected);
                }
            }
            ManagedInsn::OrderingCompare { .. } => {}
            ManagedInsn::Cast { value, target, .. } if has(value) => {
                let target = resolver.instruction_type(*target).ok_or_else(unknown)?;
                if target.numeric() {
                    // Conversion consumes the source numeric category, not the
                    // target category (int-to-long still consumes category one).
                    let source = match declarations
                        .get(value)
                        .cloned()
                        .map(ValueType::Known)
                        .map(Ok)
                        .unwrap_or_else(|| evidence(ir, *value, resolver))?
                    {
                        ValueType::RawZero => JavaType::Int,
                        ValueType::Known(source) if source.numeric() => source,
                        _ => return Err(Compatibility::Incompatible),
                    };
                    require(*value, source);
                } else if target.reference() {
                    require(*value, JavaType::Class("java.lang.Object".into()));
                } else {
                    return Err(Compatibility::Incompatible);
                }
            }
            ManagedInsn::InstanceOf { value, target, .. } if has(value) => {
                match resolver.instruction_type(*target) {
                    Some(JavaType::Class(_) | JavaType::Array(_)) => {
                        require(*value, JavaType::Class("java.lang.Object".into()));
                    }
                    Some(_) => return Err(Compatibility::Incompatible),
                    None => return Err(unknown()),
                }
            }
            ManagedInsn::Cast { .. } | ManagedInsn::InstanceOf { .. } => {}
            ManagedInsn::MonitorEnter { object } | ManagedInsn::MonitorExit { object }
                if has(object) =>
            {
                return Err(unknown());
            }
            ManagedInsn::MonitorEnter { .. } | ManagedInsn::MonitorExit { .. } => {}
            ManagedInsn::Opaque { reads, .. } if reads.iter().any(has) => return Err(unknown()),
            ManagedInsn::Opaque { .. } => {}
        }
    }
    Ok(constraints)
}

fn ordering_type(op: OrderingOp) -> JavaType {
    match op {
        OrderingOp::CmpLowFloat | OrderingOp::CmpHighFloat => JavaType::Float,
        OrderingOp::CmpLowDouble | OrderingOp::CmpHighDouble => JavaType::Double,
        OrderingOp::CmpLong => JavaType::Long,
    }
}

/// Parent declaration-plan hook. Collect all family uses internally before
/// proving the proposed concrete declaration; never infer Object or int defaults.
pub fn checked_phi_plan(
    ir: &MethodIr<SsaForm>,
    result: ValueId,
    declared: &JavaType,
    return_type: &JavaType,
    hierarchy: &Hierarchy,
    resolver: &impl JavaResolver,
) -> Result<PhiPlan, Compatibility> {
    checked_phi_plan_indexed(
        ir,
        &MethodIndex::new(ir),
        result,
        declared,
        return_type,
        hierarchy,
        resolver,
    )
}

fn checked_phi_plan_indexed(
    ir: &MethodIr<SsaForm>,
    index: &MethodIndex,
    result: ValueId,
    declared: &JavaType,
    return_type: &JavaType,
    hierarchy: &Hierarchy,
    resolver: &impl JavaResolver,
) -> Result<PhiPlan, Compatibility> {
    let proof = phi_proof_indexed(ir, index, result, declared, &[], hierarchy, resolver)?;
    let mut all_declarations = proof.preserved.clone();
    for local in &proof.plan.locals {
        all_declarations.insert(*local, declared.clone());
    }
    // Preserving a producer's declaration does not excuse an invalid original
    // use. Validate its full arena consumers against that unchanged subtype;
    // these locals never enter the selected family's declaration map.
    for (value, actual) in &proof.preserved {
        let contexts = consumer_constraints_indexed(
            ir,
            index,
            &[*value],
            return_type,
            resolver,
            &all_declarations,
        )?;
        if contexts
            .iter()
            .any(|expected| !resolver.assignable(actual, expected))
        {
            return Err(Compatibility::Incompatible);
        }
    }
    let declarations = proof
        .plan
        .locals
        .iter()
        .map(|v| (*v, declared.clone()))
        .collect();
    let consumers = consumer_constraints_indexed(
        ir,
        index,
        &proof.plan.locals,
        return_type,
        resolver,
        &declarations,
    )?;
    check_phi_consumers(
        ir,
        result,
        declared,
        &proof.leaves,
        &consumers,
        hierarchy,
        resolver,
    )?;
    Ok(proof.plan)
}

/// Atomic declaration refinements for selected phi/copy families. Candidates
/// come from genuine descriptors; every reaching definition and arena consumer
/// must accept the same declaration before any family member is adopted.
pub fn declaration_plan(
    ir: &MethodIr<SsaForm>,
    return_type: &JavaType,
    resolver: &impl JavaResolver,
) -> Result<std::collections::HashMap<ValueId, JavaType>, Compatibility> {
    declaration_plan_with_caught_producers(ir, return_type, resolver, &HashMap::new())
}

// Only the exception dispatcher passes original metadata-attested producers.
// Syntactic identity and width checks remain independent; no arbitrary Object
// or Unknown annotation is promoted to a caught-object source declaration.
pub(crate) fn declaration_plan_with_caught_producers(
    ir: &MethodIr<SsaForm>,
    return_type: &JavaType,
    resolver: &impl JavaResolver,
    caught: &HashMap<ValueId, JavaType>,
) -> Result<HashMap<ValueId, JavaType>, Compatibility> {
    let mut index = MethodIndex::new(ir);
    for (&value, ty) in caught {
        let record = ir.value(value).map_err(|_| unknown())?;
        if !ty.reference()
            || !matches!(ir.instruction(record.def()),
            Ok(ManagedInsn::Opaque {family_tag:0x8002,reads,writes,effects})
                if reads.is_empty() && writes.as_slice()==[value]
                && matches!(*effects, managed_ir::effects::EffectFlags::NONE | managed_ir::effects::EffectFlags::MAY_THROW))
        {
            return Err(Compatibility::Incompatible);
        }
        index.caught.insert(value, ty.clone());
    }
    let mut declarations = caught.clone();
    let mut candidates = vec![return_type.clone()];
    candidates.extend(
        ir.values()
            .keys()
            .filter_map(|value| caught.get(&value).cloned()),
    );
    for (_, value) in ir.values() {
        if let Some(ty) = nominal(value.ty(), resolver)
            && !candidates.contains(&ty)
        {
            candidates.push(ty);
        }
    }
    // Literal bits are producer evidence even if comparison inference has
    // spread a Boolean annotation. Such a domain requires an int candidate;
    // the family proof below still rejects every incompatible consumer.
    if !candidates.contains(&JavaType::Int)
        && ir.instructions().iter().any(|(_, instruction)| {
            matches!(instruction, ManagedInsn::Const { result, value: Constant::Int(bits) }
                if !matches!(bits, 0 | 1)
                    && ir.value(*result).is_ok_and(|v| v.ty() == &managed_ir::types::ManagedType::Boolean)
                    && evidence(ir, *result, resolver) == Ok(ValueType::Known(JavaType::Int)))
        })
    {
        candidates.push(JavaType::Int);
    }
    for (_, instruction) in ir.instructions() {
        let ManagedInsn::Phi { result, .. } = instruction else {
            continue;
        };
        if declarations.contains_key(result) {
            continue;
        }
        // Keep already concrete, homogeneous families on their existing path.
        let record = ir.value(*result).map_err(|_| unknown())?;
        let mut choices = Vec::new();
        if let Some(ty) = nominal(record.ty(), resolver) {
            choices.push(ty);
        }
        // Candidate discovery is independent per consumer: a null comparison
        // can require the declaration before its own constraint is knowable.
        // Later descriptor-backed uses still supply candidates (for example
        // TextView/ImageView -> View). Adoption below retains the complete
        // reaching-definition and full-arena consumer proof, including any
        // consumer whose constraint could not yet be discovered here.
        let family = index.copy_closure(&[*result]);
        for position in index.consumer_positions(&family) {
            if let Ok(contexts) = consumer_constraints_at_positions(
                ir,
                &index,
                &family,
                &[position],
                return_type,
                resolver,
                &HashMap::new(),
            ) {
                for context in contexts {
                    if !choices.contains(&context) {
                        choices.push(context);
                    }
                }
            }
        }
        if index.thrown.contains(result) {
            for name in ["java.lang.RuntimeException", "java.lang.Error"] {
                let ty = JavaType::Class(name.into());
                if !choices.contains(&ty) {
                    choices.insert(0, ty);
                }
            }
        }
        choices.extend(
            candidates
                .iter()
                .filter(|t| !choices.contains(t))
                .cloned()
                .collect::<Vec<_>>(),
        );
        // Throw is an upper-bound use constraint, not evidence that a selected
        // IOException must be widened to Throwable. Keep descriptor candidates
        // ahead of that bound so checked source contracts retain their precision.
        if family.iter().any(|value| index.thrown.contains(value)) {
            choices.sort_by_key(|ty| *ty == JavaType::Class("java.lang.Throwable".into()));
        }
        let selected = choices.iter().find_map(|declared| {
            checked_phi_plan_indexed(
                ir,
                &index,
                *result,
                declared,
                return_type,
                &Hierarchy::default(),
                resolver,
            )
            .ok()
            .map(|plan| (declared.clone(), plan))
        });
        let Some((declared, plan)) = selected else {
            continue;
        };
        for local in &plan.locals {
            if declarations.get(local).is_some_and(|old| old != &declared) {
                return Err(Compatibility::Incompatible);
            }
        }
        for local in plan.locals {
            declarations.insert(local, declared.clone());
        }
    }
    // Hoisting precedes instruction emission, so the constant-emission hook
    // cannot supply the type of an unresolved literal local in a branching
    // method. Establish descriptor-backed declarations for pure literal/copy
    // families before that hoisting step. Never use a Null annotation as proof.
    // A non-0/1 category-one literal cannot have a Boolean representation.
    // Inference may have propagated that annotation through an equality with
    // a real Boolean. Materialize its exact int bits only when every copy,
    // selected Phi input, and other consumer accepts the corrected declaration.
    for (_, instruction) in ir.instructions() {
        let ManagedInsn::Const {
            result,
            value: Constant::Int(bits),
        } = instruction
        else {
            continue;
        };
        if matches!(bits, 0 | 1)
            || ir.value(*result).map_err(|_| unknown())?.ty()
                != &managed_ir::types::ManagedType::Boolean
        {
            continue;
        }
        let family = index.copy_closure(&[*result]);
        if family.iter().any(|value| {
            index.definitions.get(value) != Some(&1)
                || evidence(ir, *value, resolver) != Ok(ValueType::Known(JavaType::Int))
        }) {
            continue;
        }
        let mut proposed = declarations.clone();
        for value in &family {
            proposed.insert(*value, JavaType::Int);
        }
        let Ok(contexts) = consumer_constraints_indexed(
            ir,
            &index,
            &family.iter().copied().collect::<Vec<_>>(),
            return_type,
            resolver,
            &proposed,
        ) else {
            continue;
        };
        if contexts
            .iter()
            .all(|expected| resolver.assignable(&JavaType::Int, expected))
        {
            for value in family {
                if declarations
                    .get(&value)
                    .is_some_and(|old| old != &JavaType::Int)
                {
                    return Err(Compatibility::Incompatible);
                }
                declarations.insert(value, JavaType::Int);
            }
        }
    }
    for (_, instruction) in ir.instructions() {
        let ManagedInsn::Const {
            result,
            value: Constant::Null | Constant::Zero,
        } = instruction
        else {
            continue;
        };
        let record = ir.value(*result).map_err(|_| unknown())?;
        if !matches!(
            record.ty(),
            managed_ir::types::ManagedType::Null | managed_ir::types::ManagedType::Zero
        ) {
            continue;
        }
        let mut family = Vec::new();
        let mut seen = HashSet::new();
        let mut pending = vec![*result];
        while let Some(value) = pending.pop() {
            if !seen.insert(value) {
                continue;
            }
            // Moves preserve the exact pure literal evidence. Phis are excluded:
            // selecting a predecessor remains the phi planner's responsibility.
            if !matches!(
                evidence(ir, value, resolver),
                Ok(ValueType::Null | ValueType::RawZero)
            ) {
                return Err(unknown());
            }
            family.push(value);
            pending.extend(index.copies.get(&value).into_iter().flatten().copied());
        }
        let Ok(contexts) =
            consumer_constraints_indexed(ir, &index, &family, return_type, resolver, &declarations)
        else {
            continue;
        };
        let Some(declared) = contexts.first() else {
            continue;
        };
        for expected in &contexts {
            expression(ir, *result, expected, &Hierarchy::default(), resolver)?;
        }
        // Every family use supplies its own proven literal coercion. A shared
        // local merely records materialization; no nominal ancestry is invented.
        for value in family {
            if nominal(ir.value(value).map_err(|_| unknown())?.ty(), resolver).is_none() {
                declarations
                    .entry(value)
                    .or_insert_with(|| declared.clone());
            }
        }
    }
    // Hoisted pure category-one zeros still materialize on their original path;
    // each operand/edge uses its separately proven coercion.
    for (_, instruction) in ir.instructions() {
        if let ManagedInsn::Const {
            result,
            value: Constant::Zero,
        } = instruction
            && ir.value(*result).map_err(|_| unknown())?.ty()
                == &managed_ir::types::ManagedType::Zero
        {
            declarations.insert(*result, JavaType::Int);
        }
    }
    // Exact producer contracts may repair widened reference copy locals only
    // after every full-arena consumer and definition has been checked.
    for (value, concrete) in
        producer_reference_locals::declarations_indexed(ir, &index, return_type, resolver)
    {
        if declarations
            .get(&value)
            .is_some_and(|existing| existing != &concrete)
        {
            return Err(Compatibility::Incompatible);
        }
        declarations.insert(value, concrete);
    }
    Ok(declarations)
}

fn contextual_type(
    h: &Hierarchy,
    value: &ValueType,
    expected: &JavaType,
) -> Result<Coercion, Compatibility> {
    // DEX Zero is category one. Category-two zero is Const::Long(0),
    // whose bits require separate proven long/double interpretation.
    if *value == ValueType::RawZero && matches!(expected, JavaType::Long | JavaType::Double) {
        return Err(Compatibility::Incompatible);
    }
    expected_type(h, value, expected)
}

fn nominal(ty: &managed_ir::types::ManagedType, resolver: &impl JavaResolver) -> Option<JavaType> {
    use managed_ir::types::ManagedType as M;
    Some(match ty {
        M::Boolean => JavaType::Boolean,
        M::Byte => JavaType::Byte,
        M::Short => JavaType::Short,
        M::Char => JavaType::Char,
        M::Int => JavaType::Int,
        M::Long => JavaType::Long,
        M::Float => JavaType::Float,
        M::Double => JavaType::Double,
        M::Reference(class) => JavaType::Class(resolver.class(*class)?),
        M::Array { element, .. } => JavaType::Array(Box::new(nominal(element, resolver)?)),
        _ => return None,
    })
}

/// An expression replacement is legal only for proven pure constant/copy chains.
/// Ordinary values remain identity expressions and retain their effects/order.
pub fn expression(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    expected: &JavaType,
    hierarchy: &Hierarchy,
    resolver: &impl JavaResolver,
) -> Result<Coercion, Compatibility> {
    if let Some(bit) = category_one_literal(ir, value) {
        if *expected == JavaType::Boolean {
            return Ok(Coercion::ZeroLiteral(if bit { "true" } else { "false" }));
        }
        if *expected == JavaType::Int
            && evidence(ir, value, resolver)? == ValueType::Known(JavaType::Boolean)
        {
            return Ok(Coercion::ZeroLiteral(if bit { "1" } else { "0" }));
        }
    }
    contextual_type(hierarchy, &evidence(ir, value, resolver)?, expected)
}

/// Obtain evidence from definitions, rather than interpreting an arbitrary
/// Unknown/Null/Zero annotation as permission to replace a value.
pub fn evidence(
    ir: &MethodIr<SsaForm>,
    mut value: ValueId,
    resolver: &impl JavaResolver,
) -> Result<ValueType, Compatibility> {
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(value) {
            return Err(Compatibility::Indeterminate(UnsupportedType::UnknownValue));
        }
        let record = ir
            .value(value)
            .map_err(|_| Compatibility::Indeterminate(UnsupportedType::UnknownValue))?;
        match ir
            .instruction(record.def())
            .map_err(|_| Compatibility::Indeterminate(UnsupportedType::UnknownValue))?
        {
            ManagedInsn::Const {
                result,
                value: Constant::Zero,
            } if *result == value => return Ok(ValueType::RawZero),
            ManagedInsn::Const {
                result,
                value: Constant::Null,
            } if *result == value => return Ok(ValueType::Null),
            ManagedInsn::Const {
                result,
                value: Constant::Int(bits),
            } if *result == value
                && !matches!(bits, 0 | 1)
                && *record.ty() == managed_ir::types::ManagedType::Boolean =>
            {
                return Ok(ValueType::Known(JavaType::Int));
            }
            ManagedInsn::Move {
                result,
                value: source,
            } if *result == value => value = *source,
            // A phi is a real selected value, never an unconditional literal.
            _ => {
                return nominal(record.ty(), resolver)
                    .map(ValueType::Known)
                    .ok_or(Compatibility::Indeterminate(UnsupportedType::UnknownValue));
            }
        }
    }
}

/// Check the actual reaching definitions and all consumers of one phi local.
/// The caller supplies a candidate declaration from signature/use constraints;
/// no Object fallback or casts are invented. Unknown leaves remain unsupported.
/// Result order is the original immediate input order for edge-copy emission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhiPlan {
    /// Apply the candidate to these phi/copy and attested producer locals together.
    /// `consumers` must cover all their non-copy/non-phi uses, including escapes.
    pub locals: Vec<ValueId>,
    /// Original immediate inputs, for simultaneous predecessor edge copies.
    pub inputs: Vec<(ValueId, Coercion)>,
}

/// Build a phi declaration proof from concrete reaching definitions. Callers
/// must provide all consumers of the returned local family, apply its declaration
/// atomically, and preserve predecessor selection and simultaneous edge copies.
pub fn phi_plan(
    ir: &MethodIr<SsaForm>,
    result: ValueId,
    declared: &JavaType,
    consumers: &[JavaType],
    hierarchy: &Hierarchy,
    resolver: &impl JavaResolver,
) -> Result<PhiPlan, Compatibility> {
    phi_proof_indexed(
        ir,
        &MethodIndex::new(ir),
        result,
        declared,
        consumers,
        hierarchy,
        resolver,
    )
    .map(|proof| proof.plan)
}

struct PhiProof {
    plan: PhiPlan,
    leaves: Vec<ValueType>,
    preserved: HashMap<ValueId, JavaType>,
}

/// Genuine reference producer contracts can repair a widened local annotation.
/// The caller still proves every copy, Phi input and full-arena consumer before
/// changing its materialized declaration. Unknown producers are never refined.
fn reference_producer(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    resolver: &impl JavaResolver,
) -> Option<JavaType> {
    let record = ir.value(value).ok()?;
    let ty = match ir.instruction(record.def()).ok()? {
        ManagedInsn::Invoke {
            result: Some(result),
            method,
            kind,
            args,
        } if *result == value
            && matches!(
                kind,
                managed_ir::ops::InvokeKind::Static
                    | managed_ir::ops::InvokeKind::Virtual
                    | managed_ir::ops::InvokeKind::Interface
            ) =>
        {
            let target = resolver.method(*method)?;
            if target.is_static != (*kind == managed_ir::ops::InvokeKind::Static) {
                return None;
            }
            crate::invoke_arguments::operands(&target, args).ok()?;
            target.return_type
        }
        ManagedInsn::FieldGet {
            result,
            field,
            object,
        } if *result == value => {
            let field = resolver.field(*field)?;
            if field.is_static != object.is_none() {
                return None;
            }
            field.ty
        }
        ManagedInsn::Cast { result, target, .. } if *result == value => {
            resolver.instruction_type(*target)?
        }
        _ => return None,
    };
    ty.reference().then_some(ty)
}

/// Some source positions consume a materialized local directly. A bit-domain
/// proof permits conversions only at operand sites that actually emit them;
/// it cannot turn a Boolean local into a Java array index or an int into an if.
fn check_direct_consumers(
    ir: &MethodIr<SsaForm>,
    index: &MethodIndex,
    locals: &[ValueId],
    declared: &JavaType,
) -> Result<(), Compatibility> {
    let locals: HashSet<_> = locals.iter().copied().collect();
    for position in index.consumer_positions(&locals) {
        let instruction = ir
            .instruction(index.instructions[position])
            .map_err(|_| unknown())?;
        let direct_int = match instruction {
            ManagedInsn::ArrayGet { index, .. } | ManagedInsn::ArrayPut { index, .. } => {
                locals.contains(index)
            }
            ManagedInsn::NewArray { length, .. } => locals.contains(length),
            ManagedInsn::Switch { key, .. } => locals.contains(key),
            _ => false,
        };
        if direct_int
            && !matches!(
                declared,
                JavaType::Int | JavaType::Byte | JavaType::Short | JavaType::Char
            )
        {
            return Err(Compatibility::Incompatible);
        }
        if matches!(instruction, ManagedInsn::Branch { condition: Some(value), .. } if locals.contains(value))
            && *declared != JavaType::Boolean
        {
            return Err(Compatibility::Incompatible);
        }
    }
    Ok(())
}

fn preserved_reference_copy(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    declared: &JavaType,
    resolver: &impl JavaResolver,
) -> Option<JavaType> {
    if !declared.reference() {
        return None;
    }
    let record = ir.value(value).ok()?;
    if !matches!(ir.instruction(record.def()).ok()?, ManagedInsn::Move { result, .. } if *result == value)
    {
        return None;
    }
    let ValueType::Known(actual) = evidence(ir, value, resolver).ok()? else {
        return None;
    };
    // A preserved copy is not adopted by the returned Phi declaration plan.
    // Its existing materialized slot must therefore already have this exact
    // producer type. Mismatched families use the ordinary atomic refinement
    // path, which can reach a widened Cast/Invoke/FieldGet producer.
    if !actual.reference()
        || nominal(record.ty(), resolver).as_ref() != Some(&actual)
        || !resolver.assignable(&actual, declared)
    {
        return None;
    }
    let mut current = value;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        let record = ir.value(current).ok()?;
        match ir.instruction(record.def()).ok()? {
            ManagedInsn::Move {
                result,
                value: source,
            } if *result == current => {
                if nominal(record.ty(), resolver).as_ref() != Some(&actual) {
                    return None;
                }
                current = *source;
            }
            ManagedInsn::Phi { .. } => return None,
            _ => {
                if nominal(record.ty(), resolver).as_ref() != Some(&actual)
                    || reference_producer(ir, current, resolver)
                        .is_some_and(|produced| produced != actual)
                {
                    return None;
                }
                return Some(actual);
            }
        }
    }
    None
}

fn phi_proof_indexed(
    ir: &MethodIr<SsaForm>,
    index: &MethodIndex,
    result: ValueId,
    declared: &JavaType,
    consumers: &[JavaType],
    hierarchy: &Hierarchy,
    resolver: &impl JavaResolver,
) -> Result<PhiProof, Compatibility> {
    let record = ir
        .value(result)
        .map_err(|_| Compatibility::Indeterminate(UnsupportedType::UnknownValue))?;
    let ManagedInsn::Phi {
        result: defined,
        inputs,
    } = ir
        .instruction(record.def())
        .map_err(|_| Compatibility::Indeterminate(UnsupportedType::UnknownValue))?
    else {
        return Err(Compatibility::Incompatible);
    };
    if *defined != result || inputs.is_empty() {
        return Err(Compatibility::Incompatible);
    }
    // Expand phi/copy cycles; every concrete reaching definition must assign to
    // the declaration. An SCC with no external input has no typing evidence.
    // A reference producer feeding a wider selected value retains its own
    // concrete declaration, including array operations before the merge.
    let mut pending = vec![result];
    let mut seen = HashSet::new();
    let mut leaves = Vec::new();
    let mut locals = Vec::new();
    let mut preserved = HashMap::new();
    while let Some(value) = pending.pop() {
        if !seen.insert(value) {
            continue;
        }
        // Include every downstream copy and phi, then expand their inputs too.
        // Literal leaves are not added to this family: their uses can split.
        let definition = ir.value(value).map_err(|_| unknown())?.def();
        let pure_literal = matches!(
            evidence(ir, value, resolver),
            Ok(ValueType::RawZero | ValueType::Null)
        );
        // A concrete producer copy feeding this Phi is not itself a selected
        // Phi local. Preserve its original subtype (including constructor
        // receivers) while proving it can assign into the common declaration.
        // Copies rooted in another Phi remain part of the atomic family.
        let preserved_reference = preserved_reference_copy(ir, value, declared, resolver);
        if let Some(actual) = preserved_reference {
            preserved.insert(value, actual.clone());
            leaves.push(ValueType::Known(actual));
            // The producer can also feed a sibling selected join through
            // another copy. Preserve its subtype, but prove every downstream
            // copy/Phi declaration in the same atomic family before validating
            // the producer's complete consumer set.
            pending.extend(index.downstream.get(&value).into_iter().flatten().copied());
            continue;
        }
        let selected = !pure_literal
            && matches!(
                ir.instruction(definition),
                Ok(ManagedInsn::Phi { .. } | ManagedInsn::Move { .. })
            );
        if selected {
            pending.extend(index.downstream.get(&value).into_iter().flatten().copied());
        }
        let record = ir
            .value(value)
            .map_err(|_| Compatibility::Indeterminate(UnsupportedType::UnknownValue))?;
        match ir
            .instruction(record.def())
            .map_err(|_| Compatibility::Indeterminate(UnsupportedType::UnknownValue))?
        {
            ManagedInsn::Phi {
                result: defined,
                inputs,
            } if *defined == value => {
                if inputs.is_empty() {
                    return Err(Compatibility::Incompatible);
                }
                locals.push(value);
                pending.extend(inputs.iter().map(|(_, v)| *v));
            }
            ManagedInsn::Move {
                result: defined,
                value: source,
            } if *defined == value && !pure_literal => {
                locals.push(value);
                pending.push(*source);
            }
            _ => {
                let mut evidence = if let Some(ty) = index.caught.get(&value) {
                    locals.push(value);
                    pending.extend(index.downstream.get(&value).into_iter().flatten().copied());
                    ValueType::Known(ty.clone())
                } else {
                    evidence(ir, value, resolver)?
                };
                if let ValueType::Known(actual) = &evidence
                    && actual.reference()
                    && declared.reference()
                    && !resolver.assignable(actual, declared)
                    && let Some(produced) = reference_producer(ir, value, resolver)
                    && resolver.assignable(&produced, actual)
                    && resolver.assignable(&produced, declared)
                    && resolver.assignable(declared, actual)
                {
                    // Refine the actual producer slot as part of the atomic
                    // family, rather than inventing a narrowing edge cast.
                    locals.push(value);
                    pending.extend(index.downstream.get(&value).into_iter().flatten().copied());
                    evidence = ValueType::Known(produced);
                }
                let actual = match &evidence {
                    ValueType::Known(ty) => ty.clone(),
                    _ => JavaType::Int,
                };
                let evidence =
                    if category_one_expression(ir, value, &actual, declared, resolver).is_some() {
                        ValueType::Known(declared.clone())
                    } else {
                        evidence
                    };
                leaves.push(evidence);
            }
        }
    }
    if leaves.is_empty() {
        return Err(Compatibility::Indeterminate(UnsupportedType::UnknownValue));
    }
    check_direct_consumers(ir, index, &locals, declared)?;
    // The production resolver carries genuine input/classpath hierarchy edges;
    // the caller's generic hierarchy may intentionally be empty. Normalize only
    // positively attested assignments, never unknown or nominal-only guesses.
    let leaves: Vec<_> = leaves
        .into_iter()
        .map(|leaf| match leaf {
            ValueType::Known(ref actual) if resolver.assignable(actual, declared) => {
                ValueType::Known(declared.clone())
            }
            other => other,
        })
        .collect();
    for leaf in &leaves {
        contextual_type(hierarchy, leaf, declared)?;
    }
    check_phi_consumers(
        ir, result, declared, &leaves, consumers, hierarchy, resolver,
    )?;
    let inputs = inputs
        .iter()
        .map(|(_, value)| {
            // Nested phis/copies must retain the selected local, even if every
            // external leaf happens to be constant; edge/value flow is preserved.
            let coercion = if let Some(bit) = category_one_literal(ir, *value) {
                match declared {
                    JavaType::Boolean => Coercion::ZeroLiteral(if bit { "true" } else { "false" }),
                    JavaType::Int
                        if matches!(
                            evidence(ir, *value, resolver),
                            Ok(ValueType::Known(JavaType::Boolean))
                        ) =>
                    {
                        Coercion::ZeroLiteral(if bit { "1" } else { "0" })
                    }
                    _ => expression(ir, *value, declared, hierarchy, resolver)?,
                }
            } else {
                match evidence(ir, *value, resolver) {
                    Ok(ValueType::RawZero) => {
                        contextual_type(hierarchy, &ValueType::RawZero, declared)?
                    }
                    Ok(ValueType::Null) => expected_type(hierarchy, &ValueType::Null, declared)?,
                    _ => Coercion::Identity,
                }
            };
            Ok((*value, coercion))
        })
        .collect::<Result<Vec<_>, Compatibility>>()?;
    Ok(PhiProof {
        plan: PhiPlan { locals, inputs },
        leaves,
        preserved,
    })
}

fn check_phi_consumers(
    ir: &MethodIr<SsaForm>,
    result: ValueId,
    declared: &JavaType,
    leaves: &[ValueType],
    consumers: &[JavaType],
    hierarchy: &Hierarchy,
    resolver: &impl JavaResolver,
) -> Result<(), Compatibility> {
    let uses = consumers
        .iter()
        .map(|expected| {
            if resolver.assignable(declared, expected) {
                return declared.clone();
            }
            if matches!(
                (declared, expected),
                (JavaType::Boolean, JavaType::Int) | (JavaType::Int, JavaType::Boolean)
            ) && boolean_bits(ir, result, resolver)
            {
                declared.clone()
            } else {
                expected.clone()
            }
        })
        .collect::<Vec<_>>();
    check_local_plan(hierarchy, declared, leaves, &uses)?;
    Ok(())
}

/// Category-one source conversion justified by exact value definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CategoryOneCoercion {
    /// Pure constant spelling, safe to substitute at each use.
    Literal(&'static str),
    /// Retain one Boolean expression evaluation and expose its 0/1 bits.
    BooleanToInt,
    /// An integer is proved to contain only 0 or 1, including all phi inputs.
    IntBitsToBoolean,
}

fn category_one_literal(ir: &MethodIr<SsaForm>, mut value: ValueId) -> Option<bool> {
    let mut seen = HashSet::new();
    while seen.insert(value) {
        let record = ir.value(value).ok()?;
        match ir.instruction(record.def()).ok()? {
            ManagedInsn::Const {
                result,
                value: Constant::Boolean(bit),
            } if *result == value => return Some(*bit),
            ManagedInsn::Const {
                result,
                value: Constant::Zero | Constant::Int(0),
            } if *result == value => return Some(false),
            ManagedInsn::Const {
                result,
                value: Constant::Int(1),
            } if *result == value => return Some(true),
            ManagedInsn::Move {
                result,
                value: source,
            } if *result == value => value = *source,
            _ => return None,
        }
    }
    None
}

/// Prove a category-one value is restricted to the Boolean bit domain. Nominal
/// annotations alone never prove an arbitrary integer, opaque read, or cycle.
pub fn boolean_bits(ir: &MethodIr<SsaForm>, value: ValueId, resolver: &impl JavaResolver) -> bool {
    boolean_bits_with_parameters(ir, value, resolver, &|_| false)
}

pub(crate) fn boolean_bits_with_parameters(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    resolver: &impl JavaResolver,
    parameter: &impl Fn(ValueId) -> bool,
) -> bool {
    let mut pending = vec![value];
    let mut seen = HashSet::new();
    let mut concrete = false;
    while let Some(value) = pending.pop() {
        if !seen.insert(value) {
            continue;
        }
        let Ok(record) = ir.value(value) else {
            return false;
        };
        let Ok(instruction) = ir.instruction(record.def()) else {
            return false;
        };
        match instruction {
            ManagedInsn::Opaque {
                family_tag: 0x8001,
                reads,
                writes,
                effects,
            } if parameter(value)
                && reads.is_empty()
                && writes.as_slice() == [value]
                && *effects == managed_ir::effects::EffectFlags::NONE =>
            {
                concrete = true;
            }
            ManagedInsn::Const {
                result,
                value: Constant::Boolean(_) | Constant::Zero | Constant::Int(0 | 1),
            } if *result == value => concrete = true,
            ManagedInsn::Move {
                result,
                value: source,
            } if *result == value => pending.push(*source),
            ManagedInsn::Phi { result, inputs } if *result == value && !inputs.is_empty() => {
                pending.extend(inputs.iter().map(|(_, value)| *value))
            }
            ManagedInsn::Compare { result, .. } | ManagedInsn::InstanceOf { result, .. }
                if *result == value =>
            {
                concrete = true
            }
            ManagedInsn::FieldGet { result, field, .. } if *result == value => {
                if !resolver
                    .field(*field)
                    .is_some_and(|field| field.ty == JavaType::Boolean)
                {
                    return false;
                }
                concrete = true;
            }
            ManagedInsn::Invoke {
                result: Some(result),
                method,
                ..
            } if *result == value => {
                if !resolver
                    .method(*method)
                    .is_some_and(|method| method.return_type == JavaType::Boolean)
                {
                    return false;
                }
                concrete = true;
            }
            ManagedInsn::Arithmetic {
                result,
                op:
                    managed_ir::ops::ArithOp::And
                    | managed_ir::ops::ArithOp::Or
                    | managed_ir::ops::ArithOp::Xor,
                lhs,
                rhs,
            } if *result == value => {
                if !matches!(
                    record.ty(),
                    managed_ir::types::ManagedType::Boolean
                        | managed_ir::types::ManagedType::Int
                        | managed_ir::types::ManagedType::Byte
                        | managed_ir::types::ManagedType::Short
                        | managed_ir::types::ManagedType::Char
                        | managed_ir::types::ManagedType::Zero
                ) {
                    return false;
                }
                pending.extend([*lhs, *rhs]);
            }
            _ => return false,
        }
    }
    concrete
}

/// A definition-proven category-one literal/copy fits every narrow integer
/// descriptor. The caller retains its materialized expression and availability;
/// this does not permit narrowing an arbitrary integer or selected Phi.
pub fn category_one_narrowing(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    actual: &JavaType,
    expected: &JavaType,
) -> bool {
    *actual == JavaType::Int
        && matches!(expected, JavaType::Byte | JavaType::Short | JavaType::Char)
        && category_one_literal(ir, value).is_some()
}

/// Parent operand hook. `actual` is the materialized Java expression type. No
/// integer-to-Boolean conversion is allowed without the complete bit-domain
/// proof; category-two, reference, unknown and non-0/1 values stay unsupported.
pub fn category_one_expression(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    actual: &JavaType,
    expected: &JavaType,
    resolver: &impl JavaResolver,
) -> Option<CategoryOneCoercion> {
    category_one_expression_with_parameters(ir, value, actual, expected, resolver, &|_| false)
}

/// Converts attested Boolean incoming values while retaining the original SSA
/// identity. The caller must attest an exact signature-bound initial value.
pub(crate) fn category_one_expression_with_parameters(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    actual: &JavaType,
    expected: &JavaType,
    resolver: &impl JavaResolver,
    parameter: &impl Fn(ValueId) -> bool,
) -> Option<CategoryOneCoercion> {
    if let Some(bit) = category_one_literal(ir, value) {
        return match expected {
            JavaType::Boolean => Some(CategoryOneCoercion::Literal(if bit {
                "true"
            } else {
                "false"
            })),
            JavaType::Int if *actual == JavaType::Boolean => {
                Some(CategoryOneCoercion::Literal(if bit { "1" } else { "0" }))
            }
            _ => None,
        };
    }
    if !boolean_bits_with_parameters(ir, value, resolver, parameter) {
        return None;
    }
    match (actual, expected) {
        (JavaType::Boolean, JavaType::Int) => Some(CategoryOneCoercion::BooleanToInt),
        (JavaType::Int, JavaType::Boolean) => Some(CategoryOneCoercion::IntBitsToBoolean),
        _ => None,
    }
}

#[cfg(test)]
#[path = "contextual_index_tests.rs"]
mod index_tests;

#[cfg(test)]
#[path = "contextual_consumer_candidate_tests.rs"]
mod consumer_candidate_tests;

#[cfg(test)]
#[path = "contextual_direct_consumer_tests.rs"]
mod direct_consumer_tests;

#[cfg(test)]
#[path = "contextual_reference_phi_tests.rs"]
mod reference_phi_tests;

#[cfg(test)]
#[path = "contextual_throw_phi_tests.rs"]
mod contextual_throw_phi_tests;

#[cfg(test)]
#[path = "contextual_array_join_tests.rs"]
mod array_join_tests;

#[cfg(test)]
#[path = "contextual_preserved_copy_tests.rs"]
mod preserved_copy_tests;
