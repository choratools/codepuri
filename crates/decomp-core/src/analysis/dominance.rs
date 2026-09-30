use std::collections::HashMap;

use crate::cfg::Cfg;
use crate::ids::BlockId;

use super::traversal::{predecessors, reverse_postorder};

/// Immediate-dominator information for entry-reachable blocks.
#[derive(Clone, Debug)]
pub struct Dominators {
    entry: BlockId,
    immediate: HashMap<BlockId, BlockId>,
    blocks: Vec<BlockId>,
}

impl Dominators {
    /// Returns the immediate dominator, or `None` for entry or unreachable blocks.
    pub fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        (block != self.entry)
            .then(|| self.immediate.get(&block).copied())
            .flatten()
    }

    /// Returns whether `dominator` dominates `block`.
    pub fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        if !self.immediate.contains_key(&dominator) || !self.immediate.contains_key(&block) {
            return false;
        }
        let mut cursor = block;
        loop {
            if cursor == dominator {
                return true;
            }
            if cursor == self.entry {
                return false;
            }
            let Some(parent) = self.immediate.get(&cursor).copied() else {
                return false;
            };
            cursor = parent;
        }
    }

    pub(crate) fn blocks(&self) -> &[BlockId] {
        &self.blocks
    }
}

/// Computes immediate dominators with the Cooper-Harvey-Kennedy algorithm.
pub fn dominators<T>(cfg: &Cfg<T>) -> Dominators {
    let rpo = reverse_postorder(cfg);
    let positions: HashMap<_, _> = rpo
        .iter()
        .copied()
        .enumerate()
        .map(|(position, block)| (block, position))
        .collect();
    let incoming: Vec<Vec<usize>> = rpo
        .iter()
        .map(|&block| {
            predecessors(cfg, block)
                .into_iter()
                .filter_map(|block| positions.get(&block).copied())
                .collect()
        })
        .collect();
    let parents = dense_dominators(&incoming);
    let immediate = rpo
        .iter()
        .enumerate()
        .map(|(index, &block)| (block, rpo[parents[index]]))
        .collect();
    Dominators {
        entry: cfg.entry(),
        immediate,
        blocks: rpo,
    }
}

// Nodes are in root-first RPO; unknown parents are represented by usize::MAX.
// Adjacency is translated once, keeping hashing and CFG lookups out of the
// fixed point and the (potentially long) intersection walks.
pub(crate) fn dense_dominators(incoming: &[Vec<usize>]) -> Vec<usize> {
    let mut immediate = vec![usize::MAX; incoming.len()];
    immediate[0] = 0;
    loop {
        let mut changed = false;
        for block in 1..incoming.len() {
            let mut known = incoming[block]
                .iter()
                .copied()
                .filter(|&predecessor| immediate[predecessor] != usize::MAX);
            let Some(mut candidate) = known.next() else {
                continue;
            };
            for predecessor in known {
                candidate = dense_intersect(candidate, predecessor, &immediate);
            }
            if immediate[block] != candidate {
                immediate[block] = candidate;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    immediate
}

fn dense_intersect(mut left: usize, mut right: usize, immediate: &[usize]) -> usize {
    while left != right {
        while left > right {
            left = immediate[left];
        }
        while right > left {
            right = immediate[right];
        }
    }
    left
}
