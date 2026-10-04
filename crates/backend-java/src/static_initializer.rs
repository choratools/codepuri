//! Structured lowering of linear, acyclic branching and pretest-loop `<clinit>`.
//!
//! The enclosing class must declare its fields before this block and must not
//! duplicate their initialization in field initializers. An encoded prefix block
//! may precede this block. Field writes require caller confirmation because
//! `JavaField` has no final/accessibility metadata; the dedicated final helper
//! additionally checks once-only unqualified assignment in original order.

use super::*;

/// Class-member requirements for a static initializer.
#[derive(Clone, Debug)]
pub struct StaticInitializerContext {
    /// Exact source name of the enclosing class (must match the method owner).
    pub owner: String,
    /// Fields for which the caller has verified that a class-qualified assignment
    /// is legal here: declared, accessible, static and non-final. Blank finals
    /// are deliberately unsupported; they require dedicated unqualified lowering
    /// and definite-assignment analysis. Entries must use exact resolver metadata.
    pub writable_static_fields: Vec<JavaField>,
}

/// Render a supported void static body as a class-member `static {}`. Branches,
/// Phi joins and canonical pretest loops require exact writable fields, original
/// edge copies and per-normal-path blank-final proof.
///
/// Input must be the original `<clinit>` signature with no parameters, receiver
/// or declared exceptions. The ordinary method emitter remains the validation
/// authority. No method-source text is parsed or stripped: statements are emitted
/// again from the validated IR and the terminal void return is omitted by kind.
/// The returned source must be inserted into `context.owner`, in original class
/// initialization order. This helper does not generate field declarations.
pub fn emit_static_initializer(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &ParameterBindings,
    resolver: &impl JavaResolver,
    context: &StaticInitializerContext,
) -> Result<String, Unsupported> {
    emit_static_initializer_with_finals(ir, method, bindings, resolver, context, &[])
}

/// `blank_finals` must be exact declared fields backed by a whole-input write
/// inventory proving this clinit is their only writer. Each is assigned exactly
/// once here, with an unqualified Java final assignment. No statements move.
pub fn emit_static_initializer_with_finals(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &ParameterBindings,
    resolver: &impl JavaResolver,
    context: &StaticInitializerContext,
    blank_finals: &[FieldRef],
) -> Result<String, Unsupported> {
    emit_static_initializer_body_subset(ir, method, bindings, resolver, context, blank_finals)
}

/// Emit the actual static-initializer body with locally proven blank-final writes.
/// The caller must retain genuine final declarations, reject known conflicting
/// writers, and supply the original encoded prefix without duplicate assignments.
/// This verifies local once-only assignment, not whole-input writer exclusivity
/// or complete enclosing-class initialization. Unknown external effects remain
/// separately unavailable; this result must not certify a complete source class.
pub fn emit_static_initializer_body_subset(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &ParameterBindings,
    resolver: &impl JavaResolver,
    context: &StaticInitializerContext,
    blank_finals: &[FieldRef],
) -> Result<String, Unsupported> {
    emit_static_initializer_body_contract(
        ir,
        method,
        bindings,
        resolver,
        context,
        blank_finals,
        None,
    )
}

/// Retain genuine structured source exception scopes in a static initializer.
/// Final writes must occur outside protected bodies and handlers. The original
/// CFG proves exactly-once normal completion using pre-operation state on
/// exceptional transfers; caught paths never inherit a failed store's commit.
pub fn emit_static_initializer_body_subset_with_exceptions(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &ParameterBindings,
    resolver: &impl JavaResolver,
    context: &StaticInitializerContext,
    blank_finals: &[FieldRef],
    metadata: &[&dyn exceptions::RegionMetadata],
) -> Result<String, Unsupported> {
    if metadata.is_empty() {
        return Err(Unsupported::Metadata(
            "empty static exception scopes".into(),
        ));
    }
    emit_static_initializer_body_contract(
        ir,
        method,
        bindings,
        resolver,
        context,
        blank_finals,
        Some(metadata),
    )
}

