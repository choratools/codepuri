//! Opt-in loop control grammar. Export with `pub mod loop_control` in the parent.
//! Conditions remain at their original blocks; transfers retain original edges.
use crate::{EdgeProvenance, StructureError};
use decomp_core::{
    analysis::{dominators, natural_loops},
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
/// Lossless control tree with conditions and abrupt edges at original positions.
pub enum ControlRegion {
    /// Ordinary, Phi, unconditional branch, or terminal block.
    Block(BlockId),
    /// Ordered execution; empty sequences represent empty conditional arms.
    Sequence(Vec<Self>),
    /// Conditional block, with original true and false edge identities.
    If {
        /// Original conditional branch block; emitted only here.
        header: BlockId,
        /// Selected true edge, including its Phi transfers.
        then_edge: EdgeId,
        /// True arm after traversing the selected edge.
        then_region: Box<Self>,
        /// Selected false edge, including its Phi transfers.
        else_edge: EdgeId,
        /// False arm after traversing the selected edge.
        else_region: Box<Self>,
    },
    /// General loop: tests are inside the body, never hoisted. A posttest
    /// loop therefore executes its body before reaching its first test.
    Loop {
        /// Single natural-loop entry and stable label identity.
        header: BlockId,
        /// All loop blocks, including evaluation, tests, and terminal arms.
        body: Box<Self>,
        /// Shared continuation, emitted by the enclosing scope.
        exit: Option<BlockId>,
    },
    /// The source block/conditional already owns execution of this edge.
    Transfer {
        /// Already traversed source edge; copies belong before this transfer.
        edge: EdgeId,
        /// Abrupt control destination.
        kind: TransferKind,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit labeled transfer to a natural-loop boundary.
pub enum TransferKind {
    /// Leave this loop through its shared continuation.
    Break {
        /// Owning loop's header identity.
        header: BlockId,
    },
    /// Return to this loop's original entry, after original edge copies.
    Continue {
        /// Owning loop's header identity.
        header: BlockId,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Complete graph snapshot for canonical-plan validation.
pub struct ControlCfg {
    /// Original entry identity.
    pub entry: BlockId,
    /// Original graph revision.
    pub revision: u64,
    /// Every original live block exactly once.
    pub root: ControlRegion,
    /// Every original edge in iteration order, without deduplication.
    pub edges: Vec<EdgeProvenance>,
}

struct Loop {
    members: HashSet<BlockId>,
    exit: Option<BlockId>,
}
struct Builder<'a, T> {
    cfg: &'a Cfg<T>,
    loops: HashMap<BlockId, Loop>,
    seen: HashSet<BlockId>,
}

/// Single-entry reducible loops with one continuation exit, terminal arms,
/// nested loops, early breaks, and conditional backedges. Multiple unrelated
/// continuation exits and irreducible cycles are explicitly rejected.
pub fn structure<T>(cfg: &Cfg<T>) -> Result<ControlCfg, StructureError> {
    cfg.validate()?;
    let mut reachable = HashSet::new();
    let mut todo = vec![cfg.entry()];
    while let Some(b) = todo.pop() {
        if reachable.insert(b) {
            for &e in cfg.block(b)?.outgoing() {
                todo.push(cfg.edge(e)?.target());
            }
        }
    }
    let missing: Vec<_> = cfg.block_ids().filter(|b| !reachable.contains(b)).collect();
    if !missing.is_empty() {
        return Err(StructureError::UnreachableBlocks(missing));
    }
    for b in cfg.block_ids() {
        outgoing(cfg, b)?;
    }
    let mut grouped: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();
    for lp in natural_loops(cfg, &dominators(cfg)) {
        grouped.entry(lp.header()).or_default().extend(lp.blocks());
    }
    let mut loops = HashMap::new();
    for h in cfg.block_ids().filter(|h| grouped.contains_key(h)) {
        let mut members = grouped[&h].clone();
        // Absorb private terminal-only arms. A shared terminal stays outside:
        // it can be the common loop continuation but must never be duplicated.
        let mut candidates = HashSet::new();
        for b in cfg.block_ids().filter(|b| members.contains(b)) {
            for &e in cfg.block(b)?.outgoing() {
                let t = cfg.edge(e)?.target();
                if !members.contains(&t) {
                    candidates.insert(t);
                }
            }
        }
        // Reserve the pretest exit (after any linear evaluation prefix), or
        // the posttest latch exit. Private abrupt arms may be absorbed, but
        // the common continuation must remain outside the loop.
        let mut evaluation = h;
        let mut prefix = HashSet::new();
        while prefix.insert(evaluation) {
            let es = outgoing(cfg, evaluation)?;
            if es.len() != 1 {
                break;
            }
            let next = cfg.edge(es[0])?.target();
            if !members.contains(&next) || next == h {
                break;
            }
            evaluation = next;
        }
        let mut reserved = None;
        for e in outgoing(cfg, evaluation)? {
            let target = cfg.edge(e)?.target();
            if !members.contains(&target) {
                reserved = Some(target);
            }
        }
        if reserved.is_none() {
            let mut latch_exits = Vec::new();
            for b in cfg.block_ids().filter(|b| members.contains(b)) {
                let es = outgoing(cfg, b)?;
                if es.len() == 2
                    && es
                        .iter()
                        .any(|&e| cfg.edge(e).is_ok_and(|e| e.target() == h))
                {
                    for e in es {
                        let t = cfg.edge(e)?.target();
                        if !members.contains(&t) {
                            latch_exits.push(t);
                        }
                    }
                }
            }
            // Distinct retry latches may leave via private linear bridges to
            // the same continuation. Reserve their first common successor,
            // rather than whichever latch a randomized set visits last.
            let mut common: Option<HashSet<BlockId>> = None;
            for start in latch_exits {
                let mut path = HashSet::new();
                let mut b = start;
                while !members.contains(&b) && path.insert(b) {
                    let es = outgoing(cfg, b)?;
                    if es.len() != 1 {
                        break;
                    }
                    b = cfg.edge(es[0])?.target();
                }
                if let Some(common) = &mut common {
                    common.retain(|b| path.contains(b));
                } else {
                    common = Some(path);
                }
            }
            reserved = common.and_then(|set| cfg.block_ids().find(|b| set.contains(b)));
        }
        if candidates.len() == 1 {
            reserved = candidates.iter().copied().next();
        }
        let mut terminal_sets = Vec::new();
        for start in cfg.block_ids().filter(|b| candidates.contains(b)) {
            if Some(start) == reserved {
                continue;
            }
            let mut visited = HashSet::new();
            let mut pending = vec![start];
            let mut valid = true;
            while let Some(b) = pending.pop() {
                if Some(b) == reserved {
                    continue;
                }
                if members.contains(&b) {
                    valid = false;
                    break;
                }
                if visited.insert(b) {
                    for &e in cfg.block(b)?.outgoing() {
                        pending.push(cfg.edge(e)?.target());
                    }
                }
            }
            // Cyclic external arms are not terminal-only.
            if valid
                && dag(cfg, &visited, &HashMap::new(), None).is_ok()
                && visited.iter().all(|&b| {
                    cfg.block(b).is_ok_and(|block| {
                        block.incoming().iter().all(|&e| {
                            cfg.edge(e).is_ok_and(|edge| {
                                members.contains(&edge.source()) || visited.contains(&edge.source())
                            })
                        })
                    })
                })
            {
                terminal_sets.push(visited);
            }
        }
        // A unique external target is the continuation, even when it returns.
        if candidates.len() > 1 {
            for set in terminal_sets {
                members.extend(set);
            }
        }
        let mut exits = HashSet::new();
        for b in cfg.block_ids().filter(|b| members.contains(b)) {
            for &e in cfg.block(b)?.outgoing() {
                let t = cfg.edge(e)?.target();
                if !members.contains(&t) {
                    exits.insert(t);
                }
            }
        }
        if exits.len() > 1 {
            return Err(unsupported(cfg));
        }
        for b in cfg.block_ids().filter(|b| members.contains(b)) {
            for &e in cfg.block(b)?.incoming() {
                if b != h && !members.contains(&cfg.edge(e)?.source()) {
                    return Err(unsupported(cfg));
                }
            }
        }
        loops.insert(
            h,
            Loop {
                members,
                exit: exits.into_iter().next(),
            },
        );
    }
    let values: Vec<_> = cfg.block_ids().filter_map(|b| loops.get(&b)).collect();
    for (i, a) in values.iter().enumerate() {
        for b in &values[i + 1..] {
            if !a.members.is_disjoint(&b.members)
                && !a.members.is_subset(&b.members)
                && !b.members.is_subset(&a.members)
            {
                return Err(unsupported(cfg));
            }
        }
    }
    let mut builder = Builder {
        cfg,
        loops,
        seen: HashSet::new(),
    };
    let root = builder.scope(cfg.entry(), &reachable, None)?;
    let missing: Vec<_> = cfg
        .block_ids()
        .filter(|b| !builder.seen.contains(b))
        .collect();
    if !missing.is_empty() {
        return Err(StructureError::IncompleteCoverage(missing));
    }
    let edges = cfg
        .edge_ids()
        .map(|id| {
            let e = cfg.edge(id)?;
            Ok(EdgeProvenance {
                id,
                source: e.source(),
                target: e.target(),
                kind: e.kind().clone(),
            })
        })
        .collect::<Result<_, decomp_core::cfg::CfgError>>()?;
    Ok(ControlCfg {
        entry: cfg.entry(),
        revision: cfg.revision(),
        root,
        edges,
    })
}
fn unsupported<T>(cfg: &Cfg<T>) -> StructureError {
    StructureError::CyclicControlFlow(cfg.block_ids().collect())
}
fn outgoing<T>(cfg: &Cfg<T>, b: BlockId) -> Result<Vec<EdgeId>, StructureError> {
    let es = cfg.block(b)?.outgoing();
    for &e in es {
        if matches!(
            cfg.edge(e)?.kind(),
            EdgeKind::Switch(_) | EdgeKind::Exception(_)
        ) {
            return Err(StructureError::UnsupportedEdge {
                edge: e,
                kind: cfg.edge(e)?.kind().clone(),
            });
        }
    }
    match es {
        [] => Ok(vec![]),
        [e] if cfg.edge(*e)?.kind() == &EdgeKind::Unconditional => Ok(vec![*e]),
        [a, c] => match (cfg.edge(*a)?.kind(), cfg.edge(*c)?.kind()) {
            (EdgeKind::BranchTrue, EdgeKind::BranchFalse) => Ok(vec![*a, *c]),
            (EdgeKind::BranchFalse, EdgeKind::BranchTrue) => Ok(vec![*c, *a]),
            _ => Err(StructureError::UnsupportedBranch {
                block: b,
                edges: es.to_vec(),
            }),
        },
        _ => Err(StructureError::UnsupportedBranch {
            block: b,
            edges: es.to_vec(),
        }),
    }
}
// A topological order of the scope after child-loop contraction and removal
// of explicit transfer edges. Any remaining cycle is unsupported.
type Dag = (Vec<BlockId>, HashMap<BlockId, Vec<BlockId>>);
fn dag<T>(
    cfg: &Cfg<T>,
    nodes: &HashSet<BlockId>,
    children: &HashMap<BlockId, Option<BlockId>>,
    boundary: Option<BlockId>,
) -> Result<Dag, StructureError> {
    let mut graph = HashMap::new();
    for b in cfg.block_ids().filter(|b| nodes.contains(b)) {
        let targets = if let Some(exit) = children.get(&b) {
            exit.iter().copied().collect()
        } else {
            outgoing(cfg, b)?
                .iter()
                .map(|&e| cfg.edge(e).map(|e| e.target()))
                .collect::<Result<Vec<_>, _>>()?
        };
        graph.insert(b, targets);
    }
    let mut degree: HashMap<_, usize> = nodes.iter().map(|&b| (b, 0)).collect();
    for ts in graph.values() {
        for t in ts {
            if Some(*t) != boundary
                && let Some(d) = degree.get_mut(t)
            {
                *d += 1;
            }
        }
    }
    let mut ready: Vec<_> = cfg
        .block_ids()
        .filter(|b| degree.get(b) == Some(&0))
        .collect();
    let mut order = Vec::new();
    while let Some(b) = ready.pop() {
        order.push(b);
        for t in &graph[&b] {
            if Some(*t) != boundary
                && let Some(d) = degree.get_mut(t)
            {
                *d -= 1;
                if *d == 0 {
                    ready.push(*t);
                }
            }
        }
    }
    if order.len() != nodes.len() {
        return Err(unsupported(cfg));
    }
    Ok((order, graph))
}
impl<T> Builder<'_, T> {
    fn scope(
        &mut self,
        start: BlockId,
        members: &HashSet<BlockId>,
        active: Option<BlockId>,
    ) -> Result<ControlRegion, StructureError> {
        let mut nodes = members.clone();
        let mut children = HashMap::new();
        for h in self.cfg.block_ids() {
            let Some(lp) = self.loops.get(&h) else {
                continue;
            };
            if Some(h) != active
                && lp.members.is_subset(members)
                && !self.loops.iter().any(|(&other, p)| {
                    other != h
                        && Some(other) != active
                        && p.members.is_subset(members)
                        && lp.members.is_subset(&p.members)
                })
            {
                for &b in &lp.members {
                    nodes.remove(&b);
                }
                nodes.insert(h);
                children.insert(h, lp.exit);
            }
        }
        let (order, graph) = dag(self.cfg, &nodes, &children, active)?;
        let mut pd: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();
        let mut joins = HashMap::new();
        for &b in order.iter().rev() {
            let mut sets = graph[&b].iter().map(|t| {
                if Some(*t) == active {
                    HashSet::new()
                } else {
                    pd.get(t).cloned().unwrap_or_default()
                }
            });
            let mut common = sets.next().unwrap_or_default();
            for s in sets {
                common.retain(|t| s.contains(t));
            }
            joins.insert(b, order.iter().copied().find(|t| common.contains(t)));
            common.insert(b);
            pd.insert(b, common);
        }
        self.walk(start, None, active, &nodes, &children, &joins)
    }
    fn transfer(
        &self,
        edge: EdgeId,
        active: Option<BlockId>,
    ) -> Result<Option<ControlRegion>, StructureError> {
        if let Some(h) = active {
            let target = self.cfg.edge(edge)?.target();
            let kind = if target == h {
                Some(TransferKind::Continue { header: h })
            } else if Some(target) == self.loops[&h].exit {
                Some(TransferKind::Break { header: h })
            } else {
                None
            };
            return Ok(kind.map(|kind| ControlRegion::Transfer { edge, kind }));
        }
        Ok(None)
    }
    fn arm(
        &mut self,
        e: EdgeId,
        stop: Option<BlockId>,
        active: Option<BlockId>,
        nodes: &HashSet<BlockId>,
        children: &HashMap<BlockId, Option<BlockId>>,
        joins: &HashMap<BlockId, Option<BlockId>>,
    ) -> Result<ControlRegion, StructureError> {
        if let Some(r) = self.transfer(e, active)? {
            return Ok(r);
        }
        self.walk(
            self.cfg.edge(e)?.target(),
            stop,
            active,
            nodes,
            children,
            joins,
        )
    }
    fn walk(
        &mut self,
        start: BlockId,
        stop: Option<BlockId>,
        active: Option<BlockId>,
        nodes: &HashSet<BlockId>,
        children: &HashMap<BlockId, Option<BlockId>>,
        joins: &HashMap<BlockId, Option<BlockId>>,
    ) -> Result<ControlRegion, StructureError> {
        let mut current = Some(start);
        let mut rs = Vec::new();
        while let Some(b) = current {
            if Some(b) == stop {
                break;
            }
            if !nodes.contains(&b) {
                return Err(unsupported(self.cfg));
            }
            if children.contains_key(&b) {
                let lp = &self.loops[&b];
                let members = lp.members.clone();
                let exit = lp.exit;
                let body = self.scope(b, &members, Some(b))?;
                rs.push(ControlRegion::Loop {
                    header: b,
                    body: Box::new(body),
                    exit,
                });
                // A child's normal exit can be the active parent's header.
                // The child's original break edge already owns that transfer
                // (including Phi copies); end this parent-body scope rather
                // than placing/evaluating its header a second time.
                current = exit.filter(|target| Some(*target) != active);
                continue;
            }
            if !self.seen.insert(b) {
                return Err(StructureError::OverlappingRegion(b));
            }
            let es = outgoing(self.cfg, b)?;
            match es.as_slice() {
                [] => {
                    rs.push(ControlRegion::Block(b));
                    current = None;
                }
                [e] => {
                    rs.push(ControlRegion::Block(b));
                    if let Some(r) = self.transfer(*e, active)? {
                        rs.push(r);
                        current = None;
                    } else {
                        current = Some(self.cfg.edge(*e)?.target());
                    }
                }
                [yes, no] => {
                    let join = joins[&b];
                    let a = self.arm(*yes, join, active, nodes, children, joins)?;
                    let c = self.arm(*no, join, active, nodes, children, joins)?;
                    rs.push(ControlRegion::If {
                        header: b,
                        then_edge: *yes,
                        then_region: Box::new(a),
                        else_edge: *no,
                        else_region: Box::new(c),
                    });
                    current = join;
                }
                _ => unreachable!(),
            }
        }
        Ok(ControlRegion::Sequence(rs))
    }
}
