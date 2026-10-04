//! The mutable chain substrate used by SSA construction.
//!
//! `decomp-core`'s `Cfg` intentionally exposes no payload mutation, so the
//! SSA transform copies the fused-chain graph into this lightweight
//! structure: chain payloads become mutable `Vec<InsnId>` lists and edges
//! become ordered tuples. Topology analyses (dominators, frontiers,
//! postorder) run on a mirror `Cfg<usize>` whose payloads are chain
//! indexes; phi inputs are keyed by the mirror's edge identities and
//! remapped to final edge identities at materialization. Placement edits
//! (hoisting synthetic definitions, entry initials) are plain vector
//! operations here.

use std::collections::HashMap;

use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::{BlockId, EdgeId};
use managed_ir::value::InsnId;
use managed_ir::{insn::ManagedInsn, method::InsnArena};

use crate::merge::MergedMethod;

/// One fused chain's ordered instruction list, mutable during SSA
/// construction.
pub(crate) type Chain = Vec<InsnId>;

/// The fused-chain graph with mutable payloads.
#[derive(Debug, Default)]
pub(crate) struct Chains {
    /// Payloads indexed by chain number; entry chain is index zero.
    pub payloads: Vec<Chain>,
    /// Edges as `(source chain, target chain, kind)` in stable order.
    pub edges: Vec<(usize, usize, EdgeKind)>,
    /// The merged-CFG edge identity each chain edge came from.
    pub merged_edges: Vec<EdgeId>,
}

impl Chains {
    /// Make method invocation an explicit predecessor of a loop at original
    /// entry. Without this edge, SSA would see only backedges at that header
    /// and could omit the initial input or execute initial definitions again.
    /// The offset-free branch has no effects and introduces no exception edge.
    pub fn add_invocation_entry(&mut self, insns: &mut InsnArena) {
        if !self.edges.iter().any(|(_, target, _)| *target == 0) {
            return;
        }
        // Null is not an identity allocated by any genuine CFG edge arena. It
        // names this one synthetic edge until materialization remaps all edges.
        let invocation_edge = EdgeId::default();
        let branch = insns.insert(ManagedInsn::Branch {
            condition: None,
            then_edge: invocation_edge,
            else_edge: None,
        });
        self.payloads.insert(0, vec![branch]);
        for (source, target, _) in &mut self.edges {
            *source += 1;
            *target += 1;
        }
        self.edges.push((0, 1, EdgeKind::Unconditional));
        self.merged_edges.push(invocation_edge);
    }

    /// Copies the fused-chain graph out of a merged method.
    ///
    /// Chains are numbered in the merged CFG's block order, so the entry
    /// chain is zero.
    pub fn from_merged(merged: &MergedMethod) -> Self {
        let mut index_of: HashMap<BlockId, usize> =
            HashMap::with_capacity(merged.cfg().block_count());
        let mut payloads = Vec::with_capacity(merged.cfg().block_count());
        for (index, block) in merged.cfg().block_ids().enumerate() {
            index_of.insert(block, index);
            let payload = merged
                .cfg()
                .block(block)
                .map(|record| record.payload().clone())
                .unwrap_or_default();
            payloads.push(payload);
        }
        let mut edges = Vec::with_capacity(merged.cfg().edge_count());
        let mut merged_edges = Vec::with_capacity(merged.cfg().edge_count());
        for edge in merged.cfg().edge_ids() {
            if let Ok(record) = merged.cfg().edge(edge)
                && let (Some(source), Some(target)) = (
                    index_of.get(&record.source()),
                    index_of.get(&record.target()),
                )
            {
                edges.push((*source, *target, record.kind().clone()));
                merged_edges.push(edge);
            }
        }
        Self {
            payloads,
            edges,
            merged_edges,
        }
    }

    /// Builds the mirror `Cfg<usize>` used for topology analyses.
    ///
    /// Mirror edge `i` corresponds to [`Self::edges`] entry `i`, and the
    /// mirror's block order matches chain index order.
    pub fn analysis_cfg(&self) -> Cfg<usize> {
        let mut cfg: Cfg<usize> = Cfg::new(0);
        let mut ids = vec![cfg.entry()];
        for index in 1..self.payloads.len() {
            ids.push(cfg.add_block(index));
        }
        for (source, target, kind) in &self.edges {
            let _ = cfg.add_edge(ids[*source], ids[*target], kind.clone());
        }
        cfg
    }

    /// Returns the target chain of an edge index.
    #[allow(dead_code)]
    pub fn edge_target(&self, edge: usize) -> usize {
        self.edges[edge].1
    }
}

/// Maps mirror-CFG edge identities to chain edge indexes.
pub(crate) fn edge_index_map(cfg: &Cfg<usize>, total: usize) -> HashMap<EdgeId, usize> {
    let mut map = HashMap::with_capacity(total);
    for (index, edge) in cfg.edge_ids().enumerate() {
        map.insert(edge, index);
    }
    map
}
