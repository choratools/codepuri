//! Edge-selected immutable primary cleanup, including swallowed cleanup errors.
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
        .ok_or(Error::Unsupported("shared cleanup state absent"))
}

fn aliases_after(plan: &Plan, edge: EdgeId, aliases: &HashSet<ValueId>) -> HashSet<ValueId> {
    let mut next = aliases.clone();
    for copy in plan.copies.get(&edge).into_iter().flatten() {
        if aliases.contains(&copy.source) {
            next.insert(copy.destination);
        } else {
            next.remove(&copy.destination);
        }
    }
    next
}

fn primary_paths(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
    entry: BlockId,
    nodes: &HashSet<BlockId>,
) -> Result<bool, Error> {
    let capture = plan.captures[&entry];
    let mut todo = vec![(entry, HashSet::from([capture]))];
    let mut budget = 1024usize;
    while let Some((block, mut aliases)) = todo.pop() {
        if budget == 0 || !nodes.contains(&block) {
            return Ok(false);
        }
        budget -= 1;
        let id = *ir.cfg().block(block).map_err(invalid)?.payload();
        match ir.instruction(id).map_err(invalid)? {
            ManagedInsn::Move { result, value } => {
                if aliases.contains(value) {
                    aliases.insert(*result);
                } else {
                    aliases.remove(result);
                }
            }
            ManagedInsn::Opaque { writes, .. } => {
                for value in writes {
                    if *value != capture {
                        aliases.remove(value);
                    }
                }
            }
            ManagedInsn::Throw { value } => {
                if !aliases.contains(value) {
                    return Ok(false);
                }
                continue;
            }
            _ => {}
        }
        let s = state(plan, block)?;
        if s.normal.is_empty() {
            return Ok(false);
        }
        for &edge in s.normal.iter().chain(&s.exceptional) {
            let target = ir.cfg().edge(edge).map_err(invalid)?.target();
            todo.push((target, aliases_after(plan, edge, &aliases)));
        }
    }
    Ok(true)
}

pub(super) fn prove(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
    resolver: &impl JavaResolver,
) -> Result<HashMap<BlockId, HashSet<BlockId>>, Error> {
    let mut candidates = HashMap::new();
    let mut nested_entries = HashSet::new();
    for &entry in plan.captures.keys() {
        if plan.monitors.owners[&entry].is_some() {
            continue;
        }
        let mut todo = vec![entry];
        let mut nodes = HashSet::new();
        let mut nested = false;
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
            let i = ir.instruction(id).map_err(invalid)?;
            match i {
                ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    reads,
                    writes,
                    ..
                } if reads.is_empty()
                    && writes.len() == 1
                    && plan.capture_blocks.contains(&block) => {}
                ManagedInsn::Move { .. }
                | ManagedInsn::Const { .. }
                | ManagedInsn::Compare { .. }
                | ManagedInsn::Phi { .. } => {}
                ManagedInsn::Branch {
                    condition: None,
                    else_edge: None,
                    ..
                }
                | ManagedInsn::Branch {
                    condition: Some(_),
                    else_edge: Some(_),
                    ..
                } => {}
                ManagedInsn::Invoke { .. } if source::candidate(i, resolver) => {}
                ManagedInsn::Throw { .. } if s.normal.is_empty() && s.exceptional.is_empty() => {
                    continue;
                }
                _ => {
                    valid = false;
                    break;
                }
            }
            if s.normal.is_empty() {
                valid = false;
                break;
            }
            nested |= !s.exceptional.is_empty();
            for &edge in s.normal.iter().chain(&s.exceptional) {
                todo.push(ir.cfg().edge(edge).map_err(invalid)?.target());
            }
        }
        if valid && primary_paths(ir, plan, entry, &nodes)? {
            if nested {
                nested_entries.insert(entry);
            }
            candidates.insert(entry, nodes);
        }
    }
    // Shared nodes may be entered from another proved primary suffix. Ordinary
    // body entries and unrelated handler entries cannot observe an elided node.
    loop {
        let union: HashSet<_> = candidates.values().flatten().copied().collect();
        let mut bad = HashSet::new();
        for (&entry, nodes) in &candidates {
            // A nonnested handler belongs here only when another independently
            // proved primary path shares its suffix. Single handlers retain the
            // existing linear cleanup admission and rendering path.
            if !nested_entries.contains(&entry)
                && !candidates
                    .iter()
                    .any(|(&other, other_nodes)| other != entry && !nodes.is_disjoint(other_nodes))
            {
                bad.insert(entry);
                continue;
            }
            let mut degrees = HashMap::new();
            let mut valid = true;
            for &block in nodes {
                let mut n = 0;
                for &edge in ir.cfg().block(block).map_err(invalid)?.incoming() {
                    let edge = ir.cfg().edge(edge).map_err(invalid)?;
                    if nodes.contains(&edge.source()) {
                        n += 1;
                    } else if !(union.contains(&edge.source())
                        || block == entry && matches!(edge.kind(), EdgeKind::Exception(_)))
                    {
                        valid = false;
                    }
                }
                degrees.insert(block, n);
            }
            let mut queue: VecDeque<_> = degrees
                .iter()
                .filter_map(|(&b, &n)| (n == 0).then_some(b))
                .collect();
            let mut count = 0;
            while let Some(b) = queue.pop_front() {
                count += 1;
                for &edge in state(plan, b)?
                    .normal
                    .iter()
                    .chain(&state(plan, b)?.exceptional)
                {
                    let target = ir.cfg().edge(edge).map_err(invalid)?.target();
                    let Some(n) = degrees.get_mut(&target) else {
                        valid = false;
                        break;
                    };
                    *n -= 1;
                    if *n == 0 {
                        queue.push_back(target);
                    }
                }
            }
            if !valid || count != nodes.len() {
                bad.insert(entry);
            }
        }
        if bad.is_empty() {
            return Ok(candidates);
        }
        candidates.retain(|entry, _| !bad.contains(entry));
        if candidates.is_empty() {
            return Ok(candidates);
        }
    }
}

