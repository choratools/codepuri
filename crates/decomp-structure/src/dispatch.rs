//! Exact normal-CFG state scheduling, including irreducible cycles.
use crate::{EdgeProvenance, StructureError};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use std::collections::{HashSet, VecDeque};

/// Immutable canonical state-machine snapshot; payloads remain in the CFG.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchPlan {
    /// Original entry block.
    pub entry: BlockId,
    /// Original revision.
    pub revision: u64,
    /// Each reachable real block exactly once, in breadth-first order.
    pub states: Vec<DispatchState>,
    /// Every original edge, without deduplication.
    pub edges: Vec<EdgeProvenance>,
}
/// One original instruction block and its original outgoing edges.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchState {
    /// Original block identity.
    pub block: BlockId,
    /// Original edge identities in adjacency order.
    pub outgoing: Vec<EdgeId>,
}
/// Validate and schedule an ordinary normal CFG in O(B+E) space/time.
/// Exception edges reject; monitor instructions are the typed parent's concern.
/// Unreachable live blocks reject rather than silently omitting source.
pub fn plan<T>(cfg: &Cfg<T>) -> Result<DispatchPlan, StructureError> {
    cfg.validate()?;
    let mut seen = HashSet::new();
    let mut todo = VecDeque::from([cfg.entry()]);
    let mut states = Vec::new();
    while let Some(block) = todo.pop_front() {
        if !seen.insert(block) {
            continue;
        }
        let outgoing = cfg.block(block)?.outgoing().to_vec();
        let mut kinds = Vec::new();
        for &id in &outgoing {
            let edge = cfg.edge(id)?;
            if matches!(edge.kind(), EdgeKind::Exception(_)) {
                return Err(StructureError::UnsupportedEdge {
                    edge: id,
                    kind: edge.kind().clone(),
                });
            }
            kinds.push(edge.kind());
            todo.push_back(edge.target());
        }
        let valid = match kinds.as_slice() {
            [] => true,
            [EdgeKind::Unconditional] => true,
            [EdgeKind::BranchTrue, EdgeKind::BranchFalse]
            | [EdgeKind::BranchFalse, EdgeKind::BranchTrue] => true,
            _ => {
                let mut labels = HashSet::new();
                kinds
                    .iter()
                    .filter(|k| matches!(k, EdgeKind::Unconditional))
                    .count()
                    == 1
                    && kinds.iter().all(|k| match k {
                        EdgeKind::Unconditional => true,
                        EdgeKind::Switch(value) => labels.insert(*value),
                        _ => false,
                    })
                    && kinds.iter().any(|k| matches!(k, EdgeKind::Switch(_)))
            }
        };
        if !valid {
            return Err(StructureError::UnsupportedBranch {
                block,
                edges: outgoing,
            });
        }
        states.push(DispatchState { block, outgoing });
    }
    let missing: Vec<_> = cfg.block_ids().filter(|b| !seen.contains(b)).collect();
    if !missing.is_empty() {
        return Err(StructureError::UnreachableBlocks(missing));
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
    Ok(DispatchPlan {
        entry: cfg.entry(),
        revision: cfg.revision(),
        states,
        edges,
    })
}
