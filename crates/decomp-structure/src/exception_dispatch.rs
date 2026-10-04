//! Canonical scheduling of original normal and exceptional CFG transfers.
//!
//! This layer retains exceptional edge identities and their adjacency order;
//! it does not infer catch priority, original protected ranges, caught values,
//! instruction legality, prethrow SSA state, or allocation/initializer proofs.
//! Those are obligations of the typed frontend/backend before source emission.
use crate::{EdgeProvenance, StructureError};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use std::collections::{HashMap, HashSet, VecDeque};

/// Canonical scheduling plus exact source-table priority for each throwing site.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopedPlan {
    /// Original graph snapshot.
    pub graph: Plan,
    /// Source-scope index for every covered original or attested synthetic block.
    pub coverage: HashMap<BlockId, usize>,
    /// Exceptional edges in encoded catch-table order, never adjacency order.
    pub handlers: HashMap<BlockId, Vec<EdgeId>>,
}

/// Validate source-table coverage without imposing structured-region shapes.
/// Entries, normal exits, loops, and shared handlers impose no restrictions.
/// Different source tables may not claim the same instruction; frontends must
/// first normalize their original range evidence to one effective table/site.
pub fn plan_scopes<T>(
    cfg: &Cfg<T>,
    scopes: &[crate::exceptions::RegionScope],
) -> Result<ScopedPlan, crate::exceptions::Error> {
    use crate::exceptions::Error;
    let graph = plan(cfg).map_err(|e| Error::Invalid(e.to_string()))?;
    let mut coverage = HashMap::new();
    for (index, scope) in scopes.iter().enumerate() {
        if scope.protected.is_empty() || scope.catches.is_empty() {
            return Err(Error::Unsupported("empty dispatch source scope"));
        }
        let mut identities = HashSet::new();
        for catch in &scope.catches {
            cfg.block(catch.entry)
                .map_err(|e| Error::Invalid(e.to_string()))?;
            if !identities.insert((&catch.key, catch.entry)) {
                return Err(Error::Unsupported("duplicate encoded handler"));
            }
        }
        for &block in &scope.protected {
            cfg.block(block)
                .map_err(|e| Error::Invalid(e.to_string()))?;
            if coverage.insert(block, index).is_some() {
                return Err(Error::Unsupported("ambiguous dispatch source coverage"));
            }
        }
    }
    let mut handlers = HashMap::new();
    for state in &graph.states {
        if state.exceptional.is_empty() {
            continue;
        }
        let scope = coverage
            .get(&state.block)
            .map(|&i| &scopes[i])
            .ok_or(Error::Unsupported(
                "exception edge outside original source table",
            ))?;
        let mut ordered = Vec::new();
        for catch in &scope.catches {
            let matching = state
                .exceptional
                .iter()
                .filter(|&&id| {
                    cfg.edge(id).is_ok_and(|e| {
                        e.target() == catch.entry
                            && e.kind() == &EdgeKind::Exception(catch.key.clone())
                    })
                })
                .copied()
                .collect::<Vec<_>>();
            let [edge] = matching.as_slice() else {
                return Err(Error::Unsupported(
                    "incomplete or duplicate encoded dispatch",
                ));
            };
            ordered.push(*edge);
        }
        if ordered.len() != state.exceptional.len() {
            return Err(Error::Unsupported("unattested exceptional edge"));
        }
        handlers.insert(state.block, ordered);
    }
    Ok(ScopedPlan {
        graph,
        coverage,
        handlers,
    })
}

/// Immutable canonical schedule tied to the original CFG and its revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    /// Original entry, always the first scheduled state.
    pub entry: BlockId,
    /// Original graph revision.
    pub revision: u64,
    /// Every reachable real block once, in breadth-first order over all edges.
    pub states: Vec<State>,
    /// Every original edge once in arena order, without deduplication.
    pub edges: Vec<EdgeProvenance>,
}

/// Original outgoing transfers for one real block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct State {
    /// Original block identity.
    pub block: BlockId,
    /// Normal outgoing edge identities, preserving their adjacency order.
    pub normal: Vec<EdgeId>,
    /// Exceptional edge identities, preserving their adjacency order.
    /// This order is not an attestation of encoded catch-table priority.
    pub exceptional: Vec<EdgeId>,
}

/// Validate and schedule in O(B+E) time/space without recursion.
///
/// Cycles, multiple normal exits and shared handlers are preserved. Unsupported
/// normal branch shapes and unreachable live blocks reject. A zero-normal-edge
/// state is structurally terminal; the typed parent must prove whether its
/// original instruction is a return, throw, or another supported terminal.
pub fn plan<T>(cfg: &Cfg<T>) -> Result<Plan, StructureError> {
    cfg.validate()?;
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([cfg.entry()]);
    let mut states = Vec::with_capacity(cfg.block_count());
    while let Some(block) = queue.pop_front() {
        if !seen.insert(block) {
            continue;
        }
        let outgoing = cfg.block(block)?.outgoing();
        let mut normal = Vec::new();
        let mut exceptional = Vec::new();
        let mut kinds = Vec::new();
        for &id in outgoing {
            let edge = cfg.edge(id)?;
            if matches!(edge.kind(), EdgeKind::Exception(_)) {
                exceptional.push(id);
            } else {
                normal.push(id);
                kinds.push(edge.kind());
            }
            queue.push_back(edge.target());
        }
        let valid = match kinds.as_slice() {
            [] | [EdgeKind::Unconditional] => true,
            [EdgeKind::BranchTrue, EdgeKind::BranchFalse]
            | [EdgeKind::BranchFalse, EdgeKind::BranchTrue] => true,
            _ => {
                let mut labels = HashSet::new();
                kinds
                    .iter()
                    .filter(|kind| matches!(kind, EdgeKind::Unconditional))
                    .count()
                    == 1
                    && kinds.iter().all(|kind| match kind {
                        EdgeKind::Unconditional => true,
                        EdgeKind::Switch(value) => labels.insert(*value),
                        _ => false,
                    })
                    && kinds.iter().any(|kind| matches!(kind, EdgeKind::Switch(_)))
            }
        };
        if !valid {
            return Err(StructureError::UnsupportedBranch {
                block,
                edges: normal,
            });
        }
        states.push(State {
            block,
            normal,
            exceptional,
        });
    }
    let missing = cfg
        .block_ids()
        .filter(|block| !seen.contains(block))
        .collect::<Vec<_>>();
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
    Ok(Plan {
        entry: cfg.entry(),
        revision: cfg.revision(),
        states,
        edges,
    })
}

/// Reject a stale, reordered, omitted, or fabricated snapshot transactionally.
/// Payload validation remains a separate typed-parent obligation.
pub fn validate<T>(cfg: &Cfg<T>, snapshot: &Plan) -> Result<(), StructureError> {
    let canonical = plan(cfg)?;
    if canonical == *snapshot {
        Ok(())
    } else {
        Err(StructureError::IncompleteCoverage(
            canonical.states.iter().map(|state| state.block).collect(),
        ))
    }
}
