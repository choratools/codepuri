//! Java25 retains bounded linear effectful branches before the original receiver
//! initializer. This helper neither commutes effects nor proves final stores.
use super::*;

pub(super) fn emit<R: ConstructorResolver>(
    e: &mut Emitter<'_, R>,
    signature: &JavaMethod,
    branch: BlockId,
    aliases: &HashSet<ValueId>,
    seen: &mut HashSet<BlockId>,
) -> Result<Option<BlockId>, Unsupported> {
    let checkpoint = (
        e.values.clone(),
        e.lines.clone(),
        e.next,
        e.reserved.clone(),
        e.current,
        e.hoisted.clone(),
        e.declarations.clone(),
        seen.clone(),
    );
    let restore = |e: &mut Emitter<'_, R>, seen: &mut HashSet<BlockId>| {
        e.values = checkpoint.0.clone();
        e.lines = checkpoint.1.clone();
        e.next = checkpoint.2;
        e.reserved = checkpoint.3.clone();
        e.current = checkpoint.4;
        e.hoisted = checkpoint.5.clone();
        e.declarations = checkpoint.6.clone();
        *seen = checkpoint.7.clone();
    };
    match emit_linear(e, signature, branch, aliases, seen) {
        Ok(Some(next)) => return Ok(Some(next)),
        Ok(None) => restore(e, seen),
        Err(error) => {
            restore(e, seen);
            return Err(error);
        }
    }
    let result = emit_region(e, signature, branch, aliases, seen);
    if !matches!(result, Ok(Some(_))) {
        restore(e, seen);
    }
    result
}