fn emit_static_initializer_body_contract(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &ParameterBindings,
    resolver: &impl JavaResolver,
    context: &StaticInitializerContext,
    blank_finals: &[FieldRef],
    metadata: Option<&[&dyn exceptions::RegionMetadata]>,
) -> Result<String, Unsupported> {
    let mut finals = HashMap::new();
    let mut final_names = HashSet::new();
    for reference in blank_finals {
        let f = resolver
            .field(*reference)
            .ok_or_else(|| Unsupported::Metadata("unresolved blank final".into()))?;
        member(&f.name)?;
        if !f.is_static
            || f.owner != context.owner
            || f.ty == JavaType::Void
            || !final_names.insert(f.name.clone())
            || finals.insert(*reference, (f, false)).is_some()
        {
            return Err(Unsupported::Metadata("invalid blank final context".into()));
        }
    }
    if method.name != "<clinit>"
        || !method.is_static
        || method.return_type != JavaType::Void
        || !method.parameters.is_empty()
        || !method.throws.is_empty()
        || bindings.receiver.is_some()
        || !bindings.parameters.is_empty()
        || !bindings.names.is_empty()
        || context.owner != method.owner
    {
        return Err(Unsupported::Metadata(
            "invalid class initializer signature/context".into(),
        ));
    }
    let mut signature = method.clone();
    let branching = ir.cfg().block_ids().any(|b| {
        ir.cfg().block(b).is_ok_and(|node| {
            node.outgoing().len() > 1
                || node.outgoing().iter().any(|id| {
                    ir.cfg()
                        .edge(*id)
                        .is_ok_and(|edge| *edge.kind() != EdgeKind::Unconditional)
                })
        })
    });
    signature.name = "class_initializer_body".into();
    let gate = match metadata {
        Some(scopes) => exceptions::emit_method_disjoint(
            ir,
            &signature,
            &DeclarationBindings::default(),
            resolver,
            scopes,
        ),
        None => emit_method(ir, &signature, bindings, resolver),
    };
    if let SourceResult::Unsupported(reason) = gate {
        return Err(reason);
    }

    // Reserve every qualifier root just as the method emitter does. This is
    // required even for metadata on instructions outside the traversal.
    let mut reserved = HashSet::from(["java".to_string()]);
    reserve_class(&mut reserved, &method.owner);
    // Unqualified assignments must not bind a generated local instead of a field.
    reserved.extend(final_names);
    for (_, value) in ir.values() {
        if let Ok(ty) = managed_type(value.ty(), resolver) {
            reserve_type(&mut reserved, &ty);
        }
    }
    for (_, instruction) in ir.instructions() {
        match instruction {
            ManagedInsn::Invoke { method, .. } => {
                if let Some(target) = resolver.method(*method) {
                    reserve_class(&mut reserved, &target.owner);
                    reserve_type(&mut reserved, &target.return_type);
                    for ty in &target.parameters {
                        reserve_type(&mut reserved, ty);
                    }
                }
            }
            ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                if let Some(target) = resolver.field(*field) {
                    reserve_class(&mut reserved, &target.owner);
                    reserve_type(&mut reserved, &target.ty);
                }
            }
            _ => {}
        }
    }
    let mut emitter = Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir,
        resolver,
        values: HashMap::new(),
        lines: Vec::new(),
        next: 0,
        parameters: HashSet::new(),
        reserved,
        current: None,
        hoisted: HashSet::new(),
        declarations: contextual_locals::declaration_plan(ir, &JavaType::Void, resolver)
            .map_err(|error| Unsupported::Type(format!("phi declaration plan: {error:?}")))?,
    };
    let cfg = ir.cfg();
    if let Some(metadata) = metadata {
        return emit_static_exception_regions(&mut emitter, &signature, &finals, context, metadata);
    }
    if branching {
        let joins = ir
            .instructions()
            .values()
            .any(|i| matches!(i, ManagedInsn::Phi { .. }))
            || cfg.block_ids().any(|block| {
                cfg.block(block).is_ok_and(|node| {
                    node.incoming().len() > 1
                        && !matches!(
                            ir.instruction(*node.payload()),
                            Ok(ManagedInsn::Return { value: None })
                        )
                })
            });
        if joins {
            return emit_final_joins(&mut emitter, &signature, &finals, context);
        }
        let mut seen = HashSet::new();
        emit_final_branch_tree(
            &mut emitter,
            &signature,
            cfg.entry(),
            &mut seen,
            &mut finals,
            context,
            0,
        )?;
        if !seen.iter().any(|block| {
            cfg.block(*block)
                .ok()
                .and_then(|node| ir.instruction(*node.payload()).ok())
                .is_some_and(|insn| matches!(insn, ManagedInsn::Return { value: None }))
        }) {
            return Err(Unsupported::ControlFlow(
                "static initializer cannot complete normally".into(),
            ));
        }
        if seen.len() != cfg.block_count() {
            return Err(Unsupported::ControlFlow(
                "unreachable static initializer blocks".into(),
            ));
        }
        let body = emitter
            .lines
            .iter()
            .map(|line| format!("    {line}\n"))
            .collect::<String>();
        return Ok(format!("static {{\n{body}}}\n"));
    }
    let mut block = cfg.entry();
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(block) {
            return Err(Unsupported::ControlFlow("cyclic static initializer".into()));
        }
        let node = cfg
            .block(block)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
        let id = *node.payload();
        let instruction = ir
            .instruction(id)
            .map_err(|e| Unsupported::Instruction(e.to_string()))?;
        match instruction {
            ManagedInsn::Return { value: None } => {
                if !node.outgoing().is_empty() {
                    return Err(Unsupported::ControlFlow(
                        "static initializer return has successors".into(),
                    ));
                }
                break;
            }
            ManagedInsn::Return { value: Some(_) } => {
                return Err(Unsupported::Type(
                    "class initializer returns a value".into(),
                ));
            }
            ManagedInsn::FieldPut {
                field,
                object: None,
                ..
            } => {
                let target = resolver
                    .field(*field)
                    .ok_or_else(|| Unsupported::Metadata("unresolved field".into()))?;
                if !finals.contains_key(field)
                    && !context.writable_static_fields.iter().any(|allowed| {
                        allowed.is_static
                            && target.is_static
                            && allowed.owner == target.owner
                            && allowed.name == target.name
                            && allowed.ty == target.ty
                    })
                {
                    return Err(Unsupported::Metadata(
                        "static field assignment requires writable member context".into(),
                    ));
                }
            }
            _ => {}
        }
        emitter.current = Some(id);
        match instruction {
            ManagedInsn::FieldPut {
                field,
                object: None,
                value,
            } if finals.contains_key(field) => {
                let (f, assigned) = finals
                    .get_mut(field)
                    .ok_or_else(|| Unsupported::Metadata("blank final disappeared".into()))?;
                if *assigned {
                    return Err(Unsupported::Metadata("blank final assigned twice".into()));
                }
                let (expression, ty) = emitter.operand(*value, &f.ty)?;
                emitter.compatible(&ty, &f.ty)?;
                emitter.lines.push(format!("{} = {expression};", f.name));
                *assigned = true;
            }
            ManagedInsn::FieldGet { field, .. }
                if finals.get(field).is_some_and(|(_, assigned)| !assigned) =>
            {
                return Err(Unsupported::Metadata(
                    "blank final read before assignment".into(),
                ));
            }
            _ => {
                emitter.instruction(instruction, &signature)?;
            }
        }
        // The ordinary method emitter also accepts terminal throws. They cannot
        // continue this linear initializer traversal or establish normal class
        // initialization completion. Return a diagnostic rather than indexing
        // a nonexistent successor.
        let next = node.outgoing().first().ok_or_else(|| {
            Unsupported::ControlFlow("static initializer has no normal return".into())
        })?;
        block = cfg
            .edge(*next)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            .target();
    }
    if seen.len() != cfg.block_count() {
        return Err(Unsupported::ControlFlow(
            "unreachable static initializer blocks".into(),
        ));
    }
    if finals.values().any(|(_, assigned)| !assigned) {
        return Err(Unsupported::Metadata("blank final unassigned".into()));
    }
    let mut source = String::from("static {\n");
    for line in emitter.lines {
        source.push_str("    ");
        source.push_str(&line);
        source.push('\n');
    }
    source.push_str("}\n");
    Ok(source)
}

