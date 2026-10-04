//! Exact per-instruction exception dispatch for arbitrary original CFGs.
//! This is a child of `exceptions`: typed operation/copy semantics are shared
//! with the structured emitter; only scheduling changes. No whole-method try.
#[path = "exception_dispatch_capture_declarations.rs"]
mod capture_declarations;
#[cfg(test)]
#[path = "exception_dispatch_capture_type_tests.rs"]
mod capture_type_tests;
#[path = "exception_dispatch_capture_types.rs"]
mod capture_types;
#[cfg(test)]
#[path = "exception_dispatch_handler_phi_tests.rs"]
mod handler_phi_tests;
#[path = "exception_dispatch_linear_scope.rs"]
mod linear_scope;
#[cfg(test)]
#[path = "exception_dispatch_monitor_tests.rs"]
mod monitor_tests;
#[path = "exception_dispatch_monitors.rs"]
mod monitors;
#[path = "exception_dispatch_source_scope.rs"]
mod source_scope;
#[cfg(test)]
#[path = "exception_dispatch_tests.rs"]
mod tests;
#[path = "exception_dispatch_throws.rs"]
mod throws;
#[cfg(test)]
#[path = "exception_dispatch_trusted_normal_tests.rs"]
mod trusted_normal_tests;
#[cfg(test)]
#[path = "exception_dispatch_unchecked_throw_tests.rs"]
mod unchecked_throw_tests;
use super::*;
use crate::{JavaMethod, Unsupported, type_compatibility::Compatibility};
use std::collections::VecDeque;

/// Validated arbitrary-flow exception plan. Graph and source metadata are kept
/// together so a renderer cannot reinterpret CFG adjacency as catch priority.
pub struct Plan {
    /// Original complete graph and per-site source-table priority.
    pub regions: decomp_structure::exception_dispatch::ScopedPlan,
    /// Original simultaneous Phi copies on original edges.
    pub copies: HashMap<EdgeId, Vec<Copy>>,
    /// All concrete original locals, excluding incoming declaration bindings.
    pub locals: Vec<ValueId>,
    /// Exact catch types by original source-scope and table index.
    pub types: Vec<Vec<JavaType>>,
    /// Original move-exception binding for each handler entry, if used.
    pub captures: HashMap<BlockId, ValueId>,
    /// Caught-object and identity-alias declarations proven by handler metadata.
    pub contextual: HashMap<ValueId, JavaType>,
    /// Original move-exception nodes, which become catch-local assignments.
    pub capture_blocks: HashSet<BlockId>,
    monitors: monitors::Proof,
    rethrows: throws::Proof,
}

// Original metadata supplies caught-object producer types before Phi validation.
pub(crate) fn declarations_for_scopes(
    ir: &MethodIr<SsaForm>,
    return_type: &JavaType,
    resolver: &impl JavaResolver,
    metadata: &[&(impl RegionMetadata + ?Sized)],
) -> Result<HashMap<ValueId, JavaType>, Unsupported> {
    let captured = capture_declarations::collect(ir, metadata, resolver)
        .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
    crate::contextual_locals::declaration_plan_with_caught_producers(
        ir,
        return_type,
        resolver,
        &captured.contextual,
    )
    .map_err(|err| Unsupported::Type(format!("exception local declaration: {err:?}")))
}