#[derive(Clone, Default)]
struct Identity {
    names: HashMap<ValueId, String>,
    domains: HashMap<ValueId, Vec<String>>,
}

fn domain_for(
    domain: &[String],
    caught: &JavaType,
    excluded: &[JavaType],
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> Result<Vec<String>, Unsupported> {
    let mut out = Vec::new();
    for name in domain {
        let ty = JavaType::Class(name.clone());
        if excluded
            .iter()
            .any(|e| hierarchy.assignable(&ty, e) == Compatibility::Assignable)
        {
            continue;
        }
        match (
            hierarchy.assignable(&ty, caught),
            hierarchy.assignable(caught, &ty),
        ) {
            (Compatibility::Assignable, _) => out.push(name.clone()),
            (_, Compatibility::Assignable) => {
                let JavaType::Class(n) = caught else {
                    return Err(Unsupported::Type(
                        "shared cleanup nonreference catch".into(),
                    ));
                };
                out.push(n.clone());
            }
            (Compatibility::Incompatible, Compatibility::Incompatible) => {}
            _ => {
                return Err(Unsupported::Type(
                    "shared cleanup checked-domain overlap unavailable".into(),
                ));
            }
        }
    }
    Ok(out)
}

// These enumerated primitive source forms have no checked declaration. The
// exact original instruction must already have passed the ordinary typed
// emitter at this site; unresolved calls, fields and opaque operations cannot
// obtain a contract through this path.
fn origin_contract<R: JavaResolver>(
    emitter: &crate::Emitter<'_, R>,
    caller: &str,
    id: InsnId,
    insn: &ManagedInsn,
) -> Option<Vec<String>> {
    if let Some(contract) = source::emitted_contract(emitter, caller, id, insn) {
        return Some(contract);
    }
    if emitter.current != Some(id) || emitter.ir.instruction(id).ok()? != insn {
        return None;
    }
    let integral = |v| {
        emitter.get(v).is_ok_and(|(_, t)| {
            matches!(
                t,
                JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int
            )
        })
    };
    let valid = match insn {
        ManagedInsn::ArrayGet {
            result,
            array,
            index,
        } => {
            matches!(emitter.get(*array).ok()?.1, JavaType::Array(_))
                && integral(*index)
                && emitter.get(*result).is_ok()
        }
        ManagedInsn::ArrayLength { result, array } => {
            matches!(emitter.get(*array).ok()?.1, JavaType::Array(_))
                && emitter.get(*result).is_ok_and(|(_, t)| t == JavaType::Int)
        }
        ManagedInsn::NewArray { result, length, .. } => {
            integral(*length) && matches!(emitter.get(*result).ok()?.1, JavaType::Array(_))
        }
        ManagedInsn::Cast {
            result,
            value,
            target,
        } => {
            emitter.get(*value).is_ok()
                && emitter.resolver.instruction_type(*target).is_some()
                && emitter.get(*result).is_ok()
        }
        _ => false,
    };
    valid.then(Vec::new)
}

pub(crate) fn render<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    entry: BlockId,
    caught: &str,
    origin: (InsnId, &ManagedInsn, Option<&DeferredAllocation>),
) -> Result<(), Unsupported> {
    let (original_id, original, deferred) = origin;
    let nodes = plan
        .rethrows
        .shared_cleanups
        .get(&entry)
        .ok_or_else(|| Unsupported::ControlFlow("shared cleanup proof absent".into()))?;
    let block = native
        .emitter
        .ir
        .cfg()
        .block_ids()
        .find(|&b| {
            native
                .emitter
                .ir
                .cfg()
                .block(b)
                .is_ok_and(|b| *b.payload() == original_id)
        })
        .ok_or_else(|| Unsupported::ControlFlow("shared cleanup source identity absent".into()))?;
    let scope = plan.regions.coverage[&block];
    let edges = &plan.regions.handlers[&block];
    let index = edges
        .iter()
        .position(|&e| {
            native
                .emitter
                .ir
                .cfg()
                .edge(e)
                .is_ok_and(|e| e.target() == entry)
        })
        .ok_or_else(|| Unsupported::ControlFlow("shared cleanup original edge absent".into()))?;
    let contract = origin_contract(native.emitter, &native.method.owner, original_id, original)
        .or_else(|| {
            deferred
                .filter(|p| p.matches(original_id, original))
                .map(|_| vec![])
        })
        .ok_or_else(|| {
            Unsupported::Metadata(format!(
                "shared cleanup root contract unavailable: {original_id:?} {original:?}"
            ))
        })?;
    let domain = domain_for(
        &contract,
        &plan.types[scope][index],
        &plan.types[scope][..index],
        metadata[scope].hierarchy(),
    )?;
    let capture = plan.captures[&entry];
    let identity = Identity {
        names: HashMap::from([(capture, caught.to_owned())]),
        domains: HashMap::from([(capture, domain)]),
    };
    let saved = native.emitter.values.clone();
    let saved_hoisted = native.emitter.hoisted.clone();
    let saved_current = native.emitter.current;
    let mut budget = 2048;
    let result = Renderer {
        metadata,
        plan,
        nodes,
        budget: &mut budget,
    }
    .block(native, entry, &identity);
    native.emitter.values = saved;
    native.emitter.hoisted = saved_hoisted;
    native.emitter.current = saved_current;
    result
}