// These planners check original edge identities, Phi-cluster coverage, source
// availability and single instruction placement. Local final legality remains
// a separate path obligation; no runtime immutability is inferred.
fn emit_final_joins<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    method: &JavaMethod,
    finals: &HashMap<FieldRef, (JavaField, bool)>,
    context: &StaticInitializerContext,
) -> Result<String, Unsupported> {
    prove_final_paths(e.ir, finals, false)?;
    e.declarations = contextual_locals::declaration_plan(e.ir, &JavaType::Void, e.resolver)
        .map_err(|error| Unsupported::Type(format!("phi declaration plan: {error:?}")))?;
    let declared = e.declarations.keys().copied().collect();
    let strict = decomp_structure::structure(e.ir.cfg())
        .map_err(branching::PlanError::Structure)
        .and_then(|regions| branching::plan_checked(e.ir, &regions.root, &[], &declared));
    let (locals, steps, forward) = match strict {
        Ok(plan) => (plan.locals, Some(plan.steps), None),
        Err(original) => {
            let plan = branching::plan_forward_checked(e.ir, &[], &declared).map_err(|error| {
                Unsupported::ControlFlow(format!("static join plan: {original}; {error}"))
            })?;
            (plan.locals.clone(), None, Some(plan))
        }
    };
    for value in locals {
        let ty = e.ty(value)?;
        let name = fresh_static_local(e);
        e.lines.push(format!("{} {name};", ty.source()?));
        e.values.insert(value, (name, ty));
        e.hoisted.insert(value);
    }
    // Original returns may leave only one conditional arm. Omitting such a
    // return would execute the common suffix. A labelled break keeps its edge
    // semantics while remaining legal inside Java's static initializer.
    let label = "staticInitializerBody";
    if let Some(steps) = steps {
        render_static_steps(e, &steps, method, finals, context, label, 0)?;
    } else if let Some(plan) = forward {
        let body = shared_regions::render(
            e.ir.cfg(),
            &plan.regions,
            "staticForward",
            &mut StaticForwardJava {
                emitter: e,
                method,
                finals,
                context,
                label,
                copies: &plan.copies,
            },
        )
        .map_err(Unsupported::ControlFlow)?;
        e.lines.push(body);
    }
    let body = e
        .lines
        .iter()
        .map(|line| format!("        {line}\n"))
        .collect::<String>();
    Ok(format!("static {{\n    {label}: {{\n{body}    }}\n}}\n"))
}

