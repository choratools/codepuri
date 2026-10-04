//! Lossless acyclic switch structuring over generic CFGs.
//!
//! Switch labels sharing a target are grouped. A single unconditional edge in
//! a switch is interpreted as its default arm; no default is invented when it
//! is absent. Fallthrough/overlapping arms are rejected rather than duplicated.
//! Exceptions and all cyclic flow (including irreducible flow) are explicitly
//! unsupported. Exception reconstruction is deferred.

use crate::{EdgeProvenance, StructureError};
use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::BlockId;
use std::collections::{HashMap, HashSet, VecDeque};

/// Acyclic forward-control region, used when a shared tail has no single tree
/// owner. This is explicitly distinct from a strict SESE region tree.
///
/// Each real block occurs once in topological order. Every transfer selects an
/// original edge; there are no virtual block copies and no implicit fallthrough.
/// A renderer can use nested forward labels (no dispatch loop or state variable).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardRegions {
    /// Original entry.
    pub entry: BlockId,
    /// Original graph revision.
    pub revision: u64,
    /// Single-placement block schedule.
    pub blocks: Vec<ForwardBlock>,
    /// Original edges, including parallel edges, in arena order.
    pub edges: Vec<EdgeProvenance>,
}

/// One real block and its explicit normal control transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardBlock {
    /// Original block whose non-control instructions execute here.
    pub block: BlockId,
    /// Postdominator information, not a promise of dominance or strict SESE.
    pub postdominator: Option<BlockId>,
    /// Original outgoing edges in adjacency order. Empty means terminal.
    pub outgoing: Vec<decomp_core::ids::EdgeId>,
}

/// Result of opt-in, flow-aware acyclic structuring.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlowStructuredCfg {
    /// Existing accepted trees are returned verbatim.
    Strict(ExtendedStructuredCfg),
    /// Shared-return/tail ownership requires explicit forward transfers.
    Shared(ForwardRegions),
}

/// Preserve the strict API on all previously accepted graphs, then factor
/// overlapping acyclic regions into single-placement forward regions.
///
/// This never duplicates payloads, assumes purity, changes edge identity, or
/// claims an irreducible graph is structured. Exception edges remain typed
/// unsupported errors; exception reconstruction belongs to its owning layer.
/// Construction is O((B+E) log B), excluding core validation, with no reachability
/// matrix, path enumeration, or transitive node sets.
pub fn structure_flow<T>(cfg: &Cfg<T>) -> Result<FlowStructuredCfg, StructureError> {
    match structure_extended(cfg) {
        Ok(strict) => return Ok(FlowStructuredCfg::Strict(strict)),
        Err(StructureError::OverlappingRegion(_)) => {}
        Err(error) => return Err(error),
    }
    // The strict pass has already validated reachability, edge semantics,
    // branch shapes and acyclicity. Rebuild a linear-size immutable schedule.
    let blocks: Vec<_> = cfg.block_ids().collect();
    let mut indegree = HashMap::with_capacity(blocks.len());
    let mut successors = HashMap::with_capacity(blocks.len());
    for &block in &blocks {
        indegree.insert(block, cfg.block(block)?.incoming().len());
        successors.insert(
            block,
            cfg.block(block)?
                .outgoing()
                .iter()
                .map(|&id| cfg.edge(id).map(|edge| edge.target()))
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    let mut ready: VecDeque<_> = blocks
        .iter()
        .copied()
        .filter(|block| indegree[block] == 0)
        .collect();
    let mut order = Vec::with_capacity(blocks.len());
    while let Some(block) = ready.pop_front() {
        order.push(block);
        for target in &successors[&block] {
            let Some(degree) = indegree.get_mut(target) else {
                return Err(StructureError::InvalidCfg(
                    decomp_core::cfg::CfgError::MissingBlock(*target),
                ));
            };
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(*target);
            }
        }
    }
    if order.len() != blocks.len() {
        return Err(StructureError::CyclicControlFlow(
            blocks
                .into_iter()
                .filter(|block| indegree[block] != 0)
                .collect(),
        ));
    }
    let joins = crate::postdom::joins(&order, &successors);
    let mut scheduled = Vec::with_capacity(order.len());
    for block in order {
        scheduled.push(ForwardBlock {
            block,
            postdominator: joins[&block],
            outgoing: cfg.block(block)?.outgoing().to_vec(),
        });
    }
    let edges = cfg
        .edge_ids()
        .map(|id| {
            let edge = cfg.edge(id)?;
            Ok(EdgeProvenance {
                id,
                source: edge.source(),
                target: edge.target(),
                kind: edge.kind().clone(),
            })
        })
        .collect::<Result<Vec<_>, decomp_core::cfg::CfgError>>()?;
    Ok(FlowStructuredCfg::Shared(ForwardRegions {
        entry: cfg.entry(),
        revision: cfg.revision(),
        blocks: scheduled,
        edges,
    }))
}

/// Extended region grammar; headers occur only in their owning region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtendedRegion {
    /// Ordinary or terminal block.
    Block(BlockId),
    /// Ordered regions; empty sequences represent empty arms.
    Sequence(Vec<Self>),
    /// Conditional with polarity taken from edge kinds.
    If {
        /// Original conditional block.
        header: BlockId,
        /// Arm selected by `BranchTrue`.
        then_region: Box<Self>,
        /// Arm selected by `BranchFalse`.
        else_region: Box<Self>,
        /// Nearest common postdominator, emitted by the enclosing sequence.
        join: Option<BlockId>,
    },
    /// Integral dispatch, with the join excluded from every arm.
    Switch {
        /// Original dispatch block.
        header: BlockId,
        /// Distinct targets in first-seen adjacency order.
        arms: Vec<SwitchArm>,
        /// Nearest common postdominator, or separately terminating arms.
        join: Option<BlockId>,
    },
}

