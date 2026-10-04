//! Analyses tied to one immutably borrowed mirror CFG. Payload edits do not
//! change topology; a topology edit requires dropping this context first.
use decomp_core::analysis::{Dominators, dominance_frontier, dominators};
use decomp_core::cfg::Cfg;
use decomp_core::ids::BlockId;
use std::collections::HashMap;

pub(super) struct Analysis<'a> {
    pub cfg: &'a Cfg<usize>,
    pub doms: Dominators,
    pub frontier: HashMap<BlockId, Vec<BlockId>>,
    blocks: HashMap<usize, BlockId>,
}

impl<'a> Analysis<'a> {
    pub fn new(cfg: &'a Cfg<usize>) -> Self {
        let doms = dominators(cfg);
        let frontier = dominance_frontier(cfg, &doms);
        let blocks = cfg
            .block_ids()
            .filter_map(|id| cfg.block(id).ok().map(|block| (*block.payload(), id)))
            .collect();
        Self {
            cfg,
            doms,
            frontier,
            blocks,
        }
    }

    pub fn block_of_chain(&self, chain: usize) -> Option<BlockId> {
        self.blocks.get(&chain).copied()
    }
}