fn fresh_static_local<R: JavaResolver>(e: &mut Emitter<'_, R>) -> String {
    loop {
        let name = format!("v{}", e.next);
        e.next += 1;
        if e.reserved.insert(name.clone()) {
            return name;
        }
    }
}

fn emit_static_exception_regions<'a, R: JavaResolver>(
    e: &mut Emitter<'a, R>,
    method: &'a JavaMethod,
    finals: &HashMap<FieldRef, (JavaField, bool)>,
    context: &StaticInitializerContext,
    metadata: &[&dyn exceptions::RegionMetadata],
) -> Result<String, Unsupported> {
    use exceptions::TypedEmitter;
    prove_final_paths(e.ir, finals, true)?;
    let plan = exceptions::plan_disjoint(e.ir, metadata, &[], e.resolver)
        .map_err(|error| Unsupported::ControlFlow(error.to_string()))?;
    e.declarations.extend(plan.contextual_types.iter().cloned());
    let mut renderer = exception_emitter::ExceptionEmitter {
        emitter: e,
        method,
        caught: Vec::new(),
        protected: false,
    };
    renderer.hoist(&plan.locals)?;
    render_static_exception_steps(&mut renderer, &plan.body, finals, context, false, 0)?;
    let body = renderer
        .emitter
        .lines
        .iter()
        .map(|line| format!("        {line}\n"))
        .collect::<String>();
    Ok(format!(
        "static {{\n    staticInitializerBody: {{\n{body}    }}\n}}\n"
    ))
}

// Source Java's checked-catch reachability is distinct from runtime dispatch.
// Admit unchecked/broad catches or an actual declared/explicit checked throw;
// never add a dummy throwable operation merely to make a catch compile.
fn static_catch_reachable<R: JavaResolver>(
    ir: &MethodIr<SsaForm>,
    body: &[exceptions::RegionStep],
    caught: &JavaType,
    resolver: &R,
) -> bool {
    if ["java.lang.RuntimeException", "java.lang.Error"]
        .iter()
        .any(|name| {
            let ty = JavaType::Class((*name).into());
            resolver.assignable(caught, &ty) || resolver.assignable(&ty, caught)
        })
    {
        return true;
    }
    body.iter().any(|step| match step {
        exceptions::RegionStep::Instruction(id) => match ir.instruction(*id) {
            Ok(ManagedInsn::Invoke { method, .. }) => resolver.method(*method).is_some_and(|m| {
                m.throws.iter().any(|name| {
                    let ty = JavaType::Class(name.clone());
                    resolver.assignable(&ty, caught) || resolver.assignable(caught, &ty)
                })
            }),
            Ok(ManagedInsn::Throw { value }) => ir
                .value(*value)
                .ok()
                .and_then(|v| managed_type(v.ty(), resolver).ok())
                .is_some_and(|ty| {
                    resolver.assignable(&ty, caught) || resolver.assignable(caught, &ty)
                }),
            _ => false,
        },
        exceptions::RegionStep::If {
            then_body,
            else_body,
            ..
        } => {
            static_catch_reachable(ir, then_body, caught, resolver)
                || static_catch_reachable(ir, else_body, caught, resolver)
        }
        _ => false,
    })
}