/// One distinct switch target, preserving labels in adjacency order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchArm {
    /// Original arm entry, possibly the join for an empty arm.
    pub target: BlockId,
    /// Integral values selecting this target.
    pub labels: Vec<u64>,
    /// Whether an original unconditional dispatch edge selects this target.
    pub is_default: bool,
    /// Arm contents, excluding the switch join.
    pub region: ExtendedRegion,
}

/// Complete immutable snapshot; original edge identities are never deduplicated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtendedStructuredCfg {
    /// Original entry block.
    pub entry: BlockId,
    /// Original revision, not a globally unique graph identity.
    pub revision: u64,
    /// All live blocks exactly once.
    pub root: ExtendedRegion,
    /// Every original edge in CFG iteration order.
    pub edges: Vec<EdgeProvenance>,
}

#[derive(Clone)]
enum Shape {
    Plain,
    If,
    Switch(Vec<(BlockId, Vec<u64>, bool)>),
}

type ArmTarget = (BlockId, Vec<u64>, bool);
type TargetIndex = fn(&mut Vec<ArmTarget>, &mut HashMap<BlockId, usize>, BlockId) -> usize;

fn indexed_target(
    arms: &mut Vec<ArmTarget>,
    index: &mut HashMap<BlockId, usize>,
    target: BlockId,
) -> usize {
    *index.entry(target).or_insert_with(|| {
        arms.push((target, Vec::new(), false));
        arms.len() - 1
    })
}

#[cfg(test)]
fn original_target(
    arms: &mut Vec<ArmTarget>,
    _: &mut HashMap<BlockId, usize>,
    target: BlockId,
) -> usize {
    if let Some(i) = arms.iter().position(|a| a.0 == target) {
        i
    } else {
        arms.push((target, Vec::new(), false));
        arms.len() - 1
    }
}

/// Structure chains, conditionals and non-overlapping acyclic switches.
///
/// Returns existing typed `StructureError`s, including `UnsupportedEdge` for
/// exceptions and `CyclicControlFlow` for reducible or irreducible cycles.
/// Requires no payload traits and does not mutate the CFG. DAG postdominators
/// take O((B+E) log B) time and O(B log B) space, plus CFG validation.
pub fn structure_extended<T>(cfg: &Cfg<T>) -> Result<ExtendedStructuredCfg, StructureError> {
    structure_using(cfg, crate::postdom::joins, build, indexed_target)
}

type RegionBuilder = fn(
    BlockId,
    Option<BlockId>,
    &HashMap<BlockId, Vec<BlockId>>,
    &HashMap<BlockId, Shape>,
    &HashMap<BlockId, Option<BlockId>>,
    &mut HashSet<BlockId>,
) -> Result<ExtendedRegion, StructureError>;

