//! Branching/cyclic PC regions retain one exact original checked-catch scope.
use super::*;

pub(super) struct Group {
    pub blocks: Vec<BlockId>,
    pub entries: Vec<BlockId>,
    scope: usize,
    handlers: Vec<EdgeId>,
}

fn same_handler(ir: &MethodIr<SsaForm>, a: EdgeId, b: EdgeId) -> bool {
    let (Ok(a), Ok(b)) = (ir.cfg().edge(a), ir.cfg().edge(b)) else {
        return false;
    };
    a.kind() == b.kind() && a.target() == b.target()
}

pub(super) fn prove<R: JavaResolver>(
    emitter: &crate::Emitter<'_, R>,
    caller: &str,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    excluded: &HashSet<BlockId>,
    owner: Option<BlockId>,
) -> Result<HashMap<BlockId, Group>, Unsupported> {
    let mut groups = HashMap::new();
    if owner.is_some() {
        return Ok(groups);
    }
    let states: HashMap<_, _> = plan
        .regions
        .graph
        .states
        .iter()
        .map(|s| (s.block, s))
        .collect();
    for (scope, md) in metadata.iter().enumerate() {
        let protected: HashSet<_> = md.region_scope().protected.iter().copied().collect();
        if protected.len() < 2 || protected.len() > 256 || !protected.is_disjoint(excluded) {
            continue;
        }
        let mut entries = HashSet::new();
        let mut exits = Vec::new();
        let mut valid = true;
        let mut handlers: Option<Vec<EdgeId>> = None;
        for block in &protected {
            let node = emitter
                .ir
                .cfg()
                .block(*block)
                .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
            let Some(state) = states.get(block) else {
                valid = false;
                break;
            };
            if plan.monitors.owners[block].is_some()
                || plan.capture_blocks.contains(block)
                || plan.rethrows.elided.contains(block)
                || plan.rethrows.outer_monitors.contains_key(block)
                || plan.regions.coverage.get(block) != Some(&scope)
            {
                valid = false;
                break;
            }
            if *block == emitter.ir.cfg().entry() {
                entries.insert(*block);
            }
            for edge in node.incoming() {
                let e = emitter
                    .ir
                    .cfg()
                    .edge(*edge)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
                if !protected.contains(&e.source()) {
                    if matches!(e.kind(), EdgeKind::Exception(_)) {
                        valid = false;
                        break;
                    }
                    entries.insert(*block);
                }
            }
            let insn = emitter
                .ir
                .instruction(*node.payload())
                .map_err(|e| Unsupported::Instruction(e.to_string()))?;
            if matches!(
                insn,
                ManagedInsn::MonitorEnter { .. }
                    | ManagedInsn::MonitorExit { .. }
                    | ManagedInsn::Return { .. }
                    | ManagedInsn::Throw { .. }
            ) || state.normal.is_empty()
            {
                valid = false;
                break;
            }
            for edge in &state.normal {
                let e = emitter
                    .ir
                    .cfg()
                    .edge(*edge)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
                if !protected.contains(&e.target()) {
                    exits.push(*edge);
                }
            }
            if !state.exceptional.is_empty() {
                let Some(edges) = plan.regions.handlers.get(block) else {
                    valid = false;
                    break;
                };
                if edges.iter().any(|e| {
                    !plan.copies.get(e).is_none_or(Vec::is_empty)
                        || emitter
                            .ir
                            .cfg()
                            .edge(*e)
                            .map_or(true, |e| protected.contains(&e.target()))
                }) {
                    valid = false;
                    break;
                }
                if let Some(first) = &handlers {
                    if first.len() != edges.len()
                        || first
                            .iter()
                            .zip(edges)
                            .any(|(a, b)| !same_handler(emitter.ir, *a, *b))
                    {
                        valid = false;
                        break;
                    }
                } else {
                    handlers = Some(edges.clone());
                }
            }
        }
        if !valid || entries.is_empty() || exits.is_empty() {
            continue;
        }
        let Some(handlers) = handlers else {
            continue;
        };
        // These handlers must remain real PC states. Existing lexical cleanup
        // transforms own their entries and cannot be jumped into by this path.
        if handlers.iter().any(|edge| {
            emitter.ir.cfg().edge(*edge).map_or(true, |edge| {
                let target = edge.target();
                plan.rethrows.elided.contains(&target)
                    || plan.rethrows.paths.contains_key(&target)
                    || plan.rethrows.cleanups.contains_key(&target)
                    || plan.rethrows.shared_cleanups.contains_key(&target)
                    || plan.rethrows.suppressions.contains_key(&target)
                    || plan.monitors.owners[&target].is_some()
            })
        }) || exits.iter().any(|edge| {
            emitter
                .ir
                .cfg()
                .edge(*edge)
                .map_or(true, |edge| plan.rethrows.elided.contains(&edge.target()))
        }) {
            continue;
        }
        // Keep the original complete ordered handler table, including catch kind.
        if handlers.len() != md.region_scope().catches.len()
            || handlers
                .iter()
                .zip(&md.region_scope().catches)
                .any(|(e, c)| {
                    emitter.ir.cfg().edge(*e).map_or(true, |e| {
                        e.target() != c.entry || e.kind() != &EdgeKind::Exception(c.key.clone())
                    })
                })
        {
            continue;
        }
        let entries: Vec<_> = plan
            .regions
            .graph
            .states
            .iter()
            .map(|s| s.block)
            .filter(|b| entries.contains(b))
            .collect();
        let entry = entries[0];
        let mut seen = HashSet::new();
        let mut pending = entries.clone();
        while let Some(b) = pending.pop() {
            if !seen.insert(b) {
                continue;
            }
            for edge in &states[&b].normal {
                let e = emitter
                    .ir
                    .cfg()
                    .edge(*edge)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
                if protected.contains(&e.target()) {
                    pending.push(e.target());
                }
            }
        }
        if seen != protected {
            continue;
        }
        let mut needs_group = false;
        if plan.types[scope].iter().enumerate().any(|(i, ty)| {
            plan.types[scope][..i].iter().any(|earlier| {
                matches!(
                    md.hierarchy().assignable(ty, earlier),
                    Compatibility::Indeterminate(_)
                )
            })
        }) {
            continue;
        }
        for ty in &plan.types[scope] {
            let mut reachable = false;
            for block in &protected {
                let id = *emitter
                    .ir
                    .cfg()
                    .block(*block)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                    .payload();
                let insn = emitter
                    .ir
                    .instruction(id)
                    .map_err(|e| Unsupported::Instruction(e.to_string()))?;
                let site =
                    super::native_catch_reachable(emitter, caller, id, insn, ty, md.hierarchy())?;
                reachable |= site;
                needs_group |= !site && !states[block].exceptional.is_empty();
            }
            if !reachable {
                valid = false;
                break;
            }
        }
        if valid && needs_group {
            let mut blocks = vec![entry];
            blocks.extend(
                plan.regions
                    .graph
                    .states
                    .iter()
                    .map(|s| s.block)
                    .filter(|b| *b != entry && protected.contains(b)),
            );
            groups.insert(
                entry,
                Group {
                    blocks,
                    entries,
                    scope,
                    handlers,
                },
            );
        }
    }
    Ok(groups)
}

