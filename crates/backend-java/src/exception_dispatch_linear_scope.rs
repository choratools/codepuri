//! Exact linear source scopes keep checked catches legal across distinct calls.
use super::*;

pub(super) struct Group {
    pub blocks: Vec<BlockId>,
    scope: usize,
    exit: EdgeId,
    handlers: Vec<EdgeId>,
}

fn same_handler(ir: &MethodIr<SsaForm>, left: EdgeId, right: EdgeId) -> bool {
    let (Ok(left), Ok(right)) = (ir.cfg().edge(left), ir.cfg().edge(right)) else {
        return false;
    };
    left.kind() == right.kind() && left.target() == right.target()
}

pub(super) fn prove<R: JavaResolver>(
    emitter: &crate::Emitter<'_, R>,
    caller: &str,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
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
        let protected = &md.region_scope().protected;
        if protected.len() < 2 || protected.len() > 256 {
            continue;
        }
        // The native try is precisely this source range: one normal entry,
        // one linear path and one normal exit. No handler may reenter it.
        let entries: Vec<_> = protected
            .iter()
            .copied()
            .filter(|b| {
                *b == emitter.ir.cfg().entry()
                    || emitter.ir.cfg().block(*b).is_ok_and(|node| {
                        node.incoming().iter().any(|edge| {
                            emitter
                                .ir
                                .cfg()
                                .edge(*edge)
                                .is_ok_and(|e| !protected.contains(&e.source()))
                        })
                    })
            })
            .collect();
        let [entry] = entries.as_slice() else {
            continue;
        };
        let mut blocks = Vec::new();
        let mut current = *entry;
        let mut exit = None;
        let mut handlers: Option<Vec<EdgeId>> = None;
        let mut valid = true;
        while protected.contains(&current) {
            if blocks.contains(&current)
                || plan.monitors.owners[&current].is_some()
                || plan.rethrows.elided.contains(&current)
                || plan.capture_blocks.contains(&current)
            {
                valid = false;
                break;
            }
            let node = emitter
                .ir
                .cfg()
                .block(current)
                .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
            if blocks.last().is_some_and(|previous| {
                node.incoming().len() != 1
                    || emitter
                        .ir
                        .cfg()
                        .edge(node.incoming()[0])
                        .is_ok_and(|e| e.source() != *previous)
            }) {
                valid = false;
                break;
            }
            let state = states[&current];
            let [edge] = state.normal.as_slice() else {
                valid = false;
                break;
            };
            if !plan.copies.get(edge).is_none_or(Vec::is_empty)
                || plan.rethrows.outer_monitors.contains_key(&current)
            {
                valid = false;
                break;
            }
            let insn = emitter
                .ir
                .instruction(*node.payload())
                .map_err(|e| Unsupported::Instruction(e.to_string()))?;
            if matches!(
                insn,
                ManagedInsn::MonitorEnter { .. }
                    | ManagedInsn::MonitorExit { .. }
                    | ManagedInsn::Branch {
                        condition: Some(_),
                        ..
                    }
                    | ManagedInsn::Switch { .. }
                    | ManagedInsn::Return { .. }
                    | ManagedInsn::Throw { .. }
                    | ManagedInsn::Phi { .. }
            ) {
                valid = false;
                break;
            }
            if !state.exceptional.is_empty() {
                let Some(edges) = plan.regions.handlers.get(&current) else {
                    valid = false;
                    break;
                };
                if edges
                    .iter()
                    .any(|e| !plan.copies.get(e).is_none_or(Vec::is_empty))
                {
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
            blocks.push(current);
            current = emitter
                .ir
                .cfg()
                .edge(*edge)
                .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
                .target();
            if !protected.contains(&current) {
                exit = Some(*edge);
            }
        }
        if !valid || blocks.len() != protected.len() {
            continue;
        }
        let (Some(exit), Some(handlers)) = (exit, handlers) else {
            continue;
        };
        if handlers.iter().any(|edge| {
            emitter.ir.cfg().edge(*edge).is_ok_and(|e| {
                protected.contains(&e.target())
                    || plan.rethrows.paths.contains_key(&e.target())
                    || plan.rethrows.cleanups.contains_key(&e.target())
            })
        }) {
            continue;
        }
        // Every checked catch must be justified by at least one operation
        // inside the SAME emitted try. Missing contracts remain an error.
        let mut needs_group = false;
        for ty in &plan.types[scope] {
            let mut reachable = false;
            for block in &blocks {
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
            groups.insert(
                *entry,
                Group {
                    blocks,
                    scope,
                    exit,
                    handlers,
                },
            );
        }
    }
    Ok(groups)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render<R: JavaResolver>(
    native: &mut crate::exception_emitter::ExceptionEmitter<'_, '_, R>,
    metadata: &[&dyn RegionMetadata],
    plan: &Plan,
    states: &HashMap<BlockId, usize>,
    prefix: &str,
    loop_name: &str,
    group: &Group,
) -> Result<(), Unsupported> {
    use super::super::TypedEmitter;
    native.caught = plan.types[group.scope].clone();
    native.begin_try()?;
    for block in &group.blocks {
        let id = *native
            .emitter
            .ir
            .cfg()
            .block(*block)
            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?
            .payload();
        let insn = native
            .emitter
            .ir
            .instruction(id)
            .map_err(|e| Unsupported::Instruction(e.to_string()))?;
        if !matches!(
            insn,
            ManagedInsn::Branch {
                condition: None,
                ..
            }
        ) {
            native.instruction(id)?;
        }
    }
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
            "{prefix}_pc = {};\ncontinue {loop_name};",
            states[&target]
        ));
        emitted.push(ty.clone());
    }
    native.emitter.lines.push("}".into());
    super::render_transfer(native, plan, states, prefix, loop_name, group.exit)
}

#[cfg(test)]
#[path = "exception_dispatch_linear_scope_tests.rs"]
mod tests;