fn render_static_exception_steps<R: JavaResolver>(
    native: &mut exception_emitter::ExceptionEmitter<'_, '_, R>,
    steps: &[exceptions::RegionStep],
    finals: &HashMap<FieldRef, (JavaField, bool)>,
    context: &StaticInitializerContext,
    in_region: bool,
    depth: usize,
) -> Result<(), Unsupported> {
    use exceptions::{RegionStep, TypedEmitter};
    for step in steps {
        let start = native.emitter.lines.len();
        match step {
            RegionStep::Instruction(id) => {
                let instruction = native
                    .emitter
                    .ir
                    .instruction(*id)
                    .map_err(|error| Unsupported::Instruction(error.to_string()))?;
                match instruction {
                    ManagedInsn::FieldPut { field, .. } if finals.contains_key(field) => {
                        if in_region {
                            return Err(Unsupported::Metadata(
                                "blank final store inside exception region".into(),
                            ));
                        }
                        static_instruction(
                            native.emitter,
                            *id,
                            native.method,
                            finals,
                            context,
                            "staticInitializerBody",
                        )?;
                    }
                    ManagedInsn::FieldPut { .. } | ManagedInsn::Return { .. } => {
                        static_instruction(
                            native.emitter,
                            *id,
                            native.method,
                            finals,
                            context,
                            "staticInitializerBody",
                        )?;
                    }
                    ManagedInsn::Const {
                        result,
                        value: Constant::Zero | Constant::Null,
                    } if native
                        .emitter
                        .declarations
                        .get(result)
                        .is_some_and(JavaType::reference) =>
                    {
                        let (name, _) = native.emitter.get(*result)?;
                        native.emitter.lines.push(format!("{name} = null;"));
                    }
                    ManagedInsn::Throw { .. } => {
                        let mut signature = native.method.clone();
                        if native.protected {
                            signature.throws.extend(native.caught.iter().filter_map(
                                |ty| match ty {
                                    JavaType::Class(name) => Some(name.clone()),
                                    _ => None,
                                },
                            ));
                        }
                        native.emitter.current = Some(*id);
                        native.emitter.instruction(instruction, &signature)?;
                    }
                    _ => native.instruction(*id)?,
                }
            }
            RegionStep::Copies(copies) => native.copies(copies)?,
            RegionStep::If {
                condition,
                then_body,
                else_body,
            } => {
                let (condition, ty) = native.emitter.get(*condition)?;
                compatible(&ty, &JavaType::Boolean)?;
                let indent = "    ".repeat(depth);
                native
                    .emitter
                    .lines
                    .push(format!("{indent}if ({condition}) {{"));
                render_static_exception_steps(
                    native,
                    then_body,
                    finals,
                    context,
                    in_region,
                    depth + 1,
                )?;
                native.emitter.lines.push(format!("{indent}}} else {{"));
                render_static_exception_steps(
                    native,
                    else_body,
                    finals,
                    context,
                    in_region,
                    depth + 1,
                )?;
                native.emitter.lines.push(format!("{indent}}}"));
                continue;
            }
            RegionStep::Try { body, handlers } => {
                if in_region || handlers.is_empty() {
                    return Err(Unsupported::ControlFlow(
                        "nested or empty static exception region".into(),
                    ));
                }
                for handler in handlers {
                    if !static_catch_reachable(
                        native.emitter.ir,
                        body,
                        &handler.ty,
                        native.emitter.resolver,
                    ) {
                        return Err(Unsupported::Metadata(
                            "static checked catch lacks source throw evidence".into(),
                        ));
                    }
                }
                let indent = "    ".repeat(depth);
                native.caught = handlers.iter().map(|handler| handler.ty.clone()).collect();
                native.protected = true;
                native.emitter.lines.push(format!("{indent}try {{"));
                render_static_exception_steps(native, body, finals, context, true, depth + 1)?;
                native.protected = false;
                for handler in handlers {
                    let caught = fresh_static_local(native.emitter);
                    native.emitter.lines.push(format!(
                        "{indent}}} catch ({} {caught}) {{",
                        handler.ty.source()?
                    ));
                    if let Some(value) = handler.exception {
                        let (slot, ty) = native.emitter.get(value)?;
                        native.emitter.compatible(&handler.ty, &ty)?;
                        native
                            .emitter
                            .lines
                            .push(format!("{indent}    {slot} = {caught};"));
                    }
                    render_static_exception_steps(
                        native,
                        &handler.body,
                        finals,
                        context,
                        true,
                        depth + 1,
                    )?;
                }
                native.caught.clear();
                native.emitter.lines.push(format!("{indent}}}"));
                continue;
            }
            _ => {
                return Err(Unsupported::ControlFlow(
                    "unsupported static exception cleanup/shared control".into(),
                ));
            }
        }
        for line in &mut native.emitter.lines[start..] {
            line.insert_str(0, &"    ".repeat(depth));
        }
    }
    Ok(())
}