fn emit_linear<R: ConstructorResolver>(
    e: &mut Emitter<'_, R>,
    signature: &JavaMethod,
    branch: BlockId,
    aliases: &HashSet<ValueId>,
    seen: &mut HashSet<BlockId>,
) -> Result<Option<BlockId>, Unsupported> {
    let fail = || Unsupported::ControlFlow("unproved Java25 pre-init diamond".into());
    let cfg = e.ir.cfg();
    let node = cfg.block(branch).map_err(|_| fail())?;
    let ManagedInsn::Branch {
        condition: Some(condition),
        then_edge,
        else_edge: Some(else_edge),
    } = e.ir.instruction(*node.payload()).map_err(|_| fail())?
    else {
        return Ok(None);
    };
    if node.outgoing().len() != 2
        || !node.outgoing().contains(then_edge)
        || !node.outgoing().contains(else_edge)
        || then_edge == else_edge
    {
        return Err(fail());
    }
    let yes = cfg.edge(*then_edge).map_err(|_| fail())?;
    let no = cfg.edge(*else_edge).map_err(|_| fail())?;
    if yes.kind() != &EdgeKind::BranchTrue || no.kind() != &EdgeKind::BranchFalse {
        return Err(fail());
    }
    let Some(merge) = decomp_core::analysis::post_dominators(cfg).immediate_post_dominator(branch)
    else {
        return Ok(None);
    };
    let merge_node = cfg.block(merge).map_err(|_| fail())?;
    let ManagedInsn::Phi { result, inputs } =
        e.ir.instruction(*merge_node.payload())
            .map_err(|_| fail())?
    else {
        return Ok(None);
    };
    if inputs.len() != 2 || merge_node.incoming().len() != 2 || merge_node.outgoing().len() != 1 {
        return Ok(None);
    }
    let next_edge = cfg.edge(merge_node.outgoing()[0]).map_err(|_| fail())?;
    // Multiple simultaneous columns and receiver identity need the full region
    // proof; emitting only the first Phi would lose original edge snapshots.
    if aliases.contains(result)
        || cfg
            .block(next_edge.target())
            .ok()
            .and_then(|b| e.ir.instruction(*b.payload()).ok())
            .is_some_and(|i| matches!(i, ManagedInsn::Phi { .. }))
    {
        return Ok(None);
    }
    if next_edge.kind() != &EdgeKind::Unconditional {
        return Err(fail());
    }
    let arm = |mut edge: decomp_core::ids::EdgeId| -> Option<(Vec<(BlockId, InsnId)>, ValueId)> {
        let mut visited = HashSet::new();
        let mut steps = Vec::new();
        for _ in 0..128 {
            let target = cfg.edge(edge).ok()?.target();
            if target == merge {
                let value = inputs.iter().find(|(input, _)| *input == edge)?.1;
                return Some((steps, value));
            }
            if seen.contains(&target) || !visited.insert(target) {
                return None;
            }
            let node = cfg.block(target).ok()?;
            if node.incoming().len() != 1 || node.outgoing().len() != 1 {
                return None;
            }
            let id = *node.payload();
            if matches!(
                e.ir.instruction(id).ok()?,
                ManagedInsn::Phi { .. }
                    | ManagedInsn::Branch {
                        condition: Some(_),
                        ..
                    }
                    | ManagedInsn::Return { .. }
                    | ManagedInsn::Throw { .. }
                    | ManagedInsn::Opaque { .. }
            ) {
                return None;
            }
            steps.push((target, id));
            edge = node.outgoing()[0];
            if cfg.edge(edge).ok()?.kind() != &EdgeKind::Unconditional {
                return None;
            }
        }
        None
    };
    let (Some(yes_arm), Some(no_arm)) = (arm(*then_edge), arm(*else_edge)) else {
        return Ok(None);
    };
    if yes_arm
        .0
        .iter()
        .any(|(b, _)| no_arm.0.iter().any(|(other, _)| other == b))
    {
        return Err(fail());
    }
    let (condition, condition_type) = e.get(*condition)?;
    if condition_type != JavaType::Boolean {
        return Err(fail());
    }
    let ty = e.ty(*result)?;
    let proof = crate::contextual_locals::checked_phi_plan(
        e.ir,
        *result,
        &ty,
        &signature.return_type,
        &crate::type_compatibility::Hierarchy::default(),
        e.resolver,
    )
    .map_err(|error| Unsupported::Type(format!("pre-init Phi family: {error:?}")))?;
    for value in proof.locals {
        if e.declarations.get(&value).is_some_and(|old| old != &ty) {
            return Err(fail());
        }
        e.declarations.insert(value, ty.clone());
    }
    let mut name = format!("v{}", e.next);
    e.next += 1;
    while e.reserved.contains(&name) {
        name = format!("v{}", e.next);
        e.next += 1;
    }
    e.reserved.insert(name.clone());
    e.lines.push(format!("{} {name};", ty.source()?));
    let available = e.values.clone();
    let mut emit_arm =
        |steps: &[(BlockId, InsnId)], value: ValueId| -> Result<Vec<String>, Unsupported> {
            e.values = available.clone();
            let begin = e.lines.len();
            for &(_, id) in steps {
                e.current = Some(id);
                let insn = e.ir.instruction(id).map_err(|_| fail())?;
                if matches!(
                    insn,
                    ManagedInsn::Branch {
                        condition: None,
                        ..
                    }
                ) {
                    continue;
                }
                if !source25::emit(e, insn, signature, aliases)? {
                    return Err(Unsupported::ControlFlow(format!(
                        "unproved Java25 pre-init diamond operation {id:?} {insn:?}"
                    )));
                }
            }
            let (expression, actual) = e.operand(value, &ty)?;
            e.compatible(&actual, &ty)?;
            e.lines.push(format!("{name} = {expression};"));
            Ok(e.lines.drain(begin..).collect())
        };
    let yes_lines = emit_arm(&yes_arm.0, yes_arm.1)?;
    let no_lines = emit_arm(&no_arm.0, no_arm.1)?;
    e.values = available;
    e.values.insert(*result, (name, ty));
    e.lines.push(format!("if ({condition}) {{"));
    e.lines
        .extend(yes_lines.into_iter().map(|line| format!("    {line}")));
    e.lines.push("} else {".into());
    e.lines
        .extend(no_lines.into_iter().map(|line| format!("    {line}")));
    e.lines.push("}".into());
    seen.extend(
        yes_arm
            .0
            .into_iter()
            .chain(no_arm.0)
            .map(|(block, _)| block),
    );
    seen.insert(merge);
    Ok(Some(next_edge.target()))
}

