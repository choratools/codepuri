//! Bounded acyclic original suppression handlers, with immutable catch bindings.
use super::*;

fn state(
    plan: &Plan,
    block: BlockId,
) -> Result<&decomp_structure::exception_dispatch::State, Error> {
    plan.regions
        .graph
        .states
        .iter()
        .find(|s| s.block == block)
        .ok_or(Error::Unsupported("suppression state absent"))
}

pub(super) fn prove(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
    resolver: &impl JavaResolver,
) -> Result<HashMap<BlockId, HashSet<BlockId>>, Error> {
    let handler_entries: HashSet<_> = plan
        .regions
        .graph
        .states
        .iter()
        .flat_map(|s| s.exceptional.iter())
        .filter_map(|edge| ir.cfg().edge(*edge).ok().map(|e| e.target()))
        .collect();
    let mut selected = HashMap::new();
    for &entry in plan.captures.keys() {
        let mut todo = vec![entry];
        let mut nodes = HashSet::new();
        let mut suppressed = false;
        let mut valid = true;
        while let Some(block) = todo.pop() {
            if !nodes.insert(block) {
                continue;
            }
            if nodes.len() > 64 || plan.monitors.owners[&block].is_some() {
                valid = false;
                break;
            }
            let s = state(plan, block)?;
            let id = *ir.cfg().block(block).map_err(invalid)?.payload();
            let insn = ir.instruction(id).map_err(invalid)?;
            match insn {
                ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    reads,
                    writes,
                    ..
                } if plan.capture_blocks.contains(&block)
                    && reads.is_empty()
                    && writes.len() == 1 => {}
                ManagedInsn::Move { .. }
                | ManagedInsn::Return { .. }
                | ManagedInsn::Throw { .. } => {}
                ManagedInsn::Branch {
                    condition: None,
                    else_edge: None,
                    ..
                } => {}
                ManagedInsn::Invoke {
                    kind: managed_ir::ops::InvokeKind::Virtual,
                    method,
                    args,
                    result: None,
                } if source::candidate(insn, resolver) => {
                    if let Some(m) = resolver.method(*method)
                        && m.owner == "java.lang.Throwable"
                        && m.name == "addSuppressed"
                        && m.parameters == [JavaType::Class("java.lang.Throwable".into())]
                        && m.return_type == JavaType::Void
                        && m.throws.is_empty()
                        && args.len() == 2
                    {
                        suppressed = true;
                    }
                }
                _ => {
                    valid = false;
                    break;
                }
            }
            for &edge in s.normal.iter().chain(&s.exceptional) {
                if !plan.copies.get(&edge).is_none_or(Vec::is_empty) {
                    valid = false;
                    break;
                }
                todo.push(ir.cfg().edge(edge).map_err(invalid)?.target());
            }
        }
        if !valid || !suppressed {
            continue;
        }
        // No unrelated normal entry, no Phi transfer, and no original cycle.
        let mut degrees = HashMap::new();
        for &block in &nodes {
            let mut degree = 0;
            for &edge in ir.cfg().block(block).map_err(invalid)?.incoming() {
                let edge = ir.cfg().edge(edge).map_err(invalid)?;
                if nodes.contains(&edge.source()) {
                    degree += 1;
                } else if !handler_entries.contains(&block)
                    || !matches!(edge.kind(), EdgeKind::Exception(_))
                {
                    valid = false;
                }
            }
            degrees.insert(block, degree);
        }
        let mut queue: VecDeque<_> = degrees
            .iter()
            .filter_map(|(&b, &n)| (n == 0).then_some(b))
            .collect();
        let mut count = 0;
        while let Some(block) = queue.pop_front() {
            count += 1;
            for &edge in state(plan, block)?
                .normal
                .iter()
                .chain(&state(plan, block)?.exceptional)
            {
                let target = ir.cfg().edge(edge).map_err(invalid)?.target();
                let degree = degrees
                    .get_mut(&target)
                    .ok_or(Error::Unsupported("suppression exits proof"))?;
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(target);
                }
            }
        }
        if valid && count == nodes.len() {
            selected.insert(entry, nodes);
        }
    }
    // Any shared handler that is elided must have its own closed suffix.
    let union: HashSet<_> = selected.values().flatten().copied().collect();
    for &entry in &handler_entries {
        if union.contains(&entry) && !selected.contains_key(&entry) {
            let mut todo = vec![entry];
            let mut nodes = HashSet::new();
            while let Some(b) = todo.pop() {
                if nodes.insert(b) {
                    for &edge in state(plan, b)?
                        .normal
                        .iter()
                        .chain(&state(plan, b)?.exceptional)
                    {
                        todo.push(ir.cfg().edge(edge).map_err(invalid)?.target());
                    }
                }
            }
            if nodes.is_subset(&union) {
                selected.insert(entry, nodes);
            } else {
                return Ok(HashMap::new());
            }
        }
    }
    Ok(selected)
}

