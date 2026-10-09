//! Root-dominated argument diamonds independent of unrelated method shared tails.
use super::*;
use decomp_core::ids::{BlockId, EdgeId};

pub(super) fn steps<R: JavaResolver>(
    emitter: &Emitter<'_, R>,
    root: InsnId,
    initializer: InsnId,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<Vec<Step>, Unsupported> {
    let proof = branching::plan_forward_checked(emitter.ir, bindings, declarations)
        .map_err(|_| denied())?;
    let cfg = emitter.ir.cfg();
    let find = |insn| emitter.object_block_id(insn).ok_or_else(denied);
    let start = find(root)?;
    let end = find(initializer)?;
    let dom = emitter.object_dominance();
    let post = emitter.object_post_dominance();
    if !post.post_dominates(end, start) {
        return Err(denied());
    }
    let mut local = HashSet::new();
    let mut pending = vec![start];
    while let Some(block) = pending.pop() {
        if !local.insert(block) {
            continue;
        }
        if local.len() > 256 || !dom.dominates(start, block) {
            return Err(denied());
        }
        if block == end {
            continue;
        }
        let node = cfg.block(block).map_err(|_| denied())?;
        if node.outgoing().is_empty() {
            return Err(denied());
        }
        for edge in node.outgoing() {
            let edge = cfg.edge(*edge).map_err(|_| denied())?;
            if matches!(edge.kind(), EdgeKind::Exception(_)) {
                return Err(denied());
            }
            pending.push(edge.target());
        }
    }
    struct Walker<'a> {
        ir: &'a MethodIr<SsaForm>,
        local: &'a HashSet<BlockId>,
        post: &'a decomp_core::analysis::PostDominators,
        copies: &'a HashMap<EdgeId, Vec<branching::Copy>>,
        placed: HashSet<BlockId>,
        root: BlockId,
    }
    impl Walker<'_> {
        fn edge(&self, edge: EdgeId, out: &mut Vec<Step>) {
            if let Some(copies) = self.copies.get(&edge) {
                out.push(Step::Copies {
                    edge,
                    copies: copies.clone(),
                });
            }
        }
        fn walk(&mut self, mut block: BlockId, stop: BlockId) -> Result<Vec<Step>, Unsupported> {
            let mut out = vec![];
            while block != stop {
                if !self.local.contains(&block) || !self.placed.insert(block) {
                    return Err(denied());
                }
                let node = self.ir.cfg().block(block).map_err(|_| denied())?;
                let id = *node.payload();
                let insn = self.ir.instruction(id).map_err(|_| denied())?;
                match insn {
                    ManagedInsn::Branch {
                        condition: Some(condition),
                        then_edge,
                        else_edge: Some(other),
                    } => {
                        let join = self
                            .post
                            .immediate_post_dominator(block)
                            .ok_or_else(denied)?;
                        if join == block
                            || !self.local.contains(&join)
                            || !self.post.post_dominates(stop, join)
                        {
                            return Err(denied());
                        }
                        let yes = self.ir.cfg().edge(*then_edge).map_err(|_| denied())?;
                        let no = self.ir.cfg().edge(*other).map_err(|_| denied())?;
                        if yes.source() != block
                            || no.source() != block
                            || yes.kind() != &EdgeKind::BranchTrue
                            || no.kind() != &EdgeKind::BranchFalse
                            || node.outgoing().len() != 2
                        {
                            return Err(denied());
                        }
                        let mut then_steps = vec![];
                        self.edge(*then_edge, &mut then_steps);
                        then_steps.extend(self.walk(yes.target(), join)?);
                        let mut else_steps = vec![];
                        self.edge(*other, &mut else_steps);
                        else_steps.extend(self.walk(no.target(), join)?);
                        out.push(Step::If {
                            condition: *condition,
                            then_steps,
                            else_steps,
                        });
                        block = join;
                    }
                    ManagedInsn::Return { .. }
                    | ManagedInsn::Throw { .. }
                    | ManagedInsn::Switch { .. } => return Err(denied()),
                    _ => {
                        if !matches!(
                            insn,
                            ManagedInsn::Phi { .. }
                                | ManagedInsn::Branch {
                                    condition: None,
                                    ..
                                }
                        ) && block != self.root
                        {
                            out.push(Step::Instruction(id));
                        }
                        let [edge] = node.outgoing() else {
                            return Err(denied());
                        };
                        let e = self.ir.cfg().edge(*edge).map_err(|_| denied())?;
                        if e.kind() != &EdgeKind::Unconditional {
                            return Err(denied());
                        }
                        self.edge(*edge, &mut out);
                        block = e.target();
                    }
                }
            }
            Ok(out)
        }
    }
    let mut walker = Walker {
        ir: emitter.ir,
        local: &local,
        post,
        copies: &proof.copies,
        placed: HashSet::new(),
        root: start,
    };
    let steps = walker.walk(start, end)?;
    if walker.placed.len() + 1 != local.len() {
        return Err(denied());
    }
    Ok(steps)
}