// Single-placement forward regions keep short-circuit shared tails canonical.
// Uninitialized local slots are declarations only; availability is proved from
// every original predecessor, independently of Java's definite assignment.
fn emit_region<R: ConstructorResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    entry: BlockId,
    aliases: &HashSet<ValueId>,
    seen: &mut HashSet<BlockId>,
) -> Result<Option<BlockId>, Unsupported> {
    let fail = || Unsupported::ControlFlow("unproved Java25 pre-init region".into());
    let cfg = e.ir.cfg();
    let Some(merge) = decomp_core::analysis::post_dominators(cfg).immediate_post_dominator(entry)
    else {
        return Ok(None);
    };
    if !matches!(
        e.ir.instruction(*cfg.block(merge).map_err(|_| fail())?.payload()),
        Ok(ManagedInsn::Phi { .. })
    ) {
        return Ok(None);
    }
    let mut cluster = Vec::new();
    let mut exit = merge;
    loop {
        let node = cfg.block(exit).map_err(|_| fail())?;
        let ManagedInsn::Phi { result, inputs } =
            e.ir.instruction(*node.payload()).map_err(|_| fail())?
        else {
            break;
        };
        if cluster.len() >= 32 || node.outgoing().len() != 1 {
            return Err(fail());
        }
        cluster.push((exit, *result, inputs.clone()));
        let edge = cfg.edge(node.outgoing()[0]).map_err(|_| fail())?;
        if edge.kind() != &EdgeKind::Unconditional {
            return Err(fail());
        }
        exit = edge.target();
    }
    if cfg
        .block(entry)
        .map_err(|_| fail())?
        .incoming()
        .iter()
        .any(|edge| {
            !cfg.edge(*edge).is_ok_and(|edge| {
                edge.kind() == &EdgeKind::Unconditional && seen.contains(&edge.source())
            })
        })
    {
        return Err(fail());
    }
    let mut region = HashSet::new();
    let mut todo = vec![entry];
    while let Some(block) = todo.pop() {
        if block == merge {
            continue;
        }
        if (block != entry && seen.contains(&block)) || region.len() >= 128 {
            return Err(fail());
        }
        if !region.insert(block) {
            continue;
        }
        let node = cfg.block(block).map_err(|_| fail())?;
        if node.outgoing().is_empty() {
            return Err(fail());
        }
        for edge in node.outgoing() {
            let edge = cfg.edge(*edge).map_err(|_| fail())?;
            if !matches!(
                edge.kind(),
                EdgeKind::Unconditional | EdgeKind::BranchTrue | EdgeKind::BranchFalse
            ) {
                return Err(fail());
            }
            todo.push(edge.target());
        }
    }
    region.extend(cluster.iter().map(|(b, _, _)| *b));
    for &block in &region {
        if block != entry
            && cfg
                .block(block)
                .map_err(|_| fail())?
                .incoming()
                .iter()
                .any(|edge| {
                    !cfg.edge(*edge)
                        .is_ok_and(|edge| region.contains(&edge.source()))
                })
        {
            return Err(fail());
        }
    }
    // Kahn order detects back edges and keeps each real block exactly once.
    let mut remaining = region.clone();
    let mut order = Vec::new();
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .copied()
            .filter(|block| {
                cfg.block(*block).is_ok_and(|node| {
                    node.incoming().iter().all(|edge| {
                        cfg.edge(*edge)
                            .is_ok_and(|edge| !remaining.contains(&edge.source()))
                    })
                })
            })
            .min()
            .ok_or_else(fail)?;
        remaining.remove(&next);
        order.push(next);
    }
    if order.first() != Some(&entry) {
        return Err(fail());
    }
    let initial: HashSet<_> = e.values.keys().copied().collect();
    let mut outputs: HashMap<BlockId, HashSet<ValueId>> = HashMap::new();
    let mut definitions = Vec::new();
    let cluster_inputs: HashMap<_, _> = cluster
        .iter()
        .map(|(b, v, inputs)| (*b, (*v, inputs)))
        .collect();
    let merge_edges = cfg.block(merge).map_err(|_| fail())?.incoming();
    for &block in &order {
        let node = cfg.block(block).map_err(|_| fail())?;
        let mut available = if block == entry {
            initial.clone()
        } else {
            let mut states = node.incoming().iter().map(|edge| {
                let source = cfg.edge(*edge).map_err(|_| fail())?.source();
                outputs.get(&source).cloned().ok_or_else(fail)
            });
            let mut common = states.next().ok_or_else(fail)??;
            for state in states {
                let state = state?;
                common.retain(|value| state.contains(value));
            }
            common
        };
        let insn = e.ir.instruction(*node.payload()).map_err(|_| fail())?;
        if let Some((result, inputs)) = cluster_inputs.get(&block) {
            if inputs.len() != merge_edges.len() {
                return Err(fail());
            }
            for edge in merge_edges {
                let selected: Vec<_> = inputs.iter().filter(|(id, _)| id == edge).collect();
                if selected.len() != 1
                    || aliases.contains(result) != aliases.contains(&selected[0].1)
                {
                    return Err(fail());
                }
                let source = cfg.edge(*edge).map_err(|_| fail())?.source();
                if !outputs
                    .get(&source)
                    .is_some_and(|state| state.contains(&selected[0].1))
                {
                    return Err(fail());
                }
            }
            available.insert(*result);
            if !aliases.contains(result) {
                definitions.push(*result);
            }
        } else {
            let (reads, result) = region_operands(insn).ok_or_else(fail)?;
            let receiver_copy = matches!(insn, ManagedInsn::Move { result, value }
                if aliases.contains(result) && aliases.contains(value));
            if reads.iter().any(|value| {
                !available.contains(value) || (aliases.contains(value) && !receiver_copy)
            }) {
                return Err(fail());
            }
            if let Some(result) = result {
                if initial.contains(&result) || (aliases.contains(&result) && !receiver_copy) {
                    return Err(fail());
                }
                if receiver_copy && e.ty(result)? != JavaType::Class(m.owner.clone()) {
                    return Err(fail());
                }
                available.insert(result);
                if !receiver_copy {
                    definitions.push(result);
                }
            }
        }
        outputs.insert(block, available);
    }
    for &(_, result, _) in &cluster {
        if aliases.contains(&result) {
            if e.ty(result)? != JavaType::Class(m.owner.clone()) {
                return Err(fail());
            }
            continue;
        }
        let ty = e.ty(result)?;
        let proof = crate::contextual_locals::checked_phi_plan(
            e.ir,
            result,
            &ty,
            &m.return_type,
            &crate::type_compatibility::Hierarchy::default(),
            e.resolver,
        )
        .map_err(|error| Unsupported::Type(format!("pre-init Phi family: {error:?}")))?;
        for value in proof.locals {
            if e.declarations.get(&value).is_some_and(|old| old != &ty) {
                return Err(fail());
            }
            e.declarations.insert(value, ty.clone());
        }
    }
    for value in definitions {
        let ty = e.ty(value)?;
        let mut name = format!("v{}", e.next);
        e.next += 1;
        while e.reserved.contains(&name) {
            name = format!("v{}", e.next);
            e.next += 1;
        }
        e.reserved.insert(name.clone());
        e.lines.push(format!("{} {name};", ty.source()?));
        e.values.insert(value, (name, ty));
        e.hoisted.insert(value);
    }
    let start = e.lines.len();
    let prefix = format!("preInitRegion{}", e.next);
    e.next += 1;
    let index: HashMap<_, _> = order.iter().enumerate().map(|(i, b)| (*b, i)).collect();
    for i in (1..order.len()).rev() {
        e.lines.push(format!("{prefix}_{i}: {{"));
    }
    for (i, &block) in order.iter().enumerate() {
        if i > 0 {
            e.lines.push("}".into());
        }
        let node = cfg.block(block).map_err(|_| fail())?;
        let insn = e.ir.instruction(*node.payload()).map_err(|_| fail())?;
        e.current = Some(*node.payload());
        if let ManagedInsn::Move { result, value } = insn
            && aliases.contains(result)
            && aliases.contains(value)
        {
            let binding = e.get(*value)?;
            if binding.0 != "this" || binding.1 != JavaType::Class(m.owner.clone()) {
                return Err(fail());
            }
            e.values.insert(*result, binding);
        } else if !matches!(insn, ManagedInsn::Phi { .. } | ManagedInsn::Branch { .. })
            && !source25::emit(e, insn, m, aliases)?
        {
            return Err(fail());
        }
        let edges = node.outgoing();
        let branch_condition = match insn {
            ManagedInsn::Branch {
                condition: Some(condition),
                ..
            } => {
                let (expression, ty) = e.get(*condition)?;
                if ty != JavaType::Boolean {
                    return Err(fail());
                }
                Some(expression)
            }
            _ => None,
        };
        let mut transfer = |edge_id: decomp_core::ids::EdgeId,
                            lines: &mut Vec<String>|
         -> Result<(), Unsupported> {
            let edge = cfg.edge(edge_id).map_err(|_| fail())?;
            if edge.target() == merge {
                let mut copies = Vec::new();
                for &(_, result, ref inputs) in &cluster {
                    if aliases.contains(&result) {
                        continue;
                    }
                    let value = inputs
                        .iter()
                        .find(|(id, _)| *id == edge_id)
                        .ok_or_else(fail)?
                        .1;
                    let (name, ty) = e.get(result)?;
                    let (expression, actual) = e.operand(value, &ty)?;
                    e.compatible(&actual, &ty)?;
                    let temp = format!("preInitEdge{}", e.next);
                    e.next += 1;
                    lines.push(format!("{} {temp} = {expression};", ty.source()?));
                    copies.push((name, temp));
                }
                for (name, temp) in copies {
                    lines.push(format!("{name} = {temp};"));
                }
            }
            if edge.target() != exit {
                let target = *index.get(&edge.target()).ok_or_else(fail)?;
                if target <= i {
                    return Err(fail());
                }
                lines.push(format!("break {prefix}_{target};"));
            }
            Ok(())
        };
        let mut lines = Vec::new();
        match insn {
            ManagedInsn::Branch {
                condition: Some(_),
                then_edge,
                else_edge: Some(other),
            } => {
                if edges.len() != 2
                    || !edges.contains(then_edge)
                    || !edges.contains(other)
                    || cfg.edge(*then_edge).map_err(|_| fail())?.kind() != &EdgeKind::BranchTrue
                    || cfg.edge(*other).map_err(|_| fail())?.kind() != &EdgeKind::BranchFalse
                {
                    return Err(fail());
                }
                let condition = branch_condition.as_ref().ok_or_else(fail)?;
                lines.push(format!("if ({condition}) {{"));
                transfer(*then_edge, &mut lines)?;
                lines.push("} else {".into());
                transfer(*other, &mut lines)?;
                lines.push("}".into());
            }
            _ if edges.len() == 1
                && cfg.edge(edges[0]).map_err(|_| fail())?.kind() == &EdgeKind::Unconditional =>
            {
                transfer(edges[0], &mut lines)?
            }
            _ => return Err(fail()),
        }
        e.lines.extend(lines);
    }
    if e.lines.len() == start {
        return Err(fail());
    }
    let final_available = outputs
        .get(order.last().ok_or_else(fail)?)
        .ok_or_else(fail)?;
    e.values.retain(|value, _| final_available.contains(value));
    for &(_, result, _) in &cluster {
        if aliases.contains(&result) {
            e.values
                .insert(result, ("this".into(), JavaType::Class(m.owner.clone())));
        }
    }
    seen.extend(region);
    Ok(Some(exit))
}

fn region_operands(insn: &ManagedInsn) -> Option<(Vec<ValueId>, Option<ValueId>)> {
    use ManagedInsn::*;
    Some(match insn {
        Const { result, .. } => (vec![], Some(*result)),
        Move { result, value }
        | Neg { result, value }
        | Not { result, value }
        | Cast { result, value, .. }
        | InstanceOf { result, value, .. } => (vec![*value], Some(*result)),
        Arithmetic {
            result, lhs, rhs, ..
        }
        | Compare {
            result, lhs, rhs, ..
        }
        | OrderingCompare {
            result, lhs, rhs, ..
        } => (vec![*lhs, *rhs], Some(*result)),
        FieldGet { result, object, .. } => (object.iter().copied().collect(), Some(*result)),
        Invoke {
            kind: InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface,
            result,
            args,
            ..
        } => (args.clone(), *result),
        Branch { condition, .. } => (condition.iter().copied().collect(), None),
        _ => return None,
    })
}