fn caught_domain(
    domain: &[String],
    caught: &JavaType,
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> Result<Vec<String>, Unsupported> {
    let mut out = Vec::new();
    for name in domain {
        let ty = JavaType::Class(name.clone());
        match (
            hierarchy.assignable(&ty, caught),
            hierarchy.assignable(caught, &ty),
        ) {
            (Compatibility::Assignable, _) => out.push(name.clone()),
            (_, Compatibility::Assignable) => {
                let JavaType::Class(name) = caught else {
                    return Err(Unsupported::Type("suppression nonreference catch".into()));
                };
                out.push(name.clone());
            }
            (Compatibility::Incompatible, Compatibility::Incompatible) => {}
            _ => {
                return Err(Unsupported::Type(
                    "suppression checked-domain overlap unknown".into(),
                ));
            }
        }
    }
    Ok(out)
}

#[derive(Clone)]
struct CatchIdentity {
    names: HashMap<ValueId, String>,
    domains: HashMap<ValueId, Vec<String>>,
}

pub(crate) fn render<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    entry: BlockId,
    caught: &str,
    original_id: InsnId,
    original: &ManagedInsn,
) -> Result<(), Unsupported> {
    let nodes = plan
        .rethrows
        .suppressions
        .get(&entry)
        .ok_or_else(|| Unsupported::ControlFlow("suppression proof absent".into()))?;
    let domain =
        source::emitted_contract(native.emitter, &native.method.owner, original_id, original)
            .ok_or_else(|| {
                Unsupported::Metadata("suppression root source contract absent".into())
            })?;
    let domain = if let Some(block) = native.emitter.ir.cfg().block_ids().find(|&b| {
        native
            .emitter
            .ir
            .cfg()
            .block(b)
            .is_ok_and(|b| *b.payload() == original_id)
    }) && let Some(&scope) = plan.regions.coverage.get(&block)
        && let Some(edges) = plan.regions.handlers.get(&block)
        && let Some(index) = edges.iter().position(|&edge| {
            native
                .emitter
                .ir
                .cfg()
                .edge(edge)
                .is_ok_and(|e| e.target() == entry)
        }) {
        caught_domain(
            &domain,
            &plan.types[scope][index],
            metadata[scope].hierarchy(),
        )?
    } else {
        return Err(Unsupported::ControlFlow(
            "suppression root handler identity unavailable".into(),
        ));
    };
    let mut domains = HashMap::new();
    let mut bindings = HashMap::new();
    if let Some(&capture) = plan.captures.get(&entry) {
        bindings.insert(capture, caught.to_string());
        domains.insert(capture, domain);
    }
    let saved_values = native.emitter.values.clone();
    let mut budget = 2048;
    let out = render_block(
        native,
        metadata,
        plan,
        nodes,
        entry,
        &CatchIdentity {
            names: bindings,
            domains,
        },
        &mut budget,
    );
    native.emitter.values = saved_values;
    out
}

