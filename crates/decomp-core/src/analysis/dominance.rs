use std::collections::HashMap;
use std::sync::OnceLock;

use crate::cfg::Cfg;
use crate::ids::BlockId;

use super::traversal::{predecessors, reverse_postorder};

/// Immediate-dominator information for entry-reachable blocks.
#[derive(Clone, Debug)]
pub struct Dominators {
    positions: HashMap<BlockId, usize>,
    parents: Vec<usize>,
    intervals: OnceLock<Vec<(usize, usize)>>,
    blocks: Vec<BlockId>,
}

impl Dominators {
    /// Returns the immediate dominator, or `None` for entry or unreachable blocks.
    pub fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        let &position = self.positions.get(&block)?;
        (position != 0).then(|| self.blocks[self.parents[position]])
    }

    /// Returns whether `dominator` dominates `block`. Nontrivial queries build
    /// O(B) tree intervals once; subsequent queries use expected O(1) lookups.
    pub fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        let (Some(&left), Some(&right)) =
            (self.positions.get(&dominator), self.positions.get(&block))
        else {
            return false;
        };
        if left == right || left == 0 || self.parents[right] == left {
            return true;
        }
        if right == 0 || self.parents[left] == right {
            return false;
        }
        let intervals = self.intervals.get_or_init(|| tree_intervals(&self.parents));
        let (begin, end) = intervals[left];
        let (other_begin, other_end) = intervals[right];
        begin <= other_begin && other_end <= end
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
    Dominators {
        positions,
        parents,
        intervals: OnceLock::new(),
        blocks: rpo,
    }
}

/// Iterative DFS over dense tree parents. The synthetic/real root is slot zero;
/// no recursion or separately allocated child vector is needed for long chains.
pub(crate) fn tree_intervals(parents: &[usize]) -> Vec<(usize, usize)> {
    let mut first_child = vec![usize::MAX; parents.len()];
    let mut next_sibling = vec![usize::MAX; parents.len()];
    for node in 1..parents.len() {
        next_sibling[node] = first_child[parents[node]];
        first_child[parents[node]] = node;
    }
    let mut intervals = vec![(0, 0); parents.len()];
    let mut stack = vec![(0, false)];
    let mut clock = 0;
    while let Some((node, expanded)) = stack.pop() {
        if expanded {
            intervals[node].1 = clock;
        } else {
            intervals[node].0 = clock;
            stack.push((node, true));
            let mut child = first_child[node];
            while child != usize::MAX {
                stack.push((child, false));
                child = next_sibling[child];
            }
        }
        clock += 1;
    }
    intervals
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::EdgeKind;

    #[test]
    fn trivial_queries_do_not_build_intervals_and_nontrivial_queries_reuse_them() {
        let mut cfg = Cfg::new(());
        let mut blocks = vec![cfg.entry()];
        for _ in 1..1024 {
            let block = cfg.add_block(());
            assert!(
                cfg.add_edge(blocks[blocks.len() - 1], block, EdgeKind::Unconditional)
                    .is_ok()
            );
            blocks.push(block);
        }
        let dom = dominators(&cfg);
        assert!(dom.dominates(blocks[0], blocks[1023]));
        assert!(dom.dominates(blocks[1], blocks[2]));
        assert!(!dom.dominates(blocks[2], blocks[1]));
        assert!(dom.intervals.get().is_none());
        assert!(dom.dominates(blocks[1], blocks[1023]));
        let pointer = dom.intervals.get().map(|intervals| intervals.as_ptr());
        assert!(!dom.dominates(blocks[1023], blocks[1]));
        assert_eq!(
            pointer,
            dom.intervals.get().map(|intervals| intervals.as_ptr())
        );
    }
}
