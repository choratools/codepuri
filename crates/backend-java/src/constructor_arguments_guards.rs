//! Java25 zero-Phi pre-chain guards. Calls stay inside their original arm and
//! remain ordinary calls: a declaration is never treated as a no-return oracle.
use super::*;

pub(super) fn emit<R: ConstructorResolver>(
    e: &mut Emitter<'_, R>,
    signature: &JavaMethod,
    branch: BlockId,
    aliases: &HashSet<ValueId>,
    seen: &mut HashSet<BlockId>,
) -> Result<Option<BlockId>, Unsupported> {
    let fail = || Unsupported::ControlFlow("unproved Java25 pre-chain guard".into());
    let cfg = e.ir.cfg();
    if cfg.block_count() > 256 {
        return Ok(None);
    }
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
        || then_edge == else_edge
        || !node.outgoing().contains(then_edge)
        || !node.outgoing().contains(else_edge)
    {
        return Err(fail());
    }
    for (edge, kind) in [
        (*then_edge, EdgeKind::BranchTrue),
        (*else_edge, EdgeKind::BranchFalse),
    ] {
        let edge = cfg.edge(edge).map_err(|_| fail())?;
        if edge.source() != branch || edge.kind() != &kind {
            return Err(fail());
        }
    }
    let Some(merge) = decomp_core::analysis::post_dominators(cfg).immediate_post_dominator(branch)
    else {
        return Ok(None);
    };
    if cfg.block(merge).map_err(|_| fail())?.incoming().len() != 2 || seen.contains(&merge) {
        return Ok(None);
    }
    if matches!(
        e.ir.instruction(*cfg.block(merge).map_err(|_| fail())?.payload()),
        Ok(ManagedInsn::Phi { .. })
    ) {
        return Ok(None);
    }
    let path = |first| -> Option<Vec<(BlockId, InsnId)>> {
        let mut cursor = cfg.edge(first).ok()?.target();
        let mut visited = HashSet::new();
        let mut steps = Vec::new();
        for _ in 0..128 {
            if cursor == merge {
                return Some(steps);
            }
            if seen.contains(&cursor) || !visited.insert(cursor) {
                return None;
            }
            let node = cfg.block(cursor).ok()?;
            if node.incoming().len() != 1 || node.outgoing().len() != 1 {
                return None;
            }
            let id = *node.payload();
            // Arm-local values do not escape: the zero-Phi continuation restores
            // precisely the bindings available before the branch.
            if !matches!(
                e.ir.instruction(id).ok()?,
                ManagedInsn::Const { .. }
                    | ManagedInsn::Move { .. }
                    | ManagedInsn::Invoke {
                        kind: InvokeKind::Static,
                        result: None,
                        ..
                    }
                    | ManagedInsn::Branch {
                        condition: None,
                        ..
                    }
            ) {
                return None;
            }
            steps.push((cursor, id));
            let edge = cfg.edge(node.outgoing()[0]).ok()?;
            if edge.kind() != &EdgeKind::Unconditional {
                return None;
            }
            cursor = edge.target();
        }
        None
    };
    let (Some(yes), Some(no)) = (path(*then_edge), path(*else_edge)) else {
        return Ok(None);
    };
    if yes
        .iter()
        .any(|(b, _)| no.iter().any(|(other, _)| b == other))
    {
        return Err(fail());
    }
    if aliases.contains(condition) {
        return Err(fail());
    }
    let (condition, ty) = e.get(*condition)?;
    if ty != JavaType::Boolean {
        return Err(fail());
    }
    let checkpoint = (
        e.values.clone(),
        e.lines.clone(),
        e.next,
        e.reserved.clone(),
        e.current,
        e.hoisted.clone(),
        e.declarations.clone(),
    );
    let result = (|| {
        let mut arms = Vec::new();
        for steps in [&yes, &no] {
            e.values = checkpoint.0.clone();
            let start = e.lines.len();
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
                    return Err(fail());
                }
            }
            arms.push(e.lines.drain(start..).collect::<Vec<_>>());
        }
        Ok(arms)
    })();
    e.values = checkpoint.0;
    e.current = checkpoint.4;
    let arms = match result {
        Ok(arms) => arms,
        Err(error) => {
            e.lines = checkpoint.1;
            e.next = checkpoint.2;
            e.reserved = checkpoint.3;
            e.hoisted = checkpoint.5;
            e.declarations = checkpoint.6;
            return Err(error);
        }
    };
    e.lines.push(format!("if ({condition}) {{"));
    e.lines
        .extend(arms[0].iter().map(|line| format!("    {line}")));
    if !arms[1].is_empty() {
        e.lines.push("} else {".into());
        e.lines
            .extend(arms[1].iter().map(|line| format!("    {line}")));
    }
    e.lines.push("}".into());
    seen.extend(yes.into_iter().chain(no).map(|(b, _)| b));
    Ok(Some(merge))
}
