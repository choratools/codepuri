//! Exact original caught-object cleanup while retaining Java precise rethrow.
//!
//! Declared throws constrain source typing, not runtime Throwable identities.
//! Proved monitor release and bounded acyclic non-monitor cleanup remain in
//! the original immutable lexical catch. Ordered native catch exclusions
//! constrain checked rethrow; every original cleanup operation is retained.
use super::*;
#[path = "exception_dispatch_shared_cleanup.rs"]
mod shared_cleanup;
#[path = "exception_dispatch_shared_monitor_cleanup.rs"]
mod shared_monitor_cleanup;
#[path = "exception_cleanup_source.rs"]
mod source;
#[path = "exception_dispatch_suppression.rs"]
mod suppression;
pub(super) use shared_cleanup::render as render_shared_cleanup;
pub(super) use source::emitted_contract as source_contract;
pub(super) use suppression::render as render_suppression;

/// Successful typed no-op allocation emission; constructor effects remain owned
/// by the exact paired initializer in the same original exception scope.
pub(super) struct DeferredAllocation {
    original: InsnId,
}
impl DeferredAllocation {
    fn matches(&self, id: InsnId, insn: &ManagedInsn) -> bool {
        self.original == id && matches!(insn, ManagedInsn::NewInstance { .. })
    }
}
pub(super) fn deferred_allocation<R: JavaResolver>(
    emitter: &crate::Emitter<'_, R>,
    plan: &Plan,
    id: InsnId,
    insn: &ManagedInsn,
    fragment_start: usize,
) -> Option<DeferredAllocation> {
    if emitter.current != Some(id)
        || emitter.lines.len() != fragment_start
        || emitter.ir.instruction(id).ok()? != insn
        || !matches!(insn, ManagedInsn::NewInstance { .. })
    {
        return None;
    }
    let allocation = emitter.object_plan(id).ok()?;
    if allocation.allocation != id {
        return None;
    }
    let initializer = emitter.ir.cfg().block_ids().find(|&b| {
        emitter
            .ir
            .cfg()
            .block(b)
            .is_ok_and(|b| *b.payload() == allocation.initializer)
    })?;
    let origin = emitter
        .ir
        .cfg()
        .block_ids()
        .find(|&b| emitter.ir.cfg().block(b).is_ok_and(|b| *b.payload() == id))?;
    let scope = plan.regions.coverage.get(&origin)?;
    if plan.regions.coverage.get(&initializer) != Some(scope) {
        return None;
    }
    let handlers = |block| -> Option<Vec<_>> {
        plan.regions
            .handlers
            .get(&block)?
            .iter()
            .map(|edge| {
                let edge = emitter.ir.cfg().edge(*edge).ok()?;
                Some((edge.kind().clone(), edge.target()))
            })
            .collect()
    };
    if handlers(origin)? != handlers(initializer)? {
        return None;
    }
    Some(DeferredAllocation { original: id })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Proof {
    pub paths: HashMap<BlockId, Vec<BlockId>>,
    pub elided: HashSet<BlockId>,
    pub cleanups: HashMap<BlockId, HashSet<BlockId>>,
    /// Exact checked contracts enclosed by a matching post-release catch table.
    pub outer_monitors: HashMap<BlockId, Vec<JavaType>>,
    pub enclosing: HashMap<BlockId, Vec<JavaType>>,
    pub held_cleanups: HashSet<BlockId>,
    pub suppressions: HashMap<BlockId, HashSet<BlockId>>,
    pub shared_cleanups: HashMap<BlockId, HashSet<BlockId>>,
}

pub(super) fn prove(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
    resolver: &impl JavaResolver,
    metadata: &[&dyn RegionMetadata],
) -> Result<Proof, Error> {
    let suppressions = suppression::prove(ir, plan, resolver)?;
    let reserved: HashSet<_> = suppressions.values().flatten().copied().collect();
    let mut shared_cleanups = shared_cleanup::prove(ir, plan, resolver)?;
    shared_cleanups
        .retain(|entry, nodes| !suppressions.contains_key(entry) && nodes.is_disjoint(&reserved));
    let mut proof = Proof {
        suppressions,
        shared_cleanups,
        ..Proof::default()
    };
    proof
        .elided
        .extend(proof.suppressions.values().flatten().copied());
    proof
        .elided
        .extend(proof.shared_cleanups.values().flatten().copied());
    let mut newly_typed = HashSet::new();
    for (&entry, &capture) in &plan.captures {
        let Some(owner) = plan.monitors.owners[&entry] else {
            let incoming = ir.cfg().block(entry).map_err(invalid)?.incoming();
            let native = !incoming.is_empty()
                && incoming.iter().all(|edge| {
                    let Ok(edge) = ir.cfg().edge(*edge) else {
                        return false;
                    };
                    if !matches!(edge.kind(), EdgeKind::Exception(_)) {
                        return false;
                    }
                    let Ok(block) = ir.cfg().block(edge.source()) else {
                        return false;
                    };
                    ir.instruction(*block.payload())
                        .is_ok_and(|insn| source::candidate(insn, resolver))
                });
            if native && let Some(nodes) = cleanup(ir, plan, entry, capture, resolver, None)? {
                let legacy = incoming.iter().all(|edge| {
                    let Ok(edge) = ir.cfg().edge(*edge) else { return false; };
                    let Ok(block) = ir.cfg().block(edge.source()) else { return false; };
                    plan.contextual.get(&capture) == Some(&JavaType::Class("java.lang.Throwable".into()))
                        || matches!(ir.instruction(*block.payload()), Ok(ManagedInsn::Invoke { method, .. }) if resolver.method(*method).is_some_and(|m| !m.throws.is_empty()))
                });
                if !legacy {
                    newly_typed.insert(entry);
                }
                proof.elided.extend(nodes.iter().copied());
                proof.cleanups.insert(entry, nodes);
            }
            continue;
        };
        let mut block = entry;
        let mut previous = None;
        let mut path = Vec::new();
        let mut aliases = HashSet::from([capture]);
        let mut released = false;
        let mut complete = false;
        let mut seen = HashSet::new();
        while seen.insert(block) {
            if let Some(previous) = previous {
                let exclusive = ir
                    .cfg()
                    .block(block)
                    .map_err(invalid)?
                    .incoming()
                    .iter()
                    .all(|edge| {
                        plan.monitors.impossible.contains(edge)
                            || ir.cfg().edge(*edge).is_ok_and(|edge| {
                                edge.source() == previous
                                    && !matches!(edge.kind(), EdgeKind::Exception(_))
                            })
                    });
                if !exclusive {
                    break;
                }
            }
            let id = *ir.cfg().block(block).map_err(invalid)?.payload();
            let insn = ir.instruction(id).map_err(invalid)?;
            path.push(block);
            match insn {
                ManagedInsn::Phi { result, inputs } => {
                    if inputs.iter().all(|(_, value)| aliases.contains(value)) {
                        aliases.insert(*result);
                    }
                }
                ManagedInsn::Opaque {
                    family_tag: 0x8002, ..
                } if plan.capture_blocks.contains(&block) => {}
                ManagedInsn::Move { result, value } if aliases.contains(value) => {
                    aliases.insert(*result);
                }
                ManagedInsn::Branch {
                    condition: None, ..
                } => {}
                ManagedInsn::MonitorExit { .. }
                    if !released
                        && plan.monitors.owners[&block] == Some(owner)
                        && plan.monitors.releases.contains_key(&block) =>
                {
                    released = true;
                }
                ManagedInsn::Throw { value } if released && aliases.contains(value) => {
                    complete = true;
                    break;
                }
                _ => break,
            }
            let state = plan
                .regions
                .graph
                .states
                .iter()
                .find(|state| state.block == block)
                .ok_or(Error::Unsupported(
                    "cleanup outside original dispatch graph",
                ))?;
            let [edge] = state.normal.as_slice() else {
                break;
            };
            previous = Some(block);
            block = ir.cfg().edge(*edge).map_err(invalid)?.target();
        }
        if complete {
            proof.elided.extend(path.iter().copied());
            proof.paths.insert(entry, path);
        }
    }
    for (entry, path) in shared_monitor_cleanup::prove(ir, plan)? {
        proof.elided.extend(path.iter().copied());
        proof.paths.entry(entry).or_insert(path);
    }
    loop {
        let rejected: Vec<_> = newly_typed
            .iter()
            .copied()
            .filter(|entry| !native_typed_tables(ir, plan, metadata, &proof, *entry))
            .collect();
        if rejected.is_empty() {
            break;
        }
        let mut removed = HashSet::new();
        for entry in rejected {
            newly_typed.remove(&entry);
            if let Some(nodes) = proof.cleanups.remove(&entry) {
                removed.extend(nodes);
            }
        }
        let retained: HashSet<_> = proof
            .paths
            .values()
            .flatten()
            .copied()
            .chain(proof.cleanups.values().flatten().copied())
            .chain(proof.shared_cleanups.values().flatten().copied())
            .chain(proof.suppressions.values().flatten().copied())
            .collect();
        proof
            .elided
            .retain(|node| !removed.contains(node) || retained.contains(node));
    }
    for (&entry, &capture) in &plan.captures {
        if plan.monitors.owners[&entry].is_some()
            && !proof.paths.contains_key(&entry)
            && let Some(nodes) = cleanup(ir, plan, entry, capture, resolver, Some(&proof.paths))?
        {
            proof.elided.extend(nodes.iter().copied());
            proof.cleanups.insert(entry, nodes);
            proof.held_cleanups.insert(entry);
        }
    }
    compose_monitors(ir, plan, resolver, &mut proof)?;
    Ok(proof)
}

/// A lexical acquisition catch may also dispatch escaped body cleanup only
/// when the original post-release throw has the exact same ordered table.
/// This bounded route refuses captured/snapshot transfers: no acquisition-time
/// state may be substituted for an original post-release prethrow state.
fn compose_monitors(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
    resolver: &impl JavaResolver,
    proof: &mut Proof,
) -> Result<(), Error> {
    let table = |block: BlockId| -> Option<Vec<(EdgeKind, BlockId, JavaType)>> {
        let edges = plan.regions.handlers.get(&block)?;
        let scope = *plan.regions.coverage.get(&block)?;
        edges
            .iter()
            .enumerate()
            .map(|(index, edge)| {
                let edge_data = ir.cfg().edge(*edge).ok()?;
                if !plan.copies.get(edge).is_none_or(Vec::is_empty)
                    || plan.captures.contains_key(&edge_data.target())
                {
                    return None;
                }
                Some((
                    edge_data.kind().clone(),
                    edge_data.target(),
                    plan.types[scope][index].clone(),
                ))
            })
            .collect()
    };
    for &owner in plan.monitors.acquisitions.keys() {
        let Some(outer) = table(owner) else {
            continue;
        };
        let outer_types = outer
            .iter()
            .map(|(_, _, ty)| ty.clone())
            .collect::<Vec<_>>();
        let mut entries = HashSet::new();
        let mut checked = Vec::new();
        let mut valid = true;
        for state in &plan.regions.graph.states {
            if plan.monitors.owners[&state.block] != Some(owner)
                || plan.monitors.releases.contains_key(&state.block)
                || proof.elided.contains(&state.block)
            {
                continue;
            }
            let id = *ir.cfg().block(state.block).map_err(invalid)?.payload();
            let insn = ir.instruction(id).map_err(invalid)?;
            if !may_throw(ir, insn) {
                continue;
            }
            let Some(edges) = plan.regions.handlers.get(&state.block) else {
                valid = false;
                break;
            };
            for edge in edges {
                let entry = ir.cfg().edge(*edge).map_err(invalid)?.target();
                let Some(path) = proof.paths.get(&entry) else {
                    valid = false;
                    break;
                };
                let Some(&terminal) = path.last() else {
                    valid = false;
                    break;
                };
                if plan.monitors.owners[&terminal].is_some()
                    || table(terminal).as_ref() != Some(&outer)
                {
                    valid = false;
                    break;
                }
                entries.insert(entry);
            }
            if !valid {
                break;
            }
            if let ManagedInsn::Invoke { method, .. } = insn {
                let Some(target) = resolver.method(*method) else {
                    valid = false;
                    break;
                };
                for name in target.throws {
                    let ty = JavaType::Class(name);
                    if ["java.lang.RuntimeException", "java.lang.Error"]
                        .iter()
                        .any(|root| resolver.assignable(&ty, &JavaType::Class((*root).into())))
                    {
                        continue;
                    }
                    if !outer_types
                        .iter()
                        .any(|caught| resolver.assignable(&ty, caught))
                    {
                        valid = false;
                        break;
                    }
                    if !checked.contains(&ty) {
                        checked.push(ty);
                    }
                }
            } else if matches!(insn, ManagedInsn::Throw { .. }) {
                // Explicit checked throws require a separate source-domain proof.
                valid = false;
            }
            if !valid {
                break;
            }
        }
        if valid && !entries.is_empty() {
            proof.outer_monitors.insert(owner, checked);
            for entry in entries {
                proof.enclosing.insert(entry, outer_types.clone());
            }
        }
    }
    Ok(())
}

fn native_typed_tables(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
    metadata: &[&dyn RegionMetadata],
    proof: &Proof,
    entry: BlockId,
) -> bool {
    let Ok(block) = ir.cfg().block(entry) else {
        return false;
    };
    !block.incoming().is_empty()
        && block.incoming().iter().all(|incoming| {
            let Ok(incoming) = ir.cfg().edge(*incoming) else {
                return false;
            };
            if !matches!(incoming.kind(), EdgeKind::Exception(_)) {
                return false;
            }
            let source = incoming.source();
            // Newly generalized cleanup is native only for operations whose
            // emitted checked contract is consulted by the renderer. Other
            // operations keep their original guarded ordered-handler path.
            let Ok(source_block) = ir.cfg().block(source) else {
                return false;
            };
            if !matches!(
                ir.instruction(*source_block.payload()),
                Ok(ManagedInsn::Invoke { .. }
                    | ManagedInsn::FieldGet { .. }
                    | ManagedInsn::FieldPut { .. })
            ) {
                return false;
            }
            let Some(&scope) = plan.regions.coverage.get(&source) else {
                return false;
            };
            let Some(md) = metadata.get(scope) else {
                return false;
            };
            let Some(edges) = plan.regions.handlers.get(&source) else {
                return false;
            };
            if edges.len() != md.region_scope().catches.len() {
                return false;
            }
            edges.iter().enumerate().all(|(index, edge)| {
                let Ok(edge) = ir.cfg().edge(*edge) else {
                    return false;
                };
                let Some(catch) = md.region_scope().catches.get(index) else {
                    return false;
                };
                if md.is_catch_all(&catch.key)
                    || edge.target() != catch.entry
                    || !proof.cleanups.contains_key(&edge.target())
                {
                    return false;
                }
                let Some(ty) = plan.types.get(scope).and_then(|table| table.get(index)) else {
                    return false;
                };
                ["java.lang.RuntimeException", "java.lang.Error"]
                    .iter()
                    .any(|root| {
                        let unchecked = JavaType::Class((*root).into());
                        md.hierarchy().assignable(ty, &unchecked) == Compatibility::Assignable
                            || md.hierarchy().assignable(&unchecked, ty)
                                == Compatibility::Assignable
                    })
            })
        })
}

/// A bounded, acyclic handler which terminates only by throwing its immutable
/// original caught object. Every original operation and simultaneous edge copy
/// remains inside the same lexical catch, in its original control-flow order.
fn cleanup(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
    entry: BlockId,
    capture: ValueId,
    resolver: &impl JavaResolver,
    monitor_paths: Option<&HashMap<BlockId, Vec<BlockId>>>,
) -> Result<Option<HashSet<BlockId>>, Error> {
    let states: HashMap<_, _> = plan
        .regions
        .graph
        .states
        .iter()
        .map(|s| (s.block, s))
        .collect();
    let owner = plan.monitors.owners[&entry];
    if owner.is_some() && monitor_paths.is_none() {
        return Ok(None);
    }
    let mut todo = vec![entry];
    let mut nodes = HashSet::new();
    let mut branches = 0;
    while let Some(block) = todo.pop() {
        if !nodes.insert(block) {
            continue;
        }
        if nodes.len() > 32 || plan.monitors.owners[&block] != owner {
            return Ok(None);
        }
        let state = states
            .get(&block)
            .ok_or(Error::Unsupported("cleanup lacks original state"))?;
        if !state.exceptional.is_empty() {
            let Some(owner) = owner else {
                return Ok(None);
            };
            let [edge] = state.exceptional.as_slice() else {
                return Ok(None);
            };
            let scope = plan.regions.coverage[&block];
            if plan.types[scope] != [JavaType::Class("java.lang.Throwable".into())]
                || !plan.copies.get(edge).is_none_or(Vec::is_empty)
            {
                return Ok(None);
            }
            let target = ir.cfg().edge(*edge).map_err(invalid)?.target();
            let Some(path) = monitor_paths.and_then(|paths| paths.get(&target)) else {
                return Ok(None);
            };
            let Some(&terminal) = path.last() else {
                return Ok(None);
            };
            if plan.monitors.owners[&target] != Some(owner)
                || plan.monitors.owners[&terminal].is_some()
                || !states[&terminal].exceptional.is_empty()
                || path.iter().any(|b| {
                    states[b]
                        .normal
                        .iter()
                        .any(|edge| !plan.copies.get(edge).is_none_or(Vec::is_empty))
                })
            {
                return Ok(None);
            }
        }
        let id = *ir.cfg().block(block).map_err(invalid)?.payload();
        let insn = ir.instruction(id).map_err(invalid)?;
        // Some DEX tables lexically cover their own handler's nonthrowing
        // capture-copy/goto prefix. Preserve exact edges without widening the
        // throwing scope: only identity aliases and unconditional transfers
        // with no exceptional edge may be moved into this lexical cleanup.
        if owner.is_none()
            && plan.regions.coverage.contains_key(&block)
            && !(state.exceptional.is_empty()
                && matches!(
                    insn,
                    ManagedInsn::Move { .. }
                        | ManagedInsn::Branch {
                            condition: None,
                            else_edge: None,
                            ..
                        }
                ))
        {
            return Ok(None);
        }
        if owner.is_some()
            && matches!(insn, ManagedInsn::Invoke { method, .. } if resolver.method(*method).is_none())
        {
            return Ok(None);
        }
        match insn {
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads,
                writes,
                ..
            } if reads.is_empty()
                && writes == &[capture]
                && plan.capture_blocks.contains(&block) => {}
            ManagedInsn::Phi { .. }
            | ManagedInsn::Move { .. }
            | ManagedInsn::Const { .. }
            | ManagedInsn::Compare { .. }
            | ManagedInsn::Invoke { .. } => {}
            ManagedInsn::FieldGet { .. } | ManagedInsn::FieldPut { .. }
                if source::field(insn, resolver) => {}
            ManagedInsn::Branch {
                condition: Some(_),
                else_edge: Some(_),
                ..
            } => {
                branches += 1;
                if branches > 4 {
                    return Ok(None);
                }
            }
            ManagedInsn::Branch {
                condition: None,
                else_edge: None,
                ..
            } => {}
            ManagedInsn::Throw { .. } if state.normal.is_empty() => continue,
            _ => return Ok(None),
        }
        if state.normal.is_empty() {
            return Ok(None);
        }
        for &edge in &state.normal {
            todo.push(ir.cfg().edge(edge).map_err(invalid)?.target());
        }
    }
    // No ordinary entry can observe the elided nodes, and no cycle may be
    // converted into an unrolled lexical handler. Count exact normal edges.
    let mut degree = HashMap::new();
    for &block in &nodes {
        let mut incoming = 0;
        for &edge in ir.cfg().block(block).map_err(invalid)?.incoming() {
            let edge = ir.cfg().edge(edge).map_err(invalid)?;
            if block == entry && matches!(edge.kind(), EdgeKind::Exception(_)) {
                continue;
            }
            if !nodes.contains(&edge.source()) || matches!(edge.kind(), EdgeKind::Exception(_)) {
                return Ok(None);
            }
            incoming += 1;
        }
        degree.insert(block, incoming);
    }
    let mut queue: VecDeque<_> = degree
        .iter()
        .filter_map(|(&b, &n)| (n == 0).then_some(b))
        .collect();
    let mut count = 0;
    while let Some(block) = queue.pop_front() {
        count += 1;
        for &edge in &states[&block].normal {
            let target = ir.cfg().edge(edge).map_err(invalid)?.target();
            let n = degree
                .get_mut(&target)
                .ok_or(Error::Unsupported("cleanup leaves proof"))?;
            *n -= 1;
            if *n == 0 {
                queue.push_back(target);
            }
        }
    }
    if count != nodes.len() {
        return Ok(None);
    }
    let mut aliases = HashSet::from([capture]);
    loop {
        let before = aliases.len();
        for &block in &nodes {
            let id = *ir.cfg().block(block).map_err(invalid)?.payload();
            match ir.instruction(id).map_err(invalid)? {
                ManagedInsn::Move { result, value } if aliases.contains(value) => {
                    aliases.insert(*result);
                }
                ManagedInsn::Phi { result, inputs }
                    if !inputs.is_empty() && inputs.iter().all(|(_, v)| aliases.contains(v)) =>
                {
                    aliases.insert(*result);
                }
                _ => {}
            }
        }
        if before == aliases.len() {
            break;
        }
    }
    for &block in &nodes {
        if owner.is_none() && plan.regions.coverage.contains_key(&block) {
            let id = *ir.cfg().block(block).map_err(invalid)?.payload();
            if let ManagedInsn::Move { result, value } = ir.instruction(id).map_err(invalid)?
                && (!aliases.contains(result)
                    || !aliases.contains(value)
                    || plan.contextual.get(result) != plan.contextual.get(&capture)
                    || plan.contextual.get(value) != plan.contextual.get(&capture))
            {
                return Ok(None);
            }
        }
    }
    for &block in &nodes {
        let id = *ir.cfg().block(block).map_err(invalid)?.payload();
        if let ManagedInsn::Throw { value } = ir.instruction(id).map_err(invalid)?
            && !aliases.contains(value)
        {
            return Ok(None);
        }
    }
    Ok(Some(nodes))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_cleanup<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    plan: &Plan,
    entry: BlockId,
    original: &ManagedInsn,
    original_id: InsnId,
    ty: &JavaType,
    caught: &str,
    guarded: bool,
    catch_all: bool,
    excluded: &[JavaType],
    hierarchy: &crate::type_compatibility::Hierarchy,
    deferred: Option<&DeferredAllocation>,
) -> Result<(), Unsupported> {
    let proven_unchecked_throw = catch_all
        && excluded.is_empty()
        && original_unchecked_throw_only_catch_all(
            native,
            plan,
            entry,
            original_id,
            original,
            hierarchy,
        );
    let unchecked_original_invoke =
        source::emitted_contract(native.emitter, &native.method.owner, original_id, original)
            .is_some_and(|throws| throws.is_empty())
            || deferred.is_some_and(|proof| proof.matches(original_id, original));
    if guarded
        && !(catch_all
            && (plan.rethrows.held_cleanups.contains(&entry)
                || unchecked_original_invoke
                || proven_unchecked_throw))
    {
        return Err(Unsupported::Metadata(format!(
            "cleanup requires original native ordered catches: original={original_id:?} current={:?} catch_all={catch_all} caught_type={ty:?} source_contract={:?} insn={original:?}",
            native.emitter.current,
            source::emitted_contract(native.emitter, &native.method.owner, original_id, original),
        )));
    }
    let nodes = plan
        .rethrows
        .cleanups
        .get(&entry)
        .ok_or_else(|| Unsupported::ControlFlow("cleanup proof absent".into()))?;
    #[allow(clippy::too_many_arguments)]
    fn render<R: JavaResolver>(
        native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
        plan: &Plan,
        nodes: &HashSet<BlockId>,
        block: BlockId,
        original: &ManagedInsn,
        original_id: InsnId,
        ty: &JavaType,
        caught: &str,
        catch_all: bool,
        excluded: &[JavaType],
        hierarchy: &crate::type_compatibility::Hierarchy,
    ) -> Result<(), Unsupported> {
        use super::super::TypedEmitter;
        if !nodes.contains(&block) {
            return Err(Unsupported::ControlFlow(
                "cleanup left immutable proof".into(),
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
        if matches!(insn, ManagedInsn::Throw { .. }) {
            return rethrow(
                native,
                original,
                original_id,
                ty,
                caught,
                false,
                catch_all,
                excluded,
                hierarchy,
            );
        }
        if !plan.capture_blocks.contains(&block)
            && !matches!(insn, ManagedInsn::Phi { .. } | ManagedInsn::Branch { .. })
        {
            native.instruction(id)?;
        }
        let state = plan
            .regions
            .graph
            .states
            .iter()
            .find(|s| s.block == block)
            .ok_or_else(|| Unsupported::ControlFlow("cleanup state absent".into()))?;
        match insn {
            ManagedInsn::Branch {
                condition: Some(condition),
                then_edge,
                else_edge: Some(no),
            } => {
                let (condition, actual) = native.emitter.get(condition)?;
                native.emitter.compatible(&actual, &JavaType::Boolean)?;
                native.emitter.lines.push(format!("if ({condition}) {{"));
                super::render_copies(native, plan, then_edge)?;
                let yes = ir
                    .cfg()
                    .edge(then_edge)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                    .target();
                render(
                    native,
                    plan,
                    nodes,
                    yes,
                    original,
                    original_id,
                    ty,
                    caught,
                    catch_all,
                    excluded,
                    hierarchy,
                )?;
                native.emitter.lines.push("} else {".into());
                super::render_copies(native, plan, no)?;
                let no = ir
                    .cfg()
                    .edge(no)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                    .target();
                render(
                    native,
                    plan,
                    nodes,
                    no,
                    original,
                    original_id,
                    ty,
                    caught,
                    catch_all,
                    excluded,
                    hierarchy,
                )?;
                native.emitter.lines.push("}".into());
                Ok(())
            }
            _ => {
                let [edge] = state.normal.as_slice() else {
                    return Err(Unsupported::ControlFlow(
                        "cleanup continuation not exact".into(),
                    ));
                };
                super::render_copies(native, plan, *edge)?;
                let target = ir
                    .cfg()
                    .edge(*edge)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                    .target();
                render(
                    native,
                    plan,
                    nodes,
                    target,
                    original,
                    original_id,
                    ty,
                    caught,
                    catch_all,
                    excluded,
                    hierarchy,
                )
            }
        }
    }
    render(
        native,
        plan,
        nodes,
        entry,
        original,
        original_id,
        ty,
        caught,
        catch_all,
        excluded,
        hierarchy,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rethrow<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    insn: &ManagedInsn,
    original_id: InsnId,
    ty: &JavaType,
    caught: &str,
    guarded: bool,
    catch_all: bool,
    excluded: &[JavaType],
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> Result<(), Unsupported> {
    rethrow_enclosed(
        native,
        insn,
        original_id,
        ty,
        caught,
        guarded,
        catch_all,
        excluded,
        hierarchy,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rethrow_enclosed<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    insn: &ManagedInsn,
    original_id: InsnId,
    ty: &JavaType,
    caught: &str,
    guarded: bool,
    catch_all: bool,
    excluded: &[JavaType],
    hierarchy: &crate::type_compatibility::Hierarchy,
    enclosing: &[JavaType],
) -> Result<(), Unsupported> {
    let unchecked = |ty: &JavaType| {
        ["java.lang.RuntimeException", "java.lang.Error"]
            .iter()
            .any(|root| {
                native
                    .emitter
                    .resolver
                    .assignable(ty, &JavaType::Class((*root).into()))
            })
    };
    let declared = |ty: &JavaType| {
        native.method.throws.iter().any(|root| {
            native
                .emitter
                .resolver
                .assignable(ty, &JavaType::Class(root.clone()))
        })
    };
    if catch_all {
        let checked = match insn {
            ManagedInsn::Invoke { .. }
            | ManagedInsn::FieldGet { .. }
            | ManagedInsn::FieldPut { .. } => {
                source::emitted_contract(native.emitter, &native.method.owner, original_id, insn)
                    .ok_or_else(|| {
                        Unsupported::Metadata(format!(
                            "cleanup lacks exact source contract: {insn:?}"
                        ))
                    })?
                    .into_iter()
                    .map(JavaType::Class)
                    .collect::<Vec<_>>()
            }
            ManagedInsn::Throw { value } => vec![native.emitter.ty(*value)?],
            _ => vec![],
        };
        if !checked.iter().all(|ty| {
            unchecked(ty)
                || declared(ty)
                || enclosing
                    .iter()
                    .any(|outer| native.emitter.resolver.assignable(ty, outer))
                || excluded
                    .iter()
                    .any(|earlier| native.emitter.resolver.assignable(ty, earlier))
        }) {
            return Err(Unsupported::Metadata(
                "original cleanup rethrow has undeclared checked contract".into(),
            ));
        }
        // Never assign or cast the immutable catch parameter: javac precise
        // rethrow preserves even foreign undeclared checked Throwable objects.
        native.emitter.lines.push(format!("throw {caught};"));
    } else {
        // A lexical, immutable typed catch parameter has Java precise-rethrow
        // semantics. Check the exact protected source contract after applying
        // this catch's type and original earlier-handler exclusions.
        let precise = if guarded {
            false
        } else if let Some(types) =
            source::emitted_contract(native.emitter, &native.method.owner, original_id, insn)
        {
            let mut covered = true;
            for source in types.into_iter().map(JavaType::Class) {
                let source_to_catch = hierarchy.assignable(&source, ty);
                let catch_to_source = hierarchy.assignable(ty, &source);
                let selected = if source_to_catch == Compatibility::Assignable {
                    &source
                } else if catch_to_source == Compatibility::Assignable {
                    ty
                } else if source_to_catch == Compatibility::Incompatible
                    && catch_to_source == Compatibility::Incompatible
                {
                    continue;
                } else {
                    return Err(Unsupported::Type(
                        "typed cleanup source/catch ancestry is indeterminate".into(),
                    ));
                };
                covered &= unchecked(selected)
                    || declared(selected)
                    || enclosing
                        .iter()
                        .any(|outer| native.emitter.resolver.assignable(selected, outer))
                    || excluded
                        .iter()
                        .any(|earlier| native.emitter.resolver.assignable(selected, earlier));
            }
            covered
        } else {
            false
        };
        if !precise
            && !unchecked(ty)
            && !declared(ty)
            && !enclosing
                .iter()
                .any(|outer| native.emitter.resolver.assignable(ty, outer))
        {
            return Err(Unsupported::Metadata(
                "original typed cleanup rethrow has undeclared checked contract".into(),
            ));
        }
        native.emitter.lines.push(if guarded {
            format!("throw ({}) {caught};", ty.source()?)
        } else {
            format!("throw {caught};")
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "exception_dispatch_cleanup_tests.rs"]
mod cleanup_tests;

#[cfg(test)]
#[path = "exception_dispatch_outer_cleanup_tests.rs"]
mod outer_cleanup_tests;

#[cfg(test)]
#[path = "exception_dispatch_deferred_allocation_tests.rs"]
mod deferred_allocation_tests;

/// Nonempty original Throw contracts retain their type domain. Only a known
/// unchecked domain under the exact single catch-all source table is admitted.
fn unchecked_throw_contract(
    types: &[String],
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> bool {
    use crate::type_compatibility::Compatibility;
    let throwable = JavaType::Class("java.lang.Throwable".into());
    let roots = [
        JavaType::Class("java.lang.RuntimeException".into()),
        JavaType::Class("java.lang.Error".into()),
    ];
    !types.is_empty()
        && types.iter().all(|name| {
            let ty = JavaType::Class(name.clone());
            roots.iter().any(|root| {
                hierarchy.assignable(root, &throwable) == Compatibility::Assignable
                    && hierarchy.assignable(&ty, root) == Compatibility::Assignable
            })
        })
}
fn original_unchecked_throw_only_catch_all<R: JavaResolver>(
    native: &crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    plan: &Plan,
    entry: BlockId,
    original_id: InsnId,
    original: &ManagedInsn,
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> bool {
    if !matches!(original, ManagedInsn::Throw { .. }) {
        return false;
    }
    let origins: Vec<_> = native
        .emitter
        .ir
        .cfg()
        .block_ids()
        .filter(|&b| {
            native
                .emitter
                .ir
                .cfg()
                .block(b)
                .is_ok_and(|block| *block.payload() == original_id)
        })
        .collect();
    let [origin] = origins.as_slice() else {
        return false;
    };
    let Some(scope) = plan.regions.coverage.get(origin) else {
        return false;
    };
    let Some(edges) = plan.regions.handlers.get(origin) else {
        return false;
    };
    let [edge] = edges.as_slice() else {
        return false;
    };
    if plan
        .types
        .get(*scope)
        .is_none_or(|types| types != &[JavaType::Class("java.lang.Throwable".into())])
        || native.emitter.ir.cfg().edge(*edge).map_or(true, |edge| {
            edge.target() != entry || !matches!(edge.kind(), EdgeKind::Exception(_))
        })
    {
        return false;
    }
    source::emitted_contract(native.emitter, &native.method.owner, original_id, original)
        .is_some_and(|types| unchecked_throw_contract(&types, hierarchy))
}

#[cfg(test)]
#[path = "exception_dispatch_unchecked_contract_tests.rs"]
mod unchecked_contract_tests;
