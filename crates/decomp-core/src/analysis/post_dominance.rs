use std::collections::{HashMap, HashSet};

use crate::cfg::Cfg;
use crate::ids::BlockId;

use super::dominance::dense_dominators;
use super::traversal::{predecessors, reverse_postorder, successors};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PostNode {
    Block(BlockId),
    Exit,
}

/// Immediate post-dominator information for blocks that can reach an exit.
#[derive(Clone, Debug)]
pub struct PostDominators {
    immediate: HashMap<BlockId, Option<BlockId>>,
}

impl PostDominators {
    /// Returns the immediate post-dominator when it is a real block.
    pub fn immediate_post_dominator(&self, block: BlockId) -> Option<BlockId> {
        self.immediate.get(&block).copied().flatten()
    }

    /// Returns whether `post_dominator` post-dominates `block`.
    pub fn post_dominates(&self, post_dominator: BlockId, block: BlockId) -> bool {
        if !self.immediate.contains_key(&post_dominator) || !self.immediate.contains_key(&block) {
            return false;
        }
        let mut cursor = Some(block);
        while let Some(current) = cursor {
            if current == post_dominator {
                return true;
            }
            cursor = self.immediate.get(&current).copied().flatten();
        }
        false
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
    let immediate = dense_dominators(&incoming);
    let real = rpo
        .iter()
        .copied()
        .enumerate()
        .map(|(index, node)| (node, rpo[immediate[index]]))
        .filter_map(|(node, parent)| match node {
            PostNode::Block(block) => Some((
                block,
                match parent {
                    PostNode::Block(parent_block) => Some(parent_block),
                    PostNode::Exit => None,
                },
            )),
            PostNode::Exit => None,
        })
        .collect();
    PostDominators { immediate: real }
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