/// Validate definitions, original source dispatch and prethrow SSA availability.
/// A decreasing worklist solves definite availability across all normal and
/// exceptional edges. Normal transfers see successful results; exceptional
/// transfers see the state before the original instruction, never its results.
pub fn plan(
    ir: &MethodIr<SsaForm>,
    metadata: &[&dyn RegionMetadata],
    bindings: &[ValueId],
    resolver: &impl JavaResolver,
    declarations: &HashMap<ValueId, JavaType>,
) -> Result<Plan, Error> {
    let scopes = metadata
        .iter()
        .map(|m| m.region_scope().clone())
        .collect::<Vec<_>>();
    let regions = decomp_structure::exception_dispatch::plan_scopes(ir.cfg(), &scopes)
        .map_err(Error::Scope)?;
    let (p, definitions, initial) = prepare_region(ir, bindings, declarations)?;
    if p.phi.contains(&ir.cfg().entry()) {
        return Err(Error::Unsupported(
            "dispatch entry Phi has no external input",
        ));
    }
    let capture_declarations::Captures {
        types,
        captures,
        blocks: capture_blocks,
        mut contextual,
    } = capture_declarations::collect(ir, metadata, resolver)?;
    // Pure copies and joins inherit only caught-object evidence, narrowing
    // compatible inferred supertypes without changing arbitrary object values.
    // Every immediate Phi input must already have the same concrete capture
    // lineage type. Normal inputs, missing evidence and conflicting families
    // never receive a guessed declaration. Original edges and prethrow
    // availability are independently checked by the complete dispatch plan.
    let mut changed = true;
    while changed {
        changed = false;
        for insn in ir.instructions().values() {
            let (result, ty) = match insn {
                ManagedInsn::Move { result, value } => (*result, contextual.get(value).cloned()),
                ManagedInsn::Phi { result, inputs } if !inputs.is_empty() => {
                    let ty = contextual.get(&inputs[0].1).filter(|ty| {
                        inputs
                            .iter()
                            .all(|(_, value)| contextual.get(value) == Some(*ty))
                    });
                    (*result, ty.cloned())
                }
                _ => continue,
            };
            if let Some(ty) = ty
                && capture_types::compatible_lineage(
                    ir,
                    result,
                    &ty,
                    metadata[0].hierarchy(),
                    resolver,
                )?
                && !contextual.contains_key(&result)
            {
                contextual.insert(result, ty);
                changed = true;
            }
        }
    }
    for state in &regions.graph.states {
        let (_, insn) = p.insn(state.block)?;
        if matches!(insn, ManagedInsn::Opaque { .. })
            && !capture_blocks.contains(&state.block)
            && !matches!(insn, ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects }
                if reads.is_empty() && *effects == EffectFlags::NONE && writes.iter().all(|v| initial.contains(v)))
        {
            return Err(Error::Unsupported("unknown dispatch operation"));
        }
        if regions.coverage.contains_key(&state.block)
            && !capture_blocks.contains(&state.block)
            && may_throw(ir, insn) == state.exceptional.is_empty()
        {
            return Err(Error::Unsupported(
                "original throwing-site coverage mismatch",
            ));
        }
        if let ManagedInsn::Switch {
            key,
            cases,
            default_edge: default,
        } = insn
        {
            if ir.value(*key).map_err(invalid)?.ty() != &managed_ir::types::ManagedType::Int
                || cases.len() + 1 != state.normal.len()
                || !state.normal.contains(default)
                || ir.cfg().edge(*default).map_err(invalid)?.kind() != &EdgeKind::Unconditional
            {
                return Err(Error::Unsupported(
                    "dispatch switch instruction/edge mismatch",
                ));
            }
            let mut labels = HashSet::new();
            let mut case_edges = HashSet::new();
            for (label, edge) in cases {
                if i32::try_from(*label).is_err()
                    || !labels.insert(*label)
                    || !case_edges.insert(*edge)
                    || !state.normal.contains(edge)
                    || ir.cfg().edge(*edge).map_err(invalid)?.kind()
                        != &EdgeKind::Switch(*label as u64)
                {
                    return Err(Error::Unsupported("dispatch switch case mismatch"));
                }
            }
        } else if !matches!(
            insn,
            ManagedInsn::Branch { .. } | ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }
        ) && (state.normal.len() != 1
            || ir.cfg().edge(state.normal[0]).map_err(invalid)?.kind() != &EdgeKind::Unconditional)
        {
            return Err(Error::Unsupported(
                "ordinary dispatch instruction lacks original successor",
            ));
        }
    }
    let universe = definitions.union(&initial).copied().collect::<HashSet<_>>();
    // Bound explicit memberships; this is a typed budget refusal, not a source
    // substitute. Typical small/medium methods remain far below this ceiling.
    if regions.graph.states.len().saturating_mul(universe.len()) > 4_000_000 {
        return Err(Error::Unsupported("exception dispatch availability budget"));
    }
    let mut available = regions
        .graph
        .states
        .iter()
        .map(|s| (s.block, universe.clone()))
        .collect::<HashMap<_, _>>();
    let mut queue = regions
        .graph
        .states
        .iter()
        .map(|s| s.block)
        .collect::<VecDeque<_>>();
    let mut queued = queue.iter().copied().collect::<HashSet<_>>();
    while let Some(block) = queue.pop_front() {
        queued.remove(&block);
        let mut next = universe.clone();
        if block == ir.cfg().entry() {
            next.retain(|v| initial.contains(v));
        }
        for &edge in ir.cfg().block(block).map_err(invalid)?.incoming() {
            let e = ir.cfg().edge(edge).map_err(invalid)?;
            let mut state = available[&e.source()].clone();
            if !matches!(e.kind(), EdgeKind::Exception(_)) {
                state.extend(defined(p.insn(e.source())?.1));
            }
            if let Some(copies) = p.copies.get(&edge) {
                state.extend(copies.iter().map(|c| c.destination));
            }
            next.retain(|v| state.contains(v));
        }
        if next != available[&block] {
            available.insert(block, next);
            for &edge in ir.cfg().block(block).map_err(invalid)?.outgoing() {
                let target = ir.cfg().edge(edge).map_err(invalid)?.target();
                if queued.insert(target) {
                    queue.push_back(target);
                }
            }
        }
    }
    for state in &regions.graph.states {
        let (_, insn) = p.insn(state.block)?;
        let before = &available[&state.block];
        let used = if p.phi.contains(&state.block) {
            defined(insn)
        } else {
            reads(insn)
        };
        for value in used {
            if !before.contains(&value) {
                return Err(Error::Unavailable(value));
            }
        }
        for &edge in &state.exceptional {
            if let Some(copies) = p.copies.get(&edge) {
                for copy in copies {
                    if !before.contains(&copy.source) {
                        return Err(Error::Unavailable(copy.source));
                    }
                }
            }
        }
        let mut success = before.clone();
        success.extend(defined(insn));
        for &edge in &state.normal {
            if let Some(copies) = p.copies.get(&edge) {
                for copy in copies {
                    if !success.contains(&copy.source) {
                        return Err(Error::Unavailable(copy.source));
                    }
                }
            }
        }
    }
    let mut locals = definitions.into_iter().collect::<Vec<_>>();
    locals.sort_by_key(|v| format!("{v:?}"));
    let monitors = monitors::prove(ir, &regions, metadata)?;
    let mut plan = Plan {
        regions,
        copies: p.copies,
        locals,
        types,
        captures,
        contextual,
        capture_blocks,
        monitors,
        rethrows: throws::Proof::default(),
    };
    plan.rethrows = throws::prove(ir, &plan, resolver, metadata)?;
    Ok(plan)
}

