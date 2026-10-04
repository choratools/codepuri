//! Lossless language-neutral structuring of CFGs and reducible pretest loops.
//!
//! Accepts unconditional chains and nested true/false branches, including empty
//! arms and distinct terminal blocks. Every live block must be entry-reachable
//! and occurs once in the tree (headers count as occurrences). Original edges
//! are retained verbatim. Switches, exceptions, unsupported cycles, unreachable
//! blocks and overlapping regions return typed errors. Pretest loops require a
//! single latch, a linear evaluation prefix and a single normal exit. Nested
//! loops, break/continue and irreducible flow are unsupported.

use decomp_core::cfg::{Cfg, CfgError, EdgeKind};
use decomp_core::ids::{BlockId, EdgeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
pub mod dispatch;
pub mod exception_dispatch;
pub mod exceptions;
pub mod extended;
pub mod loop_control;
mod loops;
mod postdom;
pub use extended::{ExtendedRegion, ExtendedStructuredCfg, SwitchArm, structure_extended};

#[cfg(test)]
mod differential;
#[cfg(test)]
mod tests;

/// A region references original blocks, without owning their payloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Region {
    /// Reducible pretest loop with evaluation before every condition test.
    While {
        /// Natural-loop entry, including any Phi cluster.
        header: BlockId,
        /// Header instructions, excluding the conditional branch.
        evaluation: Box<Region>,
        /// Conditional branch block.
        condition: BlockId,
        /// Polarity of the body edge.
        body_on_true: bool,
        /// Body, including the unconditional backedge source.
        body: Box<Region>,
        /// Normal exit emitted by the enclosing sequence.
        exit: BlockId,
    },
    /// One ordinary or terminal block.
    Block(BlockId),
    /// Ordered regions; an empty sequence denotes an empty arm.
    Sequence(Vec<Region>),
    /// Conditional header and arms. The header occurs here only.
    If {
        /// Original condition block.
        header: BlockId,
        /// Arm selected by `BranchTrue`.
        then_region: Box<Region>,
        /// Arm selected by `BranchFalse`.
        else_region: Box<Region>,
        /// Nearest common postdominator, excluded from both arms and emitted
        /// by the enclosing sequence. None means separately terminating arms.
        join: Option<BlockId>,
    },
}

/// Original edge identity and semantics, without deduplication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdgeProvenance {
    /// Original edge ID.
    pub id: EdgeId,
    /// Original source.
    pub source: BlockId,
    /// Original target.
    pub target: BlockId,
    /// Original edge category.
    pub kind: EdgeKind,
}

/// Complete structured CFG snapshot. IDs remain tied to the input graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuredCfg {
    /// Original entry.
    pub entry: BlockId,
    /// Original revision; not a globally unique graph identity.
    pub revision: u64,
    /// All live blocks exactly once.
    pub root: Region,
    /// All original edges in CFG iteration order.
    pub edges: Vec<EdgeProvenance>,
}

/// Reason the entire CFG cannot be represented by the supported grammar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StructureError {
    /// Core validation or lookup failed.
    InvalidCfg(CfgError),
    /// Live blocks not reachable from entry.
    UnreachableBlocks(Vec<BlockId>),
    /// Unsupported edge semantics.
    UnsupportedEdge {
        /// Original edge ID.
        edge: EdgeId,
        /// Unsupported category.
        kind: EdgeKind,
    },
    /// Neither terminal, one unconditional edge, nor one true/false pair.
    UnsupportedBranch {
        /// Original block.
        block: BlockId,
        /// Original outgoing edge IDs in adjacency order.
        edges: Vec<EdgeId>,
    },
    /// Nodes left by Kahn elimination: cyclic nodes and potentially downstream
    /// nodes. This is not an SCC report; the cycle is outside the while grammar.
    CyclicControlFlow(Vec<BlockId>),
    /// A non-nested region would emit this block twice.
    OverlappingRegion(BlockId),
    /// Final coverage check found omitted blocks.
    IncompleteCoverage(Vec<BlockId>),
}