fn render_block<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    nodes: &HashSet<BlockId>,
    block: BlockId,
    identity: &CatchIdentity,
    budget: &mut usize,
) -> Result<(), Unsupported> {
    use super::super::TypedEmitter;
    *budget = budget.checked_sub(1).ok_or_else(|| {
        Unsupported::ControlFlow("suppression lexical expansion budget exceeded".into())
    })?;
    if !nodes.contains(&block) {
        return Err(Unsupported::ControlFlow("suppression left proof".into()));
    }
    let ir = native.emitter.ir;
    let id = *ir
        .cfg()
        .block(block)
        .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
        .payload();
    let insn = ir
        .instruction(id)
        .map_err(|e| Unsupported::Instruction(e.to_string()))?
        .clone();
    let s = state(plan, block).map_err(|e| Unsupported::ControlFlow(format!("{e:?}")))?;
    let mut bindings = identity.names.clone();
    let mut domains = identity.domains.clone();
    if let ManagedInsn::Move { result, value } = insn
        && let Some(name) = bindings.get(&value).cloned()
    {
        if native.emitter.ty(result)? != native.emitter.ty(value)? {
            return Err(Unsupported::Type(
                "suppression alias narrows immutable catch type".into(),
            ));
        }
        bindings.insert(result, name.clone());
        if let Some(domain) = domains.get(&value).cloned() {
            domains.insert(result, domain);
        }
        let ty = native.emitter.ty(result)?;
        native.emitter.values.insert(result, (name, ty));
    }
    for (&value, name) in &bindings {
        let ty = native.emitter.ty(value)?;
        native.emitter.values.insert(value, (name.clone(), ty));
    }
    let edges = &s.exceptional;
    let operation_domain = match &insn {
        ManagedInsn::Throw { value } => domains.get(value).cloned().ok_or_else(|| {
            Unsupported::Metadata("suppression primary source domain absent".into())
        })?,
        ManagedInsn::Invoke { .. } => {
            source::emitted_contract(native.emitter, &native.method.owner, id, &insn).ok_or_else(
                || Unsupported::Metadata("suppression operation source domain absent".into()),
            )?
        }
        _ => vec![],
    };
    for name in &operation_domain {
        let ty = JavaType::Class(name.clone());
        let admitted = ["java.lang.RuntimeException", "java.lang.Error"]
            .iter()
            .any(|r| {
                native
                    .emitter
                    .resolver
                    .assignable(&ty, &JavaType::Class((*r).into()))
            })
            || native.method.throws.iter().any(|r| {
                native
                    .emitter
                    .resolver
                    .assignable(&ty, &JavaType::Class(r.clone()))
            })
            || plan.regions.coverage.get(&block).is_some_and(|scope| {
                plan.types[*scope]
                    .iter()
                    .any(|r| native.emitter.resolver.assignable(&ty, r))
            });
        if !admitted {
            return Err(Unsupported::Metadata(
                "suppression primary has undeclared checked source contract".into(),
            ));
        }
    }
    let saved_caught = native.caught.clone();
    let saved_protected = native.protected;
    if !edges.is_empty() {
        let scope = plan.regions.coverage[&block];
        native.caught = plan.types[scope].clone();
        native.protected = true;
        native.emitter.lines.push("try {".into());
    }
    match &insn {
        ManagedInsn::Throw { value } => {
            let name = bindings.get(value).ok_or_else(|| {
                Unsupported::ControlFlow("suppression throw is not immutable primary".into())
            })?;
            native.emitter.lines.push(format!("throw {name};"));
        }
        ManagedInsn::Opaque { .. } | ManagedInsn::Branch { .. } => {}
        ManagedInsn::Move { result, .. } if bindings.contains_key(result) => {}
        ManagedInsn::Invoke { .. } => {
            source::emitted_contract(native.emitter, &native.method.owner, id, &insn).ok_or_else(
                || Unsupported::Metadata("suppression unresolved exact source contract".into()),
            )?;
            native.instruction(id)?;
        }
        _ => native.instruction(id)?,
    }
    native.caught = saved_caught;
    native.protected = saved_protected;
    if !edges.is_empty() {
        let scope = plan.regions.coverage[&block];
        let guarded = operation_domain.is_empty();
        let guard_name = format!("suppressionGuard{}", native.emitter.next);
        native.emitter.next += 1;
        if guarded {
            native
                .emitter
                .lines
                .push(format!("}} catch (java.lang.Throwable {guard_name}) {{"));
        }
        let mut all = false;
        for (index, &edge) in edges.iter().enumerate() {
            let ty = &plan.types[scope][index];
            if !guarded
                && !super::super::native_catch_reachable(
                    native.emitter,
                    &native.method.owner,
                    id,
                    &insn,
                    ty,
                    metadata[scope].hierarchy(),
                )?
            {
                return Err(Unsupported::Metadata(
                    "suppression catch lacks source reachability".into(),
                ));
            }
            let target = ir
                .cfg()
                .edge(edge)
                .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                .target();
            let capture = plan.captures.get(&target);
            let is_all =
                metadata[scope].is_catch_all(&metadata[scope].region_scope().catches[index].key);
            let name = if guarded {
                if !is_all && capture.is_some() {
                    return Err(Unsupported::ControlFlow(
                        "suppression guarded narrow capture unavailable".into(),
                    ));
                }
                native.emitter.lines.push(format!(
                    "{}if ({}) {{",
                    if index == 0 { "" } else { "} else " },
                    if is_all {
                        "true".into()
                    } else {
                        format!("{guard_name} instanceof {}", ty.source()?)
                    }
                ));
                all |= is_all;
                guard_name.clone()
            } else {
                let name = format!("suppressedCaught{}", native.emitter.next);
                native.emitter.next += 1;
                native
                    .emitter
                    .lines
                    .push(format!("}} catch ({} {name}) {{", ty.source()?));
                name
            };
            let mut nested = bindings.clone();
            if let Some(capture) = capture {
                nested.insert(*capture, name);
            }
            let domain = caught_domain(&operation_domain, ty, metadata[scope].hierarchy())?;
            let mut nested_domains = domains.clone();
            if let Some(capture) = capture {
                nested_domains.insert(*capture, domain);
            }
            let saved_values = native.emitter.values.clone();
            render_block(
                native,
                metadata,
                plan,
                nodes,
                target,
                &CatchIdentity {
                    names: nested,
                    domains: nested_domains,
                },
                budget,
            )?;
            native.emitter.values = saved_values;
        }
        if guarded {
            native.emitter.lines.push(if all {
                "}".into()
            } else {
                format!("}} else {{ throw {guard_name}; }}")
            });
        }
        native.emitter.lines.push("}".into());
    }
    if matches!(insn, ManagedInsn::Throw { .. } | ManagedInsn::Return { .. }) {
        return Ok(());
    }
    let [edge] = s.normal.as_slice() else {
        return Err(Unsupported::ControlFlow(
            "suppression continuation ambiguous".into(),
        ));
    };
    let next = ir
        .cfg()
        .edge(*edge)
        .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
        .target();
    render_block(
        native,
        metadata,
        plan,
        nodes,
        next,
        &CatchIdentity {
            names: bindings,
            domains,
        },
        budget,
    )
}

#[cfg(test)]
#[path = "exception_dispatch_suppression_tests.rs"]
mod tests;