/// Render using the same typed Emitter, checked-call allowances and parallel
/// edge-copy emitter as ordinary and structured-exception methods.
pub(crate) fn render_into<'a, R: JavaResolver>(
    e: &mut crate::Emitter<'a, R>,
    method: &'a JavaMethod,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
) -> Result<(), Unsupported> {
    render_into_with_initializer(e, method, metadata, plan, None)
}

fn render_into_with_initializer<'a, R: JavaResolver>(
    e: &mut crate::Emitter<'a, R>,
    method: &'a JavaMethod,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    initialized: Option<InsnId>,
) -> Result<(), Unsupported> {
    use super::TypedEmitter;
    let scopes = metadata
        .iter()
        .map(|m| m.region_scope().clone())
        .collect::<Vec<_>>();
    if decomp_structure::exception_dispatch::plan_scopes(e.ir.cfg(), &scopes)
        .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
        != plan.regions
        || monitors::prove(e.ir, &plan.regions, metadata)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            != plan.monitors
        || throws::prove(e.ir, plan, e.resolver, metadata)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            != plan.rethrows
    {
        return Err(Unsupported::ControlFlow(
            "stale exception dispatch snapshot".into(),
        ));
    }
    e.declarations.extend(plan.contextual.clone());
    let mut native = crate::exception_emitter::ExceptionEmitter {
        emitter: e,
        method,
        caught: vec![],
        protected: false,
    };
    native.hoist(&plan.locals)?;
    let e = &mut native.emitter;
    let mut prefix = format!("dispatch{}", e.next);
    e.next += 1;
    while [
        format!("{prefix}_pc"),
        format!("{prefix}_loop"),
        format!("{prefix}_caught"),
    ]
    .iter()
    .any(|n| e.reserved.contains(n))
        || plan
            .regions
            .graph
            .states
            .iter()
            .enumerate()
            .any(|(index, state)| {
                plan.monitors.acquisitions.contains_key(&state.block)
                    && [
                        format!("{prefix}_lock{index}_loop"),
                        format!("{prefix}_release{index}"),
                        format!("{prefix}_entered{index}"),
                    ]
                    .iter()
                    .any(|name| e.reserved.contains(name))
            })
    {
        prefix.push('_');
    }
    for suffix in ["pc", "loop", "caught"] {
        e.reserved.insert(format!("{prefix}_{suffix}"));
    }
    let states = plan
        .regions
        .graph
        .states
        .iter()
        .enumerate()
        .map(|(i, s)| (s.block, i))
        .collect::<HashMap<_, _>>();
    e.lines.push(format!(
        "int {prefix}_pc = 0;\n{prefix}_loop: while (true) {{\nswitch ({prefix}_pc) {{"
    ));
    render_group(
        &mut native,
        method,
        metadata,
        plan,
        &states,
        &prefix,
        None,
        &format!("{prefix}_loop"),
        None,
        initialized,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_group<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    method: &JavaMethod,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    states: &HashMap<BlockId, usize>,
    prefix: &str,
    owner: Option<BlockId>,
    loop_name: &str,
    release_name: Option<&str>,
    initialized: Option<InsnId>,
) -> Result<(), Unsupported> {
    use super::TypedEmitter;
    let grouped = linear_scope::prove(native.emitter, &method.owner, metadata, plan, owner)?;
    let linear_members: HashSet<_> = grouped
        .values()
        .flat_map(|g| g.blocks.iter().copied())
        .collect();
    let source_groups = source_scope::prove(
        native.emitter,
        &method.owner,
        metadata,
        plan,
        &linear_members,
        owner,
    )?;
    let grouped_members: HashSet<_> = grouped
        .values()
        .flat_map(|g| g.blocks.iter().skip(1).copied())
        .chain(
            source_groups
                .values()
                .flat_map(|g| g.blocks.iter().skip(1).copied()),
        )
        .collect();
    for (index, state) in plan.regions.graph.states.iter().enumerate() {
        if plan.monitors.owners[&state.block] != owner
            || plan.rethrows.elided.contains(&state.block)
        {
            continue;
        }
        if grouped_members.contains(&state.block) {
            continue;
        }
        if let Some(group) = source_groups.get(&state.block) {
            for entry in &group.entries {
                native
                    .emitter
                    .lines
                    .push(format!("case {}:", states[entry]));
            }
            native.emitter.lines.push("{".into());
            source_scope::render(native, metadata, plan, states, prefix, loop_name, group)?;
            native.emitter.lines.push("}".into());
            continue;
        }
        native.emitter.lines.push(format!("case {index}: {{"));
        if let Some(group) = grouped.get(&state.block) {
            linear_scope::render(native, metadata, plan, states, prefix, loop_name, group)?;
            native.emitter.lines.push("}".into());
            continue;
        }
        let id = *native
            .emitter
            .ir
            .cfg()
            .block(state.block)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            .payload();
        let insn = native
            .emitter
            .ir
            .instruction(id)
            .map_err(|e| Unsupported::Instruction(e.to_string()))?
            .clone();
        if plan.monitors.releases.contains_key(&state.block) {
            let release = release_name
                .ok_or_else(|| Unsupported::ControlFlow("release outside lexical lock".into()))?;
            native
                .emitter
                .lines
                .push(format!("{release} = {index};\nbreak {loop_name};"));
            native.emitter.lines.push("}".into());
            continue;
        }
        let handlers = plan.regions.handlers.get(&state.block);
        let acquisition = (handlers.is_some() && matches!(insn, ManagedInsn::MonitorEnter { .. }))
            .then(|| format!("{prefix}_entered{index}"));
        if let Some(flag) = &acquisition {
            native.emitter.reserved.insert(flag.clone());
            native
                .emitter
                .lines
                .push(format!("boolean {flag} = false;"));
        }
        let mut snapshots = HashMap::new();
        if let Some(edges) = handlers {
            for edge in edges {
                for copy in plan.copies.get(edge).into_iter().flatten() {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        snapshots.entry((*edge, copy.destination))
                    {
                        let (_, expected) = native.emitter.get(copy.destination)?;
                        let (source, ty) = native.emitter.operand(copy.source, &expected)?;
                        native.emitter.compatible(&ty, &expected)?;
                        let mut name = format!("snapshot{}", native.emitter.next);
                        native.emitter.next += 1;
                        while native.emitter.reserved.contains(&name) {
                            name.push('_');
                        }
                        native.emitter.reserved.insert(name.clone());
                        native
                            .emitter
                            .lines
                            .push(format!("{} {name} = {source};", ty.source()?));
                        entry.insert((name, ty));
                    }
                }
            }
            native.caught = plan.types[plan.regions.coverage[&state.block]].clone();
            native.begin_try()?;
        }
        let fragment_start = native.emitter.lines.len();
        if let ManagedInsn::MonitorEnter { object } = insn {
            let (object, ty) = native.emitter.get(object)?;
            if !matches!(ty, JavaType::Class(_) | JavaType::Array(_)) {
                return Err(Unsupported::Type(
                    "monitor acquisition lacks reference type".into(),
                ));
            }
            let held_loop = format!("{prefix}_lock{index}_loop");
            let release = format!("{prefix}_release{index}");
            native
                .emitter
                .reserved
                .extend([held_loop.clone(), release.clone()]);
            native
                .emitter
                .lines
                .push(format!("int {release} = -1;\nsynchronized ({object}) {{"));
            if let Some(flag) = &acquisition {
                native.emitter.lines.push(format!("{flag} = true;"));
            }
            let [edge] = state.normal.as_slice() else {
                return Err(Unsupported::ControlFlow(
                    "acquisition lacks exact continuation".into(),
                ));
            };
            render_copies(native, plan, *edge)?;
            let target = native
                .emitter
                .ir
                .cfg()
                .edge(*edge)
                .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                .target();
            native.emitter.lines.push(format!(
                "{prefix}_pc = {};\n{held_loop}: while (true) {{\nswitch ({prefix}_pc) {{",
                states[&target]
            ));
            let saved_caught = native.caught.clone();
            let saved_protected = native.protected;
            render_group(
                native,
                method,
                metadata,
                plan,
                states,
                prefix,
                Some(state.block),
                &held_loop,
                Some(&release),
                initialized,
            )?;
            native.caught = saved_caught;
            native.protected = saved_protected;
            native.emitter.lines.push("}".into()); // synchronized: original release
            native.emitter.lines.push(format!("switch ({release}) {{"));
            for released in &plan.regions.graph.states {
                if plan.monitors.owners[&released.block] == Some(state.block)
                    && !plan.rethrows.elided.contains(&released.block)
                    && let Some(edge) = plan.monitors.releases.get(&released.block)
                {
                    native
                        .emitter
                        .lines
                        .push(format!("case {}: {{", states[&released.block]));
                    render_copies(native, plan, *edge)?;
                    let target = native
                        .emitter
                        .ir
                        .cfg()
                        .edge(*edge)
                        .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                        .target();
                    native
                        .emitter
                        .lines
                        .push(format!("{prefix}_pc = {};\nbreak;\n}}", states[&target]));
                }
            }
            native.emitter.lines.push(
                "default: throw new IllegalStateException(\"invalid monitor release\");\n}".into(),
            );
        } else if Some(id) != initialized
            && !plan.capture_blocks.contains(&state.block)
            && !matches!(
                insn,
                ManagedInsn::Phi { .. } | ManagedInsn::Branch { .. } | ManagedInsn::Switch { .. }
            )
        {
            // Explicit throws need the same local catch allowance as invokes.
            let mut signature = method.clone();
            if handlers.is_some()
                && let ManagedInsn::Throw { value } = insn
            {
                let (_, ty) = native.emitter.get(value)?;
                if native
                    .caught
                    .iter()
                    .any(|t| native.emitter.resolver.assignable(&ty, t))
                    && let JavaType::Class(name) = ty
                {
                    signature.throws.push(name);
                }
                native.emitter.current = Some(id);
                native.emitter.instruction(&insn, &signature)?;
            } else {
                native.instruction(id)?;
            }
        }
        let deferred_allocation =
            throws::deferred_allocation(native.emitter, plan, id, &insn, fragment_start);
        if let Some(edges) = handlers {
            let scope = plan.regions.coverage[&state.block];
            let md = metadata[scope];
            // With no checked declaration contract, Throwable precise rethrow
            // admits every encoded catch type and preserves arbitrary priority.
            // The same precise-rethrow rule applies to explicitly thrown values
            // whose emitted type is proved unchecked. A narrow checked entry in
            // their original DEX table is legal in the ordered instanceof
            // dispatcher even when a per-instruction Java native catch would be
            // unreachable. Checked or unproved thrown types keep native checks.
            // Checked-call sites use legal native catches; shadowed later DEX
            // entries are provably unselectable and remain in the complete plan.
            let enclosed_checked = plan.rethrows.outer_monitors.get(&state.block);
            let guarded = enclosed_checked.is_none_or(Vec::is_empty)
                && match &insn {
                    ManagedInsn::Throw { value } => {
                        proven_unchecked_throw_type(&native.emitter.ty(*value)?, md.hierarchy())
                    }
                    ManagedInsn::Invoke { .. }
                    | ManagedInsn::FieldGet { .. }
                    | ManagedInsn::FieldPut { .. } => {
                        throws::source_contract(native.emitter, &native.method.owner, id, &insn)
                            .is_some_and(|contract| contract.is_empty())
                    }
                    _ => true,
                };
            // Preserve a proven immutable typed capture using an actual lexical
            // catch parameter when every original clause is legal in Java.
            // Narrow unreachable checked catches retain the guarded dispatcher.
            let native_typed_cleanup = edges.iter().enumerate().all(|(index, edge)| {
                let Some(catch) = md.region_scope().catches.get(index) else {
                    return false;
                };
                if md.is_catch_all(&catch.key) {
                    return false;
                }
                let Ok(edge) = native.emitter.ir.cfg().edge(*edge) else {
                    return false;
                };
                plan.rethrows.cleanups.contains_key(&edge.target())
                    && native_catch_reachable(
                        native.emitter,
                        &native.method.owner,
                        id,
                        &insn,
                        &plan.types[scope][index],
                        md.hierarchy(),
                    ) == Ok(true)
            });
            let guarded = guarded && !native_typed_cleanup;
            let caught = format!("{prefix}_caught");
            native.protected = false;
            if guarded {
                native
                    .emitter
                    .lines
                    .push(format!("}} catch (java.lang.Throwable {caught}) {{"));
                if let Some(flag) = &acquisition
                    && enclosed_checked.is_none()
                {
                    // The Java lexical acquisition encloses its body. Only
                    // failure before acquisition belongs to this DEX table;
                    // escaped body cleanup must keep its original identity.
                    native
                        .emitter
                        .lines
                        .push(format!("if ({flag}) {{ throw {caught}; }}"));
                }
            }
            let mut emitted = Vec::<JavaType>::new();
            let mut catch_all = false;
            for (handler_index, &edge) in edges.iter().enumerate() {
                let ty = &plan.types[scope][handler_index];
                let is_all = md.is_catch_all(&md.region_scope().catches[handler_index].key);
                if !guarded {
                    let mut shadowed = false;
                    for earlier in &emitted {
                        match md.hierarchy().assignable(ty, earlier) {
                            Compatibility::Assignable => {
                                shadowed = true;
                                break;
                            }
                            Compatibility::Incompatible => {}
                            Compatibility::Indeterminate(_) => {
                                return Err(Unsupported::Type(
                                    "native catch ordering proof unavailable".into(),
                                ));
                            }
                        }
                    }
                    if shadowed {
                        continue;
                    }
                    if !native_catch_reachable(
                        native.emitter,
                        &native.method.owner,
                        id,
                        &insn,
                        ty,
                        md.hierarchy(),
                    )? && !enclosed_checked.is_some_and(|checked| {
                        checked.iter().any(|thrown| {
                            md.hierarchy().assignable(thrown, ty) == Compatibility::Assignable
                                || md.hierarchy().assignable(ty, thrown)
                                    == Compatibility::Assignable
                        })
                    }) {
                        return Err(Unsupported::Type(
                            "encoded checked catch is unreachable under the resolved Java contract"
                                .into(),
                        ));
                    }
                    native
                        .emitter
                        .lines
                        .push(format!("}} catch ({} {caught}) {{", ty.source()?));
                } else {
                    native.emitter.lines.push(if is_all {
                        if emitted.is_empty() {
                            "{".into()
                        } else {
                            "} else {".into()
                        }
                    } else {
                        format!(
                            "{}if ({caught} instanceof {}) {{",
                            if emitted.is_empty() { "" } else { "} else " },
                            ty.source()?
                        )
                    });
                }
                let target = native
                    .emitter
                    .ir
                    .cfg()
                    .edge(edge)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                    .target();
                if let Some(value) = plan.captures.get(&target)
                    && !plan.rethrows.suppressions.contains_key(&target)
                    && !plan.rethrows.shared_cleanups.contains_key(&target)
                {
                    let (name, expected) = native.emitter.get(*value)?;
                    native.emitter.compatible(ty, &expected)?;
                    native
                        .emitter
                        .lines
                        .push(format!("{name} = ({}) {caught};", ty.source()?));
                }
                for copy in plan.copies.get(&edge).into_iter().flatten() {
                    let (source, actual) = &snapshots[&(edge, copy.destination)];
                    let (destination, expected) = native.emitter.get(copy.destination)?;
                    native.emitter.compatible(actual, &expected)?;
                    native
                        .emitter
                        .lines
                        .push(format!("{destination} = {source};"));
                }
                if plan.rethrows.paths.contains_key(&target) {
                    throws::rethrow_enclosed(
                        native,
                        &insn,
                        id,
                        ty,
                        &caught,
                        guarded,
                        is_all,
                        // Only native catch clauses exclude earlier checked types.
                        // An instanceof dispatcher does not narrow precise rethrow.
                        if guarded { &[] } else { &emitted },
                        md.hierarchy(),
                        plan.rethrows
                            .enclosing
                            .get(&target)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                    )?;
                } else if plan.rethrows.shared_cleanups.contains_key(&target) {
                    throws::render_shared_cleanup(
                        native,
                        metadata,
                        plan,
                        target,
                        &caught,
                        (id, &insn, deferred_allocation.as_ref()),
                    )?;
                } else if plan.rethrows.suppressions.contains_key(&target) {
                    throws::render_suppression(native, metadata, plan, target, &caught, id, &insn)?;
                } else if plan.rethrows.cleanups.contains_key(&target) {
                    throws::render_cleanup(
                        native,
                        plan,
                        target,
                        &insn,
                        id,
                        ty,
                        &caught,
                        guarded,
                        is_all,
                        &emitted,
                        md.hierarchy(),
                        deferred_allocation.as_ref(),
                    )?;
                } else {
                    native.emitter.lines.push(format!(
                        "{prefix}_pc = {};\ncontinue {loop_name};",
                        states[&target]
                    ));
                }
                emitted.push(ty.clone());
                catch_all |= is_all;
            }
            if guarded {
                native.emitter.lines.push(if catch_all {
                    "}".into()
                } else {
                    format!("}} else {{ throw {caught}; }}")
                });
            }
            native.emitter.lines.push("}".into());
        }
        match insn {
            ManagedInsn::MonitorEnter { .. } => {
                native.emitter.lines.push(format!("continue {loop_name};"));
            }
            ManagedInsn::Branch {
                condition: Some(condition),
                then_edge,
                else_edge: Some(no),
            } => {
                // Finish this borrow before parallel-copy callback mutation.
                let (condition, ty) = native.emitter.get(condition)?;
                native.emitter.compatible(&ty, &JavaType::Boolean)?;
                native.emitter.lines.push(format!("if ({condition}) {{"));
                // Recreate the transfer hook to avoid retaining an Emitter borrow.
                render_transfer(native, plan, states, prefix, loop_name, then_edge)?;
                native.emitter.lines.push("} else {".into());
                render_transfer(native, plan, states, prefix, loop_name, no)?;
                native.emitter.lines.push("}".into());
            }
            ManagedInsn::Switch {
                key,
                cases,
                default_edge: default,
            } => {
                let (key, ty) = native.emitter.get(key)?;
                native.emitter.compatible(&ty, &JavaType::Int)?;
                native.emitter.lines.push(format!("switch ({key}) {{"));
                for (label, edge) in cases {
                    native
                        .emitter
                        .lines
                        .push(format!("case {}: {{", label as i32));
                    render_transfer(native, plan, states, prefix, loop_name, edge)?;
                    native.emitter.lines.push("}".into());
                }
                native.emitter.lines.push("default: {".into());
                render_transfer(native, plan, states, prefix, loop_name, default)?;
                native.emitter.lines.push("}\n}".into());
            }
            ManagedInsn::Return { .. } | ManagedInsn::Throw { .. } => {}
            _ => {
                let [edge] = state.normal.as_slice() else {
                    return Err(Unsupported::ControlFlow(
                        "nonterminal dispatch state".into(),
                    ));
                };
                render_transfer(native, plan, states, prefix, loop_name, *edge)?;
            }
        }
        native.emitter.lines.push("}".into());
    }
    native
        .emitter
        .lines
        .push("default: throw new IllegalStateException(\"invalid CFG state\");\n}\n}".into());
    Ok(())
}

fn proven_unchecked_throw_type(
    thrown: &JavaType,
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> bool {
    if !matches!(thrown, JavaType::Class(_)) {
        return false;
    }
    let throwable = JavaType::Class("java.lang.Throwable".into());
    ["java.lang.RuntimeException", "java.lang.Error"]
        .into_iter()
        .any(|name| {
            let unchecked = JavaType::Class(name.into());
            // Identity alone does not supply a missing declaration's ancestry.
            hierarchy.assignable(&unchecked, &throwable) == Compatibility::Assignable
                && hierarchy.assignable(thrown, &unchecked) == Compatibility::Assignable
        })
}

fn native_catch_reachable<R: JavaResolver>(
    emitter: &crate::Emitter<'_, R>,
    caller: &str,
    id: InsnId,
    insn: &ManagedInsn,
    caught: &JavaType,
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> Result<bool, Unsupported> {
    // Java admits unchecked catches and their supertypes even when the protected
    // instruction declares no such exception. Narrow checked catches need an
    // intersecting checked contract; otherwise publishing would produce javac-
    // invalid source. Guarded zero-contract sites avoid this restriction.
    for unchecked in ["java.lang.RuntimeException", "java.lang.Error"] {
        let unchecked = JavaType::Class(unchecked.into());
        if hierarchy.assignable(caught, &unchecked) == Compatibility::Assignable
            || hierarchy.assignable(&unchecked, caught) == Compatibility::Assignable
        {
            return Ok(true);
        }
    }
    let thrown = match insn {
        ManagedInsn::Invoke { .. }
        | ManagedInsn::FieldGet { .. }
        | ManagedInsn::FieldPut { .. } => throws::source_contract(emitter, caller, id, insn)
            .ok_or_else(|| {
                Unsupported::Metadata(format!(
                    "unresolved native protected source contract: {insn:?}"
                ))
            })?
            .into_iter()
            .map(JavaType::Class)
            .collect::<Vec<_>>(),
        ManagedInsn::Throw { value } if crate::proven_null_throw(emitter.ir, *value) => {
            vec![JavaType::Class("java.lang.NullPointerException".into())]
        }
        ManagedInsn::Throw { value } => vec![emitter.ty(*value)?],
        _ => Vec::new(),
    };
    for thrown in thrown {
        if hierarchy.assignable(&thrown, caught) == Compatibility::Assignable
            || hierarchy.assignable(caught, &thrown) == Compatibility::Assignable
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn render_transfer<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    plan: &Plan,
    states: &HashMap<BlockId, usize>,
    prefix: &str,
    loop_name: &str,
    edge: EdgeId,
) -> Result<(), Unsupported> {
    render_copies(native, plan, edge)?;
    let target = native
        .emitter
        .ir
        .cfg()
        .edge(edge)
        .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
        .target();
    native.emitter.lines.push(format!(
        "{prefix}_pc = {};\ncontinue {loop_name};",
        states[&target]
    ));
    Ok(())
}

fn render_copies<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    plan: &Plan,
    edge: EdgeId,
) -> Result<(), Unsupported> {
    crate::render_steps(
        native.emitter,
        &[crate::branching::Step::Copies {
            edge,
            copies: plan.copies.get(&edge).cloned().unwrap_or_default(),
        }],
        native.method,
    )?;
    Ok(())
}

/// Transactional original-method source generation for arbitrary exception CFGs.
/// All metadata, SSA and typed-rendering failures produce no source declaration.
pub fn emit_method(
    ir: &MethodIr<SsaForm>,
    method: &crate::JavaMethod,
    bindings: &crate::DeclarationBindings,
    resolver: &impl JavaResolver,
    metadata: &[&dyn RegionMetadata],
) -> crate::SourceResult {
    let Some(first) = metadata.first() else {
        return crate::SourceResult::Unsupported(Unsupported::Metadata(
            "empty source scope tables".into(),
        ));
    };
    match emit_declaration(ir, method, bindings, resolver, *first, Some(metadata)) {
        Ok((source, method_name, parameter_names)) => crate::SourceResult::Complete {
            source,
            method_name,
            parameter_names,
        },
        Err(error) => crate::SourceResult::Unsupported(error),
    }
}
fn emit_declaration(
    ir: &MethodIr<SsaForm>,
    m: &crate::JavaMethod,
    b: &crate::DeclarationBindings,
    r: &impl JavaResolver,
    metadata: &(impl RegionMetadata + ?Sized),
    disjoint: Option<&[&dyn RegionMetadata]>,
) -> Result<(String, String, Vec<String>), crate::Unsupported> {
    use crate::{Unsupported, managed_type, reserve_class, reserve_type};
    crate::qualified(&m.owner)?;
    if m.name.starts_with('<')
        || b.parameters.len() != m.parameters.len()
        || (!b.names.is_empty() && b.names.len() != m.parameters.len())
        || (m.is_static && b.receiver.is_some())
    {
        return Err(Unsupported::Metadata(
            "exception declaration binding/signature mismatch".into(),
        ));
    }
    let mut reserved = HashSet::from(["java".to_string()]);
    reserve_class(&mut reserved, &m.owner);
    reserve_type(&mut reserved, &m.return_type);
    for t in &m.parameters {
        reserve_type(&mut reserved, t);
    }
    for v in ir.values().values() {
        if let Ok(t) = managed_type(v.ty(), r) {
            reserve_type(&mut reserved, &t);
        }
    }
    for i in ir.instructions().values() {
        match i {
            ManagedInsn::Invoke { method, .. } => {
                if let Some(t) = r.method(*method) {
                    reserve_class(&mut reserved, &t.owner);
                    reserve_type(&mut reserved, &t.return_type);
                    for p in &t.parameters {
                        reserve_type(&mut reserved, p);
                    }
                }
            }
            ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                if let Some(f) = r.field(*field) {
                    reserve_class(&mut reserved, &f.owner);
                    reserve_type(&mut reserved, &f.ty);
                }
            }
            ManagedInsn::Cast { target, .. } | ManagedInsn::InstanceOf { target, .. } => {
                if let Some(t) = r.instruction_type(*target) {
                    reserve_type(&mut reserved, &t);
                }
            }
            _ => {}
        }
    }
    if let Some(all) = disjoint {
        for md in all {
            for c in &md.region_scope().catches {
                if let Some(t) = md.catch_type(&c.key) {
                    reserve_type(&mut reserved, &t);
                }
            }
        }
    } else {
        for c in &metadata.region_scope().catches {
            if let Some(t) = metadata.catch_type(&c.key) {
                reserve_type(&mut reserved, &t);
            }
        }
    }
    let declarations = match disjoint {
        Some(tables) => declarations_for_scopes(ir, &m.return_type, r, tables),
        None => declarations_for_scopes(ir, &m.return_type, r, &[metadata]),
    }?;
    let scoped_resolver = allocation_proofs::ScopedResolver::new(r);
    let mut e = crate::Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir,
        resolver: &scoped_resolver,
        values: HashMap::new(),
        lines: Vec::new(),
        next: 0,
        parameters: HashSet::new(),
        reserved,
        current: None,
        hoisted: HashSet::new(),
        declarations,
    };
    let mut names = Vec::new();
    let mut params = Vec::new();
    for (index, (value, ty)) in b.parameters.iter().zip(&m.parameters).enumerate() {
        if *ty == JavaType::Void {
            return Err(Unsupported::Metadata("void parameter".into()));
        }
        let hint = b
            .names
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("p{index}"));
        let mut name = crate::sanitize_identifier(&hint);
        if name
            .strip_prefix('v')
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        {
            name.insert_str(0, "p_");
        }
        let base = name.clone();
        let mut suffix = 0;
        while e.reserved.contains(&name) {
            suffix += 1;
            name = format!("{base}_{suffix}");
        }
        e.reserved.insert(name.clone());
        if let Some(v) = value {
            crate::bind(&mut e, *v, name.clone(), ty.clone())?;
        }
        params.push(format!("{} {name}", ty.source()?));
        names.push(name);
    }
    if let Some(v) = b.receiver {
        crate::bind(&mut e, v, "this".into(), JavaType::Class(m.owner.clone()))?;
    }
    let bound: Vec<_> = e.parameters.iter().copied().collect();
    let all = disjoint.ok_or(Unsupported::Metadata("missing source scope tables".into()))?;
    let plan = plan(ir, all, &bound, r, &e.declarations)
        .map_err(|error| Unsupported::ControlFlow(error.to_string()))?;
    scoped_resolver.install(allocation_proofs::prove(ir, all, r));
    render_into(&mut e, m, all, &plan)?;
    let original = if let Some(recipe) = r.checked_source_body_bridge(m) {
        let mut index = 0;
        let caught = loop {
            let name = format!("checkedBodyFailure{index}");
            if !e.reserved.contains(&name) && !e.values.values().any(|(value, _)| value == &name) {
                break name;
            }
            index += 1;
        };
        e.lines = recipe.wrap_body(&e.lines, &caught)?;
        recipe.body_original(m).ok_or_else(|| {
            Unsupported::Metadata("dispatch whole-body rendering signature".into())
        })?
    } else {
        m
    };
    let name = crate::sanitize_identifier(&original.name);
    let throws = if original.throws.is_empty() {
        String::new()
    } else {
        format!(
            " throws {}",
            original
                .throws
                .iter()
                .map(|n| crate::qualified(n))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )
    };
    let source = format!(
        "public {}{} {name}({}){throws} {{\n{}\n}}\n",
        if original.is_static { "static " } else { "" },
        original.return_type.source()?,
        params.join(", "),
        e.lines
            .iter()
            .map(|s| format!("    {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok((source, name, names))
}

/// Render an original constructor after an independently proved chain.
pub(crate) fn render_initialized_into<'a, R: JavaResolver>(
    e: &mut crate::Emitter<'a, R>,
    method: &'a JavaMethod,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    initialized: InsnId,
) -> Result<(), Unsupported> {
    render_into_with_initializer(e, method, metadata, plan, Some(initialized))
}

#[cfg(test)]
#[path = "exception_super_contract_tests.rs"]
mod super_contract_tests;