fn prove_final_paths(
    ir: &MethodIr<SsaForm>,
    finals: &HashMap<FieldRef, (JavaField, bool)>,
    exceptional: bool,
) -> Result<(), Unsupported> {
    let fields = finals.keys().copied().collect::<Vec<_>>();
    let mut pending = vec![(ir.cfg().entry(), vec![false; fields.len()])];
    let mut seen = HashSet::new();
    let mut normal = false;
    while let Some((block, mut assigned)) = pending.pop() {
        if !seen.insert((block, assigned.clone())) {
            continue;
        }
        if seen.len() > 65_536 {
            return Err(Unsupported::ControlFlow("static final path budget".into()));
        }
        let node = ir
            .cfg()
            .block(block)
            .map_err(|error| Unsupported::ControlFlow(error.to_string()))?;
        let before = assigned.clone();
        match ir
            .instruction(*node.payload())
            .map_err(|error| Unsupported::Instruction(error.to_string()))?
        {
            ManagedInsn::FieldPut { field, object, .. } if fields.contains(field) => {
                let position = fields
                    .iter()
                    .position(|f| f == field)
                    .ok_or_else(|| Unsupported::Metadata("blank final disappeared".into()))?;
                if object.is_some() || assigned[position] {
                    return Err(Unsupported::Metadata(
                        "blank final assigned twice on path".into(),
                    ));
                }
                assigned[position] = true;
            }
            ManagedInsn::FieldGet { field, .. }
                if fields
                    .iter()
                    .position(|f| f == field)
                    .is_some_and(|position| !assigned[position]) =>
            {
                return Err(Unsupported::Metadata(
                    "blank final read before assignment".into(),
                ));
            }
            ManagedInsn::Return { value: None } => {
                if !node.outgoing().is_empty() || assigned.iter().any(|a| !a) {
                    return Err(Unsupported::Metadata(
                        "blank final missing on normal path".into(),
                    ));
                }
                normal = true;
                continue;
            }
            ManagedInsn::Throw { .. } if node.outgoing().is_empty() => continue,
            ManagedInsn::Return { .. } => {
                return Err(Unsupported::Type(
                    "class initializer returns a value".into(),
                ));
            }
            _ => {}
        }
        if node.outgoing().is_empty() {
            return Err(Unsupported::ControlFlow(
                "static initializer has no normal return".into(),
            ));
        }
        for edge in node.outgoing() {
            let edge = ir
                .cfg()
                .edge(*edge)
                .map_err(|error| Unsupported::ControlFlow(error.to_string()))?;
            if matches!(edge.kind(), EdgeKind::Exception(_)) {
                if !exceptional {
                    return Err(Unsupported::ControlFlow("exception edges".into()));
                }
                pending.push((edge.target(), before.clone()));
            } else {
                pending.push((edge.target(), assigned.clone()));
            }
        }
    }
    if normal {
        Ok(())
    } else {
        Err(Unsupported::ControlFlow(
            "static initializer cannot complete normally".into(),
        ))
    }
}

fn static_instruction<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    id: InsnId,
    method: &JavaMethod,
    finals: &HashMap<FieldRef, (JavaField, bool)>,
    context: &StaticInitializerContext,
    label: &str,
) -> Result<(), Unsupported> {
    e.current = Some(id);
    let instruction =
        e.ir.instruction(id)
            .map_err(|error| Unsupported::Instruction(error.to_string()))?;
    match instruction {
        ManagedInsn::Phi { .. } | ManagedInsn::Branch { .. } | ManagedInsn::Switch { .. } => {}
        ManagedInsn::Return { value: None } => e.lines.push(format!("break {label};")),
        ManagedInsn::FieldPut {
            object: None,
            field,
            value,
        } => {
            let target = e
                .resolver
                .field(*field)
                .ok_or_else(|| Unsupported::Metadata("unresolved field".into()))?;
            if let Some((final_field, _)) = finals.get(field) {
                let (value, ty) = e.operand(*value, &final_field.ty)?;
                e.compatible(&ty, &final_field.ty)?;
                e.lines.push(format!("{} = {value};", final_field.name));
            } else {
                if !context.writable_static_fields.iter().any(|f| {
                    f.is_static
                        && target.is_static
                        && f.owner == target.owner
                        && f.name == target.name
                        && f.ty == target.ty
                }) {
                    return Err(Unsupported::Metadata(
                        "static field assignment requires writable member context".into(),
                    ));
                }
                e.instruction(instruction, method)?;
            }
        }
        _ => {
            e.instruction(instruction, method)?;
        }
    }
    Ok(())
}