struct Renderer<'a> {
    plan: &'a Plan,
    states: &'a HashMap<BlockId, usize>,
    prefix: &'a str,
    inner: String,
    members: HashSet<BlockId>,
    initialized_reads: std::sync::Arc<HashMap<ValueId, HashSet<InsnId>>>,
}
impl Renderer<'_> {
    fn transfer<R: JavaResolver>(
        &self,
        native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
        edge: EdgeId,
    ) -> Result<(), Unsupported> {
        super::render_copies(native, self.plan, edge)?;
        let target = native
            .emitter
            .ir
            .cfg()
            .edge(edge)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            .target();
        native.emitter.lines.push(format!(
            "{}_pc = {};\n{} {};",
            self.prefix,
            self.states[&target],
            if self.members.contains(&target) {
                "continue"
            } else {
                "break"
            },
            self.inner
        ));
        Ok(())
    }
    fn block<R: JavaResolver>(
        &self,
        native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
        block: BlockId,
    ) -> Result<(), Unsupported> {
        let id = *native
            .emitter
            .ir
            .cfg()
            .block(block)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            .payload();
        let previous = native
            .emitter
            .allocation_readable
            .replace(self.initialized_reads.clone());
        native.emitter.current = Some(id);
        let result = self.block_with_readable_slots(native, block);
        native.emitter.allocation_readable = previous;
        result
    }
    fn block_with_readable_slots<R: JavaResolver>(
        &self,
        native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
        block: BlockId,
    ) -> Result<(), Unsupported> {
        use super::super::TypedEmitter;
        let id = *native
            .emitter
            .ir
            .cfg()
            .block(block)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            .payload();
        let insn = native
            .emitter
            .ir
            .instruction(id)
            .map_err(|e| Unsupported::Instruction(e.to_string()))?
            .clone();
        if !matches!(
            insn,
            ManagedInsn::Phi { .. } | ManagedInsn::Branch { .. } | ManagedInsn::Switch { .. }
        ) {
            native.instruction(id)?;
        }
        match insn {
            ManagedInsn::Branch {
                condition: Some(value),
                then_edge,
                else_edge: Some(no),
            } => {
                let (name, ty) = native.emitter.get(value)?;
                native.emitter.compatible(&ty, &JavaType::Boolean)?;
                native.emitter.lines.push(format!("if ({name}) {{"));
                self.transfer(native, then_edge)?;
                native.emitter.lines.push("} else {".into());
                self.transfer(native, no)?;
                native.emitter.lines.push("}".into());
            }
            ManagedInsn::Switch {
                key,
                cases,
                default_edge,
            } => {
                let (name, ty) = native.emitter.get(key)?;
                native.emitter.compatible(&ty, &JavaType::Int)?;
                native.emitter.lines.push(format!("switch ({name}) {{"));
                for (label, edge) in cases {
                    native.emitter.lines.push(format!("case {label}: {{"));
                    self.transfer(native, edge)?;
                    native.emitter.lines.push("}".into());
                }
                native.emitter.lines.push("default: {".into());
                self.transfer(native, default_edge)?;
                native.emitter.lines.push("}\n}".into());
            }
            _ => {
                let state = self
                    .plan
                    .regions
                    .graph
                    .states
                    .iter()
                    .find(|s| s.block == block)
                    .ok_or_else(|| Unsupported::ControlFlow("scope state".into()))?;
                let [edge] = state.normal.as_slice() else {
                    return Err(Unsupported::ControlFlow("scope terminal".into()));
                };
                self.transfer(native, *edge)?;
            }
        }
        Ok(())
    }
}

