//! Bounded acyclic constructor suffix joins and simultaneous Phi transfers.
use super::*;

// A bounded acyclic suffix join emits its original effects once. Phi copies
// remain on the two original incoming edges, before the common final store.
pub(super) fn emit_phi_diamond<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    branch: decomp_core::ids::BlockId,
    seen: &mut HashSet<decomp_core::ids::BlockId>,
    path: &HashSet<decomp_core::ids::BlockId>,
    budget: &mut usize,
    depth: usize,
) -> Result<bool, Unsupported> {
    let fail = || Unsupported::ControlFlow("unproved constructor suffix Phi diamond".into());
    let cfg = e.ir.cfg();
    if cfg.block_count() > 256 {
        return Ok(false);
    }
    let Some(merge) = decomp_core::analysis::post_dominators(cfg).immediate_post_dominator(branch)
    else {
        return Ok(false);
    };
    let merge_node = cfg.block(merge).map_err(|_| fail())?;
    if !matches!(
        e.ir.instruction(*merge_node.payload())
            .map_err(|_| fail())?,
        ManagedInsn::Phi { .. }
    ) {
        return Ok(false);
    }
    // SSA Phi clusters retain the incoming edge IDs of the cluster head.
    // Every destination gets a separate local, and every edge source is read
    // before any destination is assigned.
    let incoming = merge_node.incoming();
    if incoming.len() != 2 || incoming[0] == incoming[1] {
        return Err(fail());
    }
    let mut slots = Vec::new();
    let mut cluster = HashSet::new();
    let mut current = merge;
    let exit;
    loop {
        if slots.len() >= 32 || path.contains(&current) || !cluster.insert(current) {
            return Err(fail());
        }
        let node = cfg.block(current).map_err(|_| fail())?;
        let ManagedInsn::Phi { result, inputs } =
            e.ir.instruction(*node.payload()).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        if inputs.len() != 2
            || inputs[0].0 == inputs[1].0
            || inputs.iter().any(|(edge, _)| !incoming.contains(edge))
            || node.outgoing().len() != 1
            || e.values.contains_key(result)
            || slots.iter().any(|(_, prior, _, _, _)| prior == result)
        {
            return Err(fail());
        }
        let ty = e.ty(*result)?;
        let name = fresh_local(e);
        slots.push((current, *result, inputs.clone(), ty, name));
        let edge_id = node.outgoing()[0];
        let edge = cfg.edge(edge_id).map_err(|_| fail())?;
        if edge.kind() != &EdgeKind::Unconditional {
            return Err(fail());
        }
        let next = edge.target();
        let next_node = cfg.block(next).map_err(|_| fail())?;
        if matches!(
            e.ir.instruction(*next_node.payload()).map_err(|_| fail())?,
            ManagedInsn::Phi { .. }
        ) {
            if next_node.incoming() != [edge_id] {
                return Err(fail());
            }
            current = next;
        } else {
            exit = next;
            break;
        }
    }
    let branch_node = cfg.block(branch).map_err(|_| fail())?;
    let ManagedInsn::Branch {
        condition: Some(condition),
        then_edge,
        else_edge: Some(else_edge),
    } =
        e.ir.instruction(*branch_node.payload())
            .map_err(|_| fail())?
    else {
        return Err(fail());
    };
    if branch_node.outgoing().len() != 2
        || !branch_node.outgoing().contains(then_edge)
        || !branch_node.outgoing().contains(else_edge)
        || then_edge == else_edge
    {
        return Err(fail());
    }
    let (condition_expression, condition_type) = e.get(*condition)?;
    compatible(&condition_type, &JavaType::Boolean)?;
    for (_, _, _, ty, name) in &slots {
        e.lines.push(format!("{} {name};", ty.source()?));
    }
    let base = e.values.clone();
    let mut occurrences = HashSet::new();
    e.lines.push(format!("if ({condition_expression}) {{"));
    for (arm, first) in [*then_edge, *else_edge].into_iter().enumerate() {
        e.values = base.clone();
        let mut edge_id = first;
        let expected = if arm == 0 {
            EdgeKind::BranchTrue
        } else {
            EdgeKind::BranchFalse
        };
        if cfg.edge(first).map_err(|_| fail())?.kind() != &expected {
            return Err(fail());
        }
        let mut steps = 0;
        loop {
            let edge = cfg.edge(edge_id).map_err(|_| fail())?;
            let block = edge.target();
            if block == merge {
                break;
            }
            steps += 1;
            if steps > 32 || *budget == 0 || path.contains(&block) || !occurrences.insert(block) {
                return Err(fail());
            }
            let node = cfg.block(block).map_err(|_| fail())?;
            if node.incoming() != [edge_id] || node.outgoing().len() != 1 {
                return Err(fail());
            }
            let id = *node.payload();
            let insn = e.ir.instruction(id).map_err(|_| fail())?;
            *budget -= 1;
            seen.insert(block);
            e.current = Some(id);
            match insn {
                ManagedInsn::Branch {
                    condition: None,
                    then_edge,
                    else_edge: None,
                } if node.outgoing() == [*then_edge] => {}
                ManagedInsn::Phi { .. }
                | ManagedInsn::Branch { .. }
                | ManagedInsn::Return { .. }
                | ManagedInsn::Throw { .. }
                | ManagedInsn::Invoke {
                    kind: InvokeKind::Super,
                    ..
                } => return Err(fail()),
                ManagedInsn::Invoke {
                    kind: InvokeKind::Special,
                    args,
                    ..
                } => {
                    // Reuse the ordinary allocation pairing/argument/effect proof;
                    // this arm cannot introduce another receiver initializer.
                    let root = args
                        .first()
                        .and_then(|value| e.allocation_root(*value))
                        .ok_or_else(fail)?;
                    if e.object_plan(root)?.initializer != id || e.instruction(insn, m)? {
                        return Err(fail());
                    }
                }
                _ => {
                    if e.instruction(insn, m)? {
                        return Err(fail());
                    }
                }
            }
            edge_id = node.outgoing()[0];
            if cfg.edge(edge_id).map_err(|_| fail())?.kind() != &EdgeKind::Unconditional {
                return Err(fail());
            }
        }
        let mut copies = Vec::new();
        for (_, _, inputs, ty, name) in &slots {
            let source = inputs
                .iter()
                .find(|(edge, _)| *edge == edge_id)
                .map(|(_, value)| *value)
                .ok_or_else(fail)?;
            let (expression, actual) = e.operand(source, ty)?;
            e.compatible(&actual, ty)?;
            let snapshot = fresh_local(e);
            e.lines
                .push(format!("{} {snapshot} = {expression};", ty.source()?));
            copies.push((name, snapshot));
        }
        for (name, snapshot) in copies {
            e.lines.push(format!("{name} = {snapshot};"));
        }
        if arm == 0 {
            e.lines.push("} else {".into());
        }
    }
    e.lines.push("}".into());
    e.values = base;
    for (block, result, _, ty, name) in slots {
        if *budget == 0 || e.values.insert(result, (name, ty)).is_some() {
            return Err(fail());
        }
        *budget -= 1;
        seen.insert(block);
    }
    let mut next_path = path.clone();
    next_path.extend(occurrences);
    next_path.extend(cluster);
    emit_constructor_tail_path(e, m, exit, seen, &next_path, budget, depth + 1)?;
    Ok(true)
}

fn fresh_local<R: JavaResolver>(e: &mut Emitter<'_, R>) -> String {
    loop {
        let name = format!("v{}", e.next);
        e.next += 1;
        if e.reserved.insert(name.clone()) {
            return name;
        }
    }
}