fn render_static_steps<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    steps: &[branching::Step],
    method: &JavaMethod,
    finals: &HashMap<FieldRef, (JavaField, bool)>,
    context: &StaticInitializerContext,
    label: &str,
    depth: usize,
) -> Result<(), Unsupported> {
    for step in steps {
        let start = e.lines.len();
        match step {
            branching::Step::Instruction(id) => {
                static_instruction(e, *id, method, finals, context, label)?
            }
            branching::Step::Copies { .. } => {
                render_steps_at(e, std::slice::from_ref(step), method, 0)?
            }
            branching::Step::If {
                condition,
                then_steps,
                else_steps,
            } => {
                let (condition, ty) = e.get(*condition)?;
                compatible(&ty, &JavaType::Boolean)?;
                e.lines
                    .push(format!("{}if ({condition}) {{", "    ".repeat(depth)));
                render_static_steps(e, then_steps, method, finals, context, label, depth + 1)?;
                e.lines.push(format!("{}}} else {{", "    ".repeat(depth)));
                render_static_steps(e, else_steps, method, finals, context, label, depth + 1)?;
                e.lines.push(format!("{}}}", "    ".repeat(depth)));
                continue;
            }
            branching::Step::While {
                evaluation,
                condition,
                body_on_true,
                body_steps,
                exit_steps,
            } => {
                // Hoisted names identify the original condition value. Its
                // evaluation stays inside the loop on every iteration; edge
                // Phi batches remain inside their selected body/exit paths.
                let (condition, ty) = e.get(*condition)?;
                compatible(&ty, &JavaType::Boolean)?;
                let indent = "    ".repeat(depth);
                e.lines.push(format!("{indent}while (true) {{"));
                render_static_steps(e, evaluation, method, finals, context, label, depth + 1)?;
                let stop = if *body_on_true {
                    format!("!{condition}")
                } else {
                    condition
                };
                e.lines.push(format!("{indent}    if ({stop}) {{"));
                render_static_steps(e, exit_steps, method, finals, context, label, depth + 2)?;
                e.lines.push(format!("{indent}        break;"));
                e.lines.push(format!("{indent}    }}"));
                render_static_steps(e, body_steps, method, finals, context, label, depth + 1)?;
                e.lines.push(format!("{indent}}}"));
                continue;
            }
            _ => {
                return Err(Unsupported::ControlFlow(
                    "unsupported static join loop/switch".into(),
                ));
            }
        }
        for line in &mut e.lines[start..] {
            line.insert_str(0, &"    ".repeat(depth));
        }
    }
    Ok(())
}