fn successful_allocation_reads<R: JavaResolver>(
    emitter: &crate::Emitter<'_, R>,
) -> Result<HashMap<ValueId, HashSet<InsnId>>, Unsupported> {
    // Source PC cases may be printed before their initializer case. Readability
    // follows the exact successful constructor CFG exit, never printed order.
    // Exceptional constructor bypasses remain unavailable; the initializer and
    // allocation themselves are never authorized by this temporary overlay.
    let mut initialized_reads = HashMap::new();
    let mut success = HashMap::new();
    for value in emitter.hoisted.iter().copied() {
        let Some(root) = emitter.allocation_root(value) else {
            continue;
        };
        let Ok(object) = emitter.object_plan(root) else {
            continue;
        };
        if !object.aliases.iter().any(|(alias, _)| *alias == value) {
            continue;
        }
        let available = match success.entry(root) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut available = emitter.after_initializer(object.initializer)?;
                available.remove(&object.initializer);
                available.remove(&object.allocation);
                entry.insert(available)
            }
        };
        initialized_reads.insert(value, available.clone());
    }
    Ok(initialized_reads)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    states: &HashMap<BlockId, usize>,
    prefix: &str,
    outer: &str,
    group: &Group,
) -> Result<(), Unsupported> {
    use super::super::TypedEmitter;
    let initialized_reads = successful_allocation_reads(native.emitter)?;
    let renderer = Renderer {
        plan,
        states,
        prefix,
        inner: format!("{prefix}_scope{}", group.scope),
        members: group.blocks.iter().copied().collect(),
        initialized_reads: std::sync::Arc::new(initialized_reads),
    };
    native.caught = plan.types[group.scope].clone();
    native.begin_try()?;
    native.emitter.lines.push(format!(
        "{}: while (true) {{\nswitch ({prefix}_pc) {{",
        renderer.inner
    ));
    for block in &group.blocks {
        native
            .emitter
            .lines
            .push(format!("case {}: {{", states[block]));
        renderer.block(native, *block)?;
        native.emitter.lines.push("}".into());
    }
    native
        .emitter
        .lines
        .push("default: throw new IllegalStateException(\"invalid scope state\");\n}\n}".into());
    native.protected = false;
    let caught = format!("{prefix}_caught");
    let md = metadata[group.scope];
    let mut emitted = Vec::new();
    for (i, edge) in group.handlers.iter().enumerate() {
        let ty = &plan.types[group.scope][i];
        if emitted
            .iter()
            .any(|earlier| md.hierarchy().assignable(ty, earlier) == Compatibility::Assignable)
        {
            continue;
        }
        native
            .emitter
            .lines
            .push(format!("}} catch ({} {caught}) {{", ty.source()?));
        let target = native
            .emitter
            .ir
            .cfg()
            .edge(*edge)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            .target();
        if let Some(value) = plan.captures.get(&target) {
            let (name, expected) = native.emitter.get(*value)?;
            native.emitter.compatible(ty, &expected)?;
            native
                .emitter
                .lines
                .push(format!("{name} = ({}) {caught};", ty.source()?));
        }
        native.emitter.lines.push(format!(
            "{prefix}_pc = {};\ncontinue {outer};",
            states[&target]
        ));
        emitted.push(ty.clone());
    }
    native.emitter.lines.push(format!("}}\ncontinue {outer};"));
    Ok(())
}

#[cfg(test)]
#[path = "exception_dispatch_source_scope_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "exception_dispatch_source_scope_readability_tests.rs"]
mod readability_tests;