impl fmt::Display for StructureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported CFG structure: {self:?}")
    }
}
impl std::error::Error for StructureError {}
impl From<CfgError> for StructureError {
    fn from(value: CfgError) -> Self {
        Self::InvalidCfg(value)
    }
}

/// Structure an entire CFG without mutation or trait bounds on its payload.
///
/// Branch polarity comes from edge kinds, never adjacency order. A virtual
/// exit shared by terminal blocks is used for postdominators but never emitted.
/// DAG postdominator construction takes O((B+E) log B) time and O(B log B)
/// space, in addition to CFG validation. Region construction uses explicit frames.
pub fn structure<T>(cfg: &Cfg<T>) -> Result<StructuredCfg, StructureError> {
    structure_using(cfg, postdom::joins, build)
}

type JoinBuilder =
    fn(&[BlockId], &HashMap<BlockId, Vec<BlockId>>) -> HashMap<BlockId, Option<BlockId>>;
type RegionBuilder = fn(
    BlockId,
    Option<BlockId>,
    &HashMap<BlockId, Vec<BlockId>>,
    &HashMap<BlockId, Option<BlockId>>,
    &mut HashSet<BlockId>,
) -> Result<Region, StructureError>;

fn structure_using<T>(
    cfg: &Cfg<T>,
    join_builder: JoinBuilder,
    region_builder: RegionBuilder,
) -> Result<StructuredCfg, StructureError> {
    cfg.validate()?;
    let blocks: Vec<_> = cfg.block_ids().collect();
    let mut reachable = HashSet::new();
    let mut pending = vec![cfg.entry()];
    while let Some(block) = pending.pop() {
        if reachable.insert(block) {
            for &edge in cfg.block(block)?.outgoing() {
                pending.push(cfg.edge(edge)?.target());
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
        let edge = cfg.edge(id)?;
        if matches!(edge.kind(), EdgeKind::Switch(_) | EdgeKind::Exception(_)) {
            return Err(StructureError::UnsupportedEdge {
                edge: id,
                kind: edge.kind().clone(),
            });
        }
        edges.push(EdgeProvenance {
            id,
            source: edge.source(),
            target: edge.target(),
            kind: edge.kind().clone(),
        });
    }
    let mut successors = HashMap::new();
    let mut indegree = HashMap::new();
    for &block in &blocks {
        let outgoing = cfg.block(block)?.outgoing();
        let mut targets = Vec::new();
        let valid = match outgoing {
            [] => true,
            [id] => {
                let edge = cfg.edge(*id)?;
                targets.push(edge.target());
                *edge.kind() == EdgeKind::Unconditional
            }
            [a, b] => {
                let a = cfg.edge(*a)?;
                let b = cfg.edge(*b)?;
                match (a.kind(), b.kind()) {
                    (EdgeKind::BranchTrue, EdgeKind::BranchFalse) => {
                        targets.extend([a.target(), b.target()]);
                        true
                    }
                    (EdgeKind::BranchFalse, EdgeKind::BranchTrue) => {
                        targets.extend([b.target(), a.target()]);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if !valid {
            return Err(StructureError::UnsupportedBranch {
                block,
                edges: outgoing.to_vec(),
            });
        }
        successors.insert(block, targets);
        indegree.insert(block, cfg.block(block)?.incoming().len());
    }
    let mut queue: VecDeque<_> = blocks
        .iter()
        .copied()
        .filter(|b| indegree[b] == 0)
        .collect();
    let mut order = Vec::new();
    while let Some(block) = queue.pop_front() {
        order.push(block);
        for target in &successors[&block] {
            if let Some(count) = indegree.get_mut(target) {
                *count -= 1;
                if *count == 0 {
                    queue.push_back(*target);
                }
            }
        }
    }
    if order.len() != blocks.len() {
        return loops::structure(cfg, &blocks, &successors, edges);
    }
    let joins = join_builder(&order, &successors);
    let mut seen = HashSet::new();
    let root = region_builder(cfg.entry(), None, &successors, &joins, &mut seen)?;
    let missing: Vec<_> = blocks.into_iter().filter(|b| !seen.contains(b)).collect();
    if !missing.is_empty() {
        return Err(StructureError::IncompleteCoverage(missing));
    }
    Ok(StructuredCfg {
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
    joins: &HashMap<BlockId, Option<BlockId>>,
    seen: &mut HashSet<BlockId>,
) -> Result<Region, StructureError> {
    let mut current = Some(start);
    let mut regions = Vec::new();
    while let Some(block) = current {
        if Some(block) == stop {
            break;
        }
        if !seen.insert(block) {
            return Err(StructureError::OverlappingRegion(block));
        }
        match successors[&block].as_slice() {
            [] => {
                regions.push(Region::Block(block));
                current = None;
            }
            [next] => {
                regions.push(Region::Block(block));
                current = Some(*next);
            }
            [yes, no] => {
                let join = joins[&block];
                let then_region = original_build(*yes, join, successors, joins, seen)?;
                let else_region = original_build(*no, join, successors, joins, seen)?;
                regions.push(Region::If {
                    header: block,
                    then_region: Box::new(then_region),
                    else_region: Box::new(else_region),
                    join,
                });
                current = join;
            }
            _ => unreachable!("branch shapes validated before construction"),
        }
    }
    Ok(if regions.len() == 1 {
        regions.remove(0)
    } else {
        Region::Sequence(regions)
    })
}

fn build(
    start: BlockId,
    stop: Option<BlockId>,
    successors: &HashMap<BlockId, Vec<BlockId>>,
    joins: &HashMap<BlockId, Option<BlockId>>,
    seen: &mut HashSet<BlockId>,
) -> Result<Region, StructureError> {
    enum Task {
        Walk(Option<BlockId>, Option<BlockId>, Vec<Region>),
        Then(
            BlockId,
            BlockId,
            Option<BlockId>,
            Option<BlockId>,
            Vec<Region>,
        ),
        Else(
            BlockId,
            Option<BlockId>,
            Option<BlockId>,
            Vec<Region>,
            Region,
        ),
    }
    let mut tasks = vec![Task::Walk(Some(start), stop, Vec::new())];
    let mut value = Region::Sequence(Vec::new());
    while let Some(task) = tasks.pop() {
        match task {
            Task::Walk(mut current, stop, mut regions) => {
                let mut suspended = false;
                while let Some(block) = current {
                    if Some(block) == stop {
                        break;
                    }
                    if !seen.insert(block) {
                        return Err(StructureError::OverlappingRegion(block));
                    }
                    match successors[&block].as_slice() {
                        [] => {
                            regions.push(Region::Block(block));
                            current = None;
                        }
                        [next] => {
                            regions.push(Region::Block(block));
                            current = Some(*next);
                        }
                        [yes, no] => {
                            let join = joins[&block];
                            tasks.push(Task::Then(
                                block,
                                *no,
                                join,
                                stop,
                                std::mem::take(&mut regions),
                            ));
                            tasks.push(Task::Walk(Some(*yes), join, Vec::new()));
                            suspended = true;
                            break;
                        }
                        _ => unreachable!("validated branch shape"),
                    }
                }
                if !suspended {
                    value = if regions.len() == 1 {
                        regions.remove(0)
                    } else {
                        Region::Sequence(regions)
                    };
                }
            }
            Task::Then(header, no, join, stop, regions) => {
                tasks.push(Task::Else(
                    header,
                    join,
                    stop,
                    regions,
                    std::mem::replace(&mut value, Region::Sequence(Vec::new())),
                ));
                tasks.push(Task::Walk(Some(no), join, Vec::new()));
            }
            Task::Else(header, join, stop, mut regions, then_region) => {
                regions.push(Region::If {
                    header,
                    then_region: Box::new(then_region),
                    else_region: Box::new(std::mem::replace(
                        &mut value,
                        Region::Sequence(Vec::new()),
                    )),
                    join,
                });
                tasks.push(Task::Walk(join, stop, regions));
            }
        }
    }
    Ok(value)
}
