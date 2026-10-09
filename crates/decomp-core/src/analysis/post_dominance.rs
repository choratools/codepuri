use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::cfg::Cfg;
use crate::ids::BlockId;

use super::dominance::{dense_dominators, tree_intervals};
use super::traversal::{predecessors, reverse_postorder, successors};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PostNode {
    Block(BlockId),
    Exit,
}

/// Immediate post-dominator information for blocks that can reach an exit.
#[derive(Clone, Debug)]
pub struct PostDominators {
    positions: HashMap<BlockId, usize>,
    nodes: Vec<PostNode>,
    parents: Vec<usize>,
    intervals: OnceLock<Vec<(usize, usize)>>,
}

impl PostDominators {
    /// Returns the immediate post-dominator when it is a real block.
    pub fn immediate_post_dominator(&self, block: BlockId) -> Option<BlockId> {
        let &position = self.positions.get(&block)?;
        match self.nodes[self.parents[position]] {
            PostNode::Block(parent) => Some(parent),
            PostNode::Exit => None,
        }
    }

    /// Returns whether `post_dominator` post-dominates `block`. Nontrivial
    /// queries build the O(B) synthetic-exit intervals once, then use O(1)
    /// interval checks and expected O(1) block lookups.
    pub fn post_dominates(&self, post_dominator: BlockId, block: BlockId) -> bool {
        let (Some(&left), Some(&right)) = (
            self.positions.get(&post_dominator),
            self.positions.get(&block),
        ) else {
            return false;
        };
        if left == right || self.parents[right] == left {
            return true;
        }
        if self.parents[left] == right {
            return false;
        }
        let intervals = self.intervals.get_or_init(|| tree_intervals(&self.parents));
        let (begin, end) = intervals[left];
        let (other_begin, other_end) = intervals[right];
        begin <= other_begin && other_end <= end
    }
}

/// Computes post-dominators over entry-reachable blocks that can reach an exit.
pub fn post_dominators<T>(cfg: &Cfg<T>) -> PostDominators {
    let reachable_order = reverse_postorder(cfg);
    let reachable: HashSet<_> = reachable_order.iter().copied().collect();
    let exits: Vec<_> = reachable_order
        .iter()
        .copied()
        .filter(|block| {
            successors(cfg, *block)
                .into_iter()
                .all(|successor| !reachable.contains(&successor))
        })
        .collect();
    let rpo = post_reverse_postorder(cfg, &exits, &reachable);
    let positions: HashMap<_, _> = rpo
        .iter()
        .copied()
        .enumerate()
        .map(|(position, node)| (node, position))
        .collect();
    let exit_set: HashSet<_> = exits.iter().copied().collect();
    let incoming: Vec<Vec<usize>> = rpo
        .iter()
        .map(|node| match node {
            PostNode::Exit => Vec::new(),
            PostNode::Block(block) => {
                let mut incoming: Vec<_> = successors(cfg, *block)
                    .into_iter()
                    .filter_map(|block| positions.get(&PostNode::Block(block)).copied())
                    .collect();
                if exit_set.contains(block) {
                    incoming.push(0);
                }
                incoming
            }
        })
        .collect();
    let parents = dense_dominators(&incoming);
    let positions = positions
        .into_iter()
        .filter_map(|(node, position)| match node {
            PostNode::Block(block) => Some((block, position)),
            PostNode::Exit => None,
        })
        .collect();
    PostDominators {
        positions,
        nodes: rpo,
        parents,
        intervals: OnceLock::new(),
    }
}

fn post_reverse_postorder<T>(
    cfg: &Cfg<T>,
    exits: &[BlockId],
    reachable: &HashSet<BlockId>,
) -> Vec<PostNode> {
    let mut seen = HashSet::from([PostNode::Exit]);
    let mut stack = vec![(PostNode::Exit, false)];
    let mut order = Vec::new();
    while let Some((node, expanded)) = stack.pop() {
        if expanded {
            order.push(node);
            continue;
        }
        stack.push((node, true));
        let next: Vec<PostNode> = match node {
            PostNode::Exit => exits.iter().copied().map(PostNode::Block).collect(),
            PostNode::Block(block) => predecessors(cfg, block)
                .into_iter()
                .filter(|candidate| reachable.contains(candidate))
                .map(PostNode::Block)
                .collect(),
        };
        for successor in next.into_iter().rev() {
            if seen.insert(successor) {
                stack.push((successor, false));
            }
        }
    }
    order.reverse();
    order
}