struct StaticForwardJava<'e, 'a, R> {
    emitter: &'e mut Emitter<'a, R>,
    method: &'e JavaMethod,
    finals: &'e HashMap<FieldRef, (JavaField, bool)>,
    context: &'e StaticInitializerContext,
    label: &'e str,
    copies: &'e HashMap<decomp_core::ids::EdgeId, Vec<branching::Copy>>,
}
impl<R: JavaResolver> StaticForwardJava<'_, '_, R> {
    fn emit(&mut self, block: decomp_core::ids::BlockId, terminal: bool) -> Result<String, String> {
        let id = *self
            .emitter
            .ir
            .cfg()
            .block(block)
            .map_err(|error| error.to_string())?
            .payload();
        let instruction = self
            .emitter
            .ir
            .instruction(id)
            .map_err(|error| error.to_string())?;
        if matches!(
            instruction,
            ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }
        ) != terminal
        {
            return Ok(String::new());
        }
        let start = self.emitter.lines.len();
        static_instruction(
            self.emitter,
            id,
            self.method,
            self.finals,
            self.context,
            self.label,
        )
        .map_err(|error| format!("{error:?}"))?;
        Ok(self
            .emitter
            .lines
            .drain(start..)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}
impl<R: JavaResolver> shared_regions::ForwardEmitter for StaticForwardJava<'_, '_, R> {
    fn instructions(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        self.emit(block, false)
    }
    fn terminal(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        self.emit(block, true)
    }
    fn condition(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        let node = self
            .emitter
            .ir
            .cfg()
            .block(block)
            .map_err(|error| error.to_string())?;
        let ManagedInsn::Branch {
            condition: Some(value),
            ..
        } = self
            .emitter
            .ir
            .instruction(*node.payload())
            .map_err(|error| error.to_string())?
        else {
            return Err("static branch condition".into());
        };
        let (name, ty) = self
            .emitter
            .get(*value)
            .map_err(|error| format!("{error:?}"))?;
        compatible(&ty, &JavaType::Boolean).map_err(|error| format!("{error:?}"))?;
        Ok(name)
    }
    fn selector(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        let node = self
            .emitter
            .ir
            .cfg()
            .block(block)
            .map_err(|error| error.to_string())?;
        let ManagedInsn::Switch { key, .. } = self
            .emitter
            .ir
            .instruction(*node.payload())
            .map_err(|error| error.to_string())?
        else {
            return Err("static switch selector".into());
        };
        let (name, ty) = self
            .emitter
            .get(*key)
            .map_err(|error| format!("{error:?}"))?;
        compatible(&ty, &JavaType::Int).map_err(|error| format!("{error:?}"))?;
        Ok(name)
    }
    fn copies(&mut self, edge: decomp_core::ids::EdgeId) -> Result<String, String> {
        let start = self.emitter.lines.len();
        render_steps_at(
            self.emitter,
            &[branching::Step::Copies {
                edge,
                copies: self.copies.get(&edge).cloned().unwrap_or_default(),
            }],
            self.method,
            0,
        )
        .map_err(|error| format!("{error:?}"))?;
        Ok(self
            .emitter
            .lines
            .drain(start..)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn emit_final_branch_tree<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    block: decomp_core::ids::BlockId,
    seen: &mut HashSet<decomp_core::ids::BlockId>,
    finals: &mut HashMap<FieldRef, (JavaField, bool)>,
    context: &StaticInitializerContext,
    depth: usize,
) -> Result<(), Unsupported> {
    let fail = || Unsupported::ControlFlow("unsupported static initializer branch tree".into());
    if depth > 128 {
        return Err(fail());
    }
    let node = e.ir.cfg().block(block).map_err(|_| fail())?;
    let id = *node.payload();
    let instruction = e.ir.instruction(id).map_err(|_| fail())?;
    if !seen.insert(block) && !matches!(instruction, ManagedInsn::Return { value: None }) {
        return Err(fail());
    }
    e.current = Some(id);
    if let ManagedInsn::Return { value: None } = instruction {
        if !node.outgoing().is_empty() || finals.values().any(|(_, assigned)| !assigned) {
            return Err(Unsupported::Metadata(
                "blank final missing on normal path".into(),
            ));
        }
        return Ok(());
    }
    if let ManagedInsn::Branch {
        condition: Some(condition),
        then_edge,
        else_edge: Some(else_edge),
    } = instruction
    {
        if node.outgoing().len() != 2 {
            return Err(fail());
        }
        let yes = e.ir.cfg().edge(*then_edge).map_err(|_| fail())?;
        let no = e.ir.cfg().edge(*else_edge).map_err(|_| fail())?;
        if !node.outgoing().contains(then_edge)
            || !node.outgoing().contains(else_edge)
            || *yes.kind() != EdgeKind::BranchTrue
            || *no.kind() != EdgeKind::BranchFalse
        {
            return Err(fail());
        }
        let (condition, ty) = e.get(*condition)?;
        compatible(&ty, &JavaType::Boolean)?;
        let values = e.values.clone();
        let initial = finals.clone();
        e.lines.push(format!("if ({condition}) {{"));
        emit_final_branch_tree(e, m, yes.target(), seen, finals, context, depth + 1)?;
        e.values = values.clone();
        *finals = initial;
        e.lines.push("} else {".into());
        emit_final_branch_tree(e, m, no.target(), seen, finals, context, depth + 1)?;
        e.values = values;
        e.lines.push("}".into());
        return Ok(());
    }
    if matches!(
        instruction,
        ManagedInsn::Phi { .. } | ManagedInsn::Branch { .. }
    ) {
        return Err(fail());
    }
    match instruction {
        ManagedInsn::FieldPut {
            field,
            object: None,
            value,
        } if finals.contains_key(field) => {
            let (mapped, assigned) = finals.get_mut(field).ok_or_else(fail)?;
            if *assigned {
                return Err(Unsupported::Metadata(
                    "blank final assigned twice on path".into(),
                ));
            }
            let (expression, ty) = e.operand(*value, &mapped.ty)?;
            e.compatible(&ty, &mapped.ty)?;
            e.lines.push(format!("{} = {expression};", mapped.name));
            *assigned = true;
        }
        ManagedInsn::FieldGet { field, .. }
            if finals.get(field).is_some_and(|(_, assigned)| !assigned) =>
        {
            return Err(Unsupported::Metadata(
                "blank final read before assignment".into(),
            ));
        }
        ManagedInsn::FieldPut {
            field,
            object: None,
            ..
        } => {
            let target = e.resolver.field(*field).ok_or_else(fail)?;
            if !context.writable_static_fields.iter().any(|f| {
                f.is_static
                    && target.is_static
                    && f.owner == target.owner
                    && f.name == target.name
                    && f.ty == target.ty
            }) {
                return Err(Unsupported::Metadata(
                    "static field assignment requires writable member context".into(),
                ));
            }
            e.instruction(instruction, m)?;
        }
        _ => {
            if e.instruction(instruction, m)? {
                return if node.outgoing().is_empty() {
                    Ok(())
                } else {
                    Err(fail())
                };
            }
        }
    }
    if node.outgoing().len() != 1 {
        return Err(fail());
    }
    let edge = e.ir.cfg().edge(node.outgoing()[0]).map_err(|_| fail())?;
    if *edge.kind() != EdgeKind::Unconditional {
        return Err(fail());
    }
    emit_final_branch_tree(e, m, edge.target(), seen, finals, context, depth + 1)
}

#[cfg(test)]
#[path = "static_initializer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "static_initializer_join_tests.rs"]
mod join_tests;

#[cfg(test)]
#[path = "static_initializer_loop_tests.rs"]
mod loop_tests;

#[cfg(test)]
#[path = "static_initializer_exception_tests.rs"]
mod exception_tests;