fn structure_using<T>(
    cfg: &Cfg<T>,
    join_builder: crate::JoinBuilder,
    region_builder: RegionBuilder,
    target_indexer: TargetIndex,
) -> Result<ExtendedStructuredCfg, StructureError> {
    cfg.validate()?;
    let blocks: Vec<_> = cfg.block_ids().collect();
    let mut reachable = HashSet::new();
    let mut pending = vec![cfg.entry()];
    while let Some(b) = pending.pop() {
        if reachable.insert(b) {
            for &id in cfg.block(b)?.outgoing() {
                pending.push(cfg.edge(id)?.target());
            }
        }
    }
    let missing: Vec<_> = blocks
        .iter()
        .copied()
        .filter(|b| !reachable.contains(b))
        .collect();
    if !missing.is_empty() {
        return Err(StructureError::UnreachableBlocks(missing));
    }
    let mut edges = Vec::new();
    for id in cfg.edge_ids() {
        let e = cfg.edge(id)?;
        if matches!(e.kind(), EdgeKind::Exception(_)) {
            return Err(StructureError::UnsupportedEdge {
                edge: id,
                kind: e.kind().clone(),
            });
        }
        edges.push(EdgeProvenance {
            id,
            source: e.source(),
            target: e.target(),
            kind: e.kind().clone(),
        });
    }
    let mut successors = HashMap::new();
    let mut shapes = HashMap::new();
    let mut indegree = HashMap::new();
    for &b in &blocks {
        let ids = cfg.block(b)?.outgoing();
        let outgoing = ids
            .iter()
            .map(|id| cfg.edge(*id))
            .collect::<Result<Vec<_>, _>>()?;
        let invalid = || StructureError::UnsupportedBranch {
            block: b,
            edges: ids.to_vec(),
        };
        let (shape, targets) = if outgoing
            .iter()
            .any(|e| matches!(e.kind(), EdgeKind::Switch(_)))
        {
            let mut labels = HashSet::new();
            let mut default = false;
            let mut arms: Vec<(BlockId, Vec<u64>, bool)> = Vec::new();
            let mut target_index = HashMap::new();
            for e in &outgoing {
                let label = match e.kind() {
                    EdgeKind::Switch(v) if labels.insert(*v) => Some(*v),
                    EdgeKind::Unconditional if !default => {
                        default = true;
                        None
                    }
                    _ => return Err(invalid()),
                };
                let i = target_indexer(&mut arms, &mut target_index, e.target());
                if let Some(v) = label {
                    arms[i].1.push(v);
                } else {
                    arms[i].2 = true;
                }
            }
            // Keep duplicate targets here: indegrees count original edges.
            (
                Shape::Switch(arms),
                outgoing.iter().map(|e| e.target()).collect(),
            )
        } else {
            match outgoing.as_slice() {
                [] => (Shape::Plain, Vec::new()),
                [e] if *e.kind() == EdgeKind::Unconditional => (Shape::Plain, vec![e.target()]),
                [a, c]
                    if *a.kind() == EdgeKind::BranchTrue && *c.kind() == EdgeKind::BranchFalse =>
                {
                    (Shape::If, vec![a.target(), c.target()])
                }
                [a, c]
                    if *c.kind() == EdgeKind::BranchTrue && *a.kind() == EdgeKind::BranchFalse =>
                {
                    (Shape::If, vec![c.target(), a.target()])
                }
                _ => return Err(invalid()),
            }
        };
        successors.insert(b, targets);
        shapes.insert(b, shape);
        indegree.insert(b, cfg.block(b)?.incoming().len());
    }
    let mut queue: VecDeque<_> = blocks
        .iter()
        .copied()
        .filter(|b| indegree[b] == 0)
        .collect();
    let mut order = Vec::new();
    while let Some(b) = queue.pop_front() {
        order.push(b);
        for t in &successors[&b] {
            if let Some(n) = indegree.get_mut(t) {
                *n -= 1;
                if *n == 0 {
                    queue.push_back(*t);
                }
            }
        }
    }
    if order.len() != blocks.len() {
        return Err(StructureError::CyclicControlFlow(
            blocks
                .iter()
                .copied()
                .filter(|b| indegree[b] != 0)
                .collect(),
        ));
    }
    let joins = join_builder(&order, &successors);
    let mut seen = HashSet::new();
    let root = region_builder(cfg.entry(), None, &successors, &shapes, &joins, &mut seen)?;
    let missing: Vec<_> = blocks.into_iter().filter(|b| !seen.contains(b)).collect();
    if !missing.is_empty() {
        return Err(StructureError::IncompleteCoverage(missing));
    }
    Ok(ExtendedStructuredCfg {
        entry: cfg.entry(),
        revision: cfg.revision(),
        root,
        edges,
    })
}

#[cfg(test)]
fn original_build(
    start: BlockId,
    stop: Option<BlockId>,
    successors: &HashMap<BlockId, Vec<BlockId>>,
    shapes: &HashMap<BlockId, Shape>,
    joins: &HashMap<BlockId, Option<BlockId>>,
    seen: &mut HashSet<BlockId>,
) -> Result<ExtendedRegion, StructureError> {
    let mut current = Some(start);
    let mut regions = Vec::new();
    while let Some(b) = current {
        if Some(b) == stop {
            break;
        }
        if !seen.insert(b) {
            return Err(StructureError::OverlappingRegion(b));
        }
        let join = joins[&b];
        match &shapes[&b] {
            Shape::Plain => {
                regions.push(ExtendedRegion::Block(b));
                current = successors[&b].first().copied();
            }
            Shape::If => {
                let targets = &successors[&b];
                let then_region = Box::new(original_build(
                    targets[0], join, successors, shapes, joins, seen,
                )?);
                let else_region = Box::new(original_build(
                    targets[1], join, successors, shapes, joins, seen,
                )?);
                regions.push(ExtendedRegion::If {
                    header: b,
                    then_region,
                    else_region,
                    join,
                });
                current = join;
            }
            Shape::Switch(targets) => {
                let mut arms = Vec::new();
                for (target, labels, is_default) in targets {
                    arms.push(SwitchArm {
                        target: *target,
                        labels: labels.clone(),
                        is_default: *is_default,
                        region: original_build(*target, join, successors, shapes, joins, seen)?,
                    });
                }
                regions.push(ExtendedRegion::Switch {
                    header: b,
                    arms,
                    join,
                });
                current = join;
            }
        }
    }
    Ok(if regions.len() == 1 {
        regions.remove(0)
    } else {
        ExtendedRegion::Sequence(regions)
    })
}