struct Renderer<'a, 'm> {
    metadata: &'a [&'m dyn RegionMetadata],
    plan: &'a Plan,
    nodes: &'a HashSet<BlockId>,
    budget: &'a mut usize,
}
impl Renderer<'_, '_> {
    fn transfer<R: JavaResolver>(
        &mut self,
        native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
        edge: EdgeId,
        identity: &Identity,
    ) -> Result<(), Unsupported> {
        let mut next = identity.clone();
        let mut copies = Vec::new();
        for copy in self.plan.copies.get(&edge).into_iter().flatten() {
            if let Some(name) = identity.names.get(&copy.source) {
                let (_, ty) = native.emitter.get(copy.source)?;
                let (_, expected) = native.emitter.get(copy.destination)?;
                if ty != expected {
                    return Err(Unsupported::Type(
                        "shared cleanup primary edge narrows type".into(),
                    ));
                }
                next.names.insert(copy.destination, name.clone());
                next.domains.insert(
                    copy.destination,
                    identity.domains.get(&copy.source).cloned().ok_or_else(|| {
                        Unsupported::Metadata("shared cleanup primary edge domain absent".into())
                    })?,
                );
            } else {
                next.names.remove(&copy.destination);
                next.domains.remove(&copy.destination);
                copies.push(copy.clone());
            }
        }
        // Existing copy lowering proves contextual operand types and snapshots every
        // source before assigning any destination. Immutable primary bindings are
        // updated only after those snapshots, preserving simultaneous edge semantics.
        crate::render_steps(
            native.emitter,
            &[crate::branching::Step::Copies { edge, copies }],
            native.method,
        )?;
        for (&value, name) in &next.names {
            let ty = native.emitter.ty(value)?;
            native.emitter.values.insert(value, (name.clone(), ty));
        }
        let target = native
            .emitter
            .ir
            .cfg()
            .edge(edge)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            .target();
        self.block(native, target, &next)
    }
    fn block<R: JavaResolver>(
        &mut self,
        native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
        block: BlockId,
        identity: &Identity,
    ) -> Result<(), Unsupported> {
        use super::super::TypedEmitter;
        *self.budget = self.budget.checked_sub(1).ok_or_else(|| {
            Unsupported::ControlFlow("shared cleanup lexical budget exceeded".into())
        })?;
        if !self.nodes.contains(&block) {
            return Err(Unsupported::ControlFlow(
                "shared cleanup left original proof".into(),
            ));
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
        let s = state(self.plan, block).map_err(|e| Unsupported::ControlFlow(format!("{e:?}")))?;
        let mut current = identity.clone();
        if let ManagedInsn::Move { result, value } = &insn {
            if let Some(name) = current.names.get(value).cloned() {
                if native.emitter.ty(*result)? != native.emitter.ty(*value)? {
                    return Err(Unsupported::Type(
                        "shared cleanup primary alias narrows type".into(),
                    ));
                }
                current.names.insert(*result, name);
                current.domains.insert(
                    *result,
                    current.domains.get(value).cloned().ok_or_else(|| {
                        Unsupported::Metadata("shared cleanup alias domain absent".into())
                    })?,
                );
            } else {
                current.names.remove(result);
                current.domains.remove(result);
            }
        }
        for (&v, n) in &current.names {
            let ty = native.emitter.ty(v)?;
            native.emitter.values.insert(v, (n.clone(), ty));
        }
        let operation = match &insn {
            ManagedInsn::Throw { value } => {
                current.domains.get(value).cloned().ok_or_else(|| {
                    Unsupported::Metadata("shared cleanup primary source domain absent".into())
                })?
            }
            ManagedInsn::Invoke { .. } => {
                source::emitted_contract(native.emitter, &native.method.owner, id, &insn)
                    .ok_or_else(|| {
                        Unsupported::Metadata(
                            "shared cleanup operation contract unavailable".into(),
                        )
                    })?
            }
            _ => vec![],
        };
        for name in &operation {
            let ty = JavaType::Class(name.clone());
            if !["java.lang.RuntimeException", "java.lang.Error"]
                .into_iter()
                .chain(native.method.throws.iter().map(String::as_str))
                .any(|n| {
                    native
                        .emitter
                        .resolver
                        .assignable(&ty, &JavaType::Class(n.to_string()))
                })
                && !self.plan.regions.coverage.get(&block).is_some_and(|scope| {
                    self.plan.types[*scope]
                        .iter()
                        .any(|t| native.emitter.resolver.assignable(&ty, t))
                })
            {
                return Err(Unsupported::Metadata(
                    "shared cleanup primary checked domain undeclared".into(),
                ));
            }
        }
        let old_caught = native.caught.clone();
        let old_protected = native.protected;
        if !s.exceptional.is_empty() {
            let scope = self.plan.regions.coverage[&block];
            native.caught = self.plan.types[scope].clone();
            native.protected = true;
            native.emitter.lines.push("try {".into());
        }
        match &insn {
            ManagedInsn::Throw { value } => {
                let name = current.names.get(value).ok_or_else(|| {
                    Unsupported::ControlFlow("shared cleanup throw not immutable primary".into())
                })?;
                native.emitter.lines.push(format!("throw {name};"));
            }
            ManagedInsn::Opaque { .. } | ManagedInsn::Phi { .. } | ManagedInsn::Branch { .. } => (),
            ManagedInsn::Move { result, .. } if current.names.contains_key(result) => (),
            _ => native.instruction(id).map_err(|e| {
                Unsupported::ControlFlow(format!("shared cleanup original {id:?} {insn:?}: {e:?}"))
            })?,
        }
        native.caught = old_caught;
        native.protected = old_protected;
        if !s.exceptional.is_empty() {
            let scope = self.plan.regions.coverage[&block];
            let guarded = operation.is_empty();
            let guard = format!("cleanupGuard{}", native.emitter.next);
            native.emitter.next += 1;
            if guarded {
                native
                    .emitter
                    .lines
                    .push(format!("}} catch (java.lang.Throwable {guard}) {{"));
            }
            let mut all = false;
            for (index, &edge) in s.exceptional.iter().enumerate() {
                let ty = &self.plan.types[scope][index];
                let md = self.metadata[scope];
                let catchall = md.is_catch_all(&md.region_scope().catches[index].key);
                if !guarded
                    && !super::super::native_catch_reachable(
                        native.emitter,
                        &native.method.owner,
                        id,
                        &insn,
                        ty,
                        md.hierarchy(),
                    )?
                {
                    return Err(Unsupported::Metadata(
                        "shared cleanup catch source reachability unavailable".into(),
                    ));
                }
                let name = if guarded {
                    if !catchall {
                        return Err(Unsupported::ControlFlow(
                            "shared cleanup guarded narrow binding unavailable".into(),
                        ));
                    }
                    native.emitter.lines.push(if index == 0 {
                        "{".into()
                    } else {
                        "} else {".into()
                    });
                    all = true;
                    guard.clone()
                } else {
                    let n = format!("cleanupCaught{}", native.emitter.next);
                    native.emitter.next += 1;
                    native
                        .emitter
                        .lines
                        .push(format!("}} catch ({} {n}) {{", ty.source()?));
                    n
                };
                let target = ir
                    .cfg()
                    .edge(edge)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                    .target();
                let mut nested = current.clone();
                let saved = native.emitter.values.clone();
                let saved_hoisted = native.emitter.hoisted.clone();
                let saved_current = native.emitter.current;
                if let Some(&capture) = self.plan.captures.get(&target) {
                    nested.names.insert(capture, name.clone());
                    nested.domains.insert(
                        capture,
                        domain_for(
                            &operation,
                            ty,
                            &self.plan.types[scope][..index],
                            md.hierarchy(),
                        )?,
                    );
                    let expected = native.emitter.ty(capture)?;
                    native.emitter.compatible(ty, &expected)?;
                    // The original catch variable may be viewed through a wider
                    // SSA declaration; this is an implicit widening only, never
                    // a cast or a changed exception object.
                    native.emitter.values.insert(capture, (name, expected));
                }
                let outcome = self.transfer(native, edge, &nested);
                native.emitter.values = saved;
                native.emitter.hoisted = saved_hoisted;
                native.emitter.current = saved_current;
                outcome?;
            }
            if guarded {
                native.emitter.lines.push(if all {
                    "}".into()
                } else {
                    format!("}} else {{ throw {guard}; }}")
                });
            }
            native.emitter.lines.push("}".into());
        }
        if matches!(insn, ManagedInsn::Throw { .. }) {
            return Ok(());
        }
        if let ManagedInsn::Branch {
            condition: Some(c),
            then_edge,
            else_edge: Some(no),
        } = insn
        {
            let (expr, ty) = native.emitter.get(c)?;
            native.emitter.compatible(&ty, &JavaType::Boolean)?;
            let saved = native.emitter.values.clone();
            let saved_hoisted = native.emitter.hoisted.clone();
            native.emitter.lines.push(format!("if ({expr}) {{"));
            self.transfer(native, then_edge, &current)?;
            native.emitter.values = saved.clone();
            native.emitter.hoisted = saved_hoisted.clone();
            native.emitter.lines.push("} else {".into());
            self.transfer(native, no, &current)?;
            native.emitter.values = saved;
            native.emitter.hoisted = saved_hoisted;
            native.emitter.lines.push("}".into());
            Ok(())
        } else {
            let [edge] = s.normal.as_slice() else {
                return Err(Unsupported::ControlFlow(
                    "shared cleanup continuation unavailable".into(),
                ));
            };
            self.transfer(native, *edge, &current)
        }
    }
}

#[cfg(test)]
#[path = "exception_dispatch_shared_cleanup_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "exception_dispatch_shared_plain_cleanup_tests.rs"]
mod plain_tests;
