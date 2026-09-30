use std::collections::HashSet;

use crate::cfg::Cfg;
use crate::ids::BlockId;

use super::Dominators;
use super::traversal::predecessors;

#[derive(Clone, Copy)]
struct BackEdge {
    header: BlockId,
    latch: BlockId,
}

/// A natural loop induced by one dominance back edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NaturalLoop {
    header: BlockId,
    latch: BlockId,
    blocks: Vec<BlockId>,
}

impl NaturalLoop {
    /// Returns the loop header targeted by the back edge.
    pub const fn header(&self) -> BlockId {
        self.header
    }

    /// Returns the back-edge source.
    pub const fn latch(&self) -> BlockId {
        self.latch
    }

    /// Returns loop members in deterministic CFG block order.
    pub fn blocks(&self) -> &[BlockId] {
        &self.blocks
    }
}

/// Finds natural loops from edges whose targets dominate their sources.
pub fn natural_loops<T>(cfg: &Cfg<T>, dominators: &Dominators) -> Vec<NaturalLoop> {
    cfg.edge_ids()
        .filter_map(|edge_id| cfg.edge(edge_id).ok())
        .filter(|edge| dominators.dominates(edge.target(), edge.source()))
        .map(|edge| {
            build_loop(
                cfg,
                dominators,
                BackEdge {
                    header: edge.target(),
                    latch: edge.source(),
                },
            )
        })
        .collect()
}

fn build_loop<T>(cfg: &Cfg<T>, dominators: &Dominators, back_edge: BackEdge) -> NaturalLoop {
    let BackEdge { header, latch } = back_edge;
    let mut members = HashSet::from([header, latch]);
    let mut stack = if header == latch {
        Vec::new()
    } else {
        vec![latch]
    };
    while let Some(block) = stack.pop() {
        for predecessor in predecessors(cfg, block) {
            if dominators.dominates(header, predecessor)
                && members.insert(predecessor)
                && predecessor != header
            {
                stack.push(predecessor);
            }
        }
    }
    let blocks = cfg
        .block_ids()
        .filter(|block| members.contains(block))
        .collect();
    NaturalLoop {
        header,
        latch,
        blocks,
    }
}