fn build(
    start: BlockId,
    stop: Option<BlockId>,
    successors: &HashMap<BlockId, Vec<BlockId>>,
    shapes: &HashMap<BlockId, Shape>,
    joins: &HashMap<BlockId, Option<BlockId>>,
    seen: &mut HashSet<BlockId>,
) -> Result<ExtendedRegion, StructureError> {
    enum Task {
        Walk(Option<BlockId>, Option<BlockId>, Vec<ExtendedRegion>),
        Arm(
            BlockId,
            Option<BlockId>,
            Vec<ExtendedRegion>,
            Vec<ExtendedRegion>,
            usize,
        ),
    }
    let mut tasks = vec![Task::Walk(Some(start), stop, Vec::new())];
    let mut value = ExtendedRegion::Sequence(Vec::new());
    while let Some(task) = tasks.pop() {
        match task {
            Task::Walk(mut current, stop, mut regions) => {
                let mut suspended = false;
                while let Some(b) = current {
                    if Some(b) == stop {
                        break;
                    }
                    if !seen.insert(b) {
                        return Err(StructureError::OverlappingRegion(b));
                    }
                    match &shapes[&b] {
                        Shape::Plain => {
                            regions.push(ExtendedRegion::Block(b));
                            current = successors[&b].first().copied();
                        }
                        shape => {
                            let first = match shape {
                                Shape::If => successors[&b][0],
                                Shape::Switch(arms) => arms[0].0,
                                Shape::Plain => unreachable!(),
                            };
                            tasks.push(Task::Arm(
                                b,
                                stop,
                                std::mem::take(&mut regions),
                                Vec::new(),
                                0,
                            ));
                            tasks.push(Task::Walk(Some(first), joins[&b], Vec::new()));
                            suspended = true;
                            break;
                        }
                    }
                }
                if !suspended {
                    value = if regions.len() == 1 {
                        regions.remove(0)
                    } else {
                        ExtendedRegion::Sequence(regions)
                    };
                }
            }
            Task::Arm(b, stop, mut regions, mut children, index) => {
                children.push(std::mem::replace(
                    &mut value,
                    ExtendedRegion::Sequence(Vec::new()),
                ));
                let join = joins[&b];
                let next = match &shapes[&b] {
                    Shape::If => successors[&b].get(index + 1).copied(),
                    Shape::Switch(arms) => arms.get(index + 1).map(|a| a.0),
                    Shape::Plain => unreachable!(),
                };
                if let Some(next) = next {
                    tasks.push(Task::Arm(b, stop, regions, children, index + 1));
                    tasks.push(Task::Walk(Some(next), join, Vec::new()));
                } else {
                    let mut children = children.into_iter();
                    let region = match &shapes[&b] {
                        Shape::If => {
                            let (Some(then_region), Some(else_region)) =
                                (children.next(), children.next())
                            else {
                                unreachable!("validated conditional has two completed arms")
                            };
                            ExtendedRegion::If {
                                header: b,
                                then_region: Box::new(then_region),
                                else_region: Box::new(else_region),
                                join,
                            }
                        }
                        Shape::Switch(arms) => ExtendedRegion::Switch {
                            header: b,
                            arms: arms
                                .iter()
                                .zip(children)
                                .map(|((target, labels, is_default), region)| SwitchArm {
                                    target: *target,
                                    labels: labels.clone(),
                                    is_default: *is_default,
                                    region,
                                })
                                .collect(),
                            join,
                        },
                        Shape::Plain => unreachable!(),
                    };
                    regions.push(region);
                    tasks.push(Task::Walk(join, stop, regions));
                }
            }
        }
    }
    Ok(value)
}

#[cfg(test)]
pub(crate) fn original_structure<T>(cfg: &Cfg<T>) -> Result<ExtendedStructuredCfg, StructureError> {
    structure_using(
        cfg,
        crate::postdom::original_joins,
        original_build,
        original_target,
    )
}
