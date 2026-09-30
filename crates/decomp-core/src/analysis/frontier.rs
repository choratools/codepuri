use std::collections::HashMap;

use crate::cfg::Cfg;
use crate::ids::BlockId;

use super::Dominators;
use super::traversal::predecessors;

/// Computes dominance-frontier blocks in deterministic CFG block order.
pub fn dominance_frontier<T>(
    cfg: &Cfg<T>,
    dominators: &Dominators,
) -> HashMap<BlockId, Vec<BlockId>> {
    let mut sets: HashMap<BlockId, Vec<BlockId>> = dominators
        .blocks()
        .iter()
        .copied()
        .map(|block| (block, Vec::new()))
        .collect();
    for block in dominators.blocks().iter().copied() {
        let stop = dominators.immediate_dominator(block);
        for predecessor in predecessors(cfg, block) {
            if !sets.contains_key(&predecessor) {
                continue;
            }
            let mut runner = Some(predecessor);
            while runner != stop {
                let Some(current) = runner else {
                    break;
                };
                if let Some(frontier) = sets.get_mut(&current) {
                    // All walks for this target are consecutive. Once a walk
                    // meets an already visited ancestor, its remaining suffix
                    // has also been visited (including parallel-edge walks).
                    if frontier.last() == Some(&block) {
                        break;
                    }
                    frontier.push(block);
                }
                runner = dominators.immediate_dominator(current);
            }
        }
    }
    let positions: HashMap<_, _> = cfg
        .block_ids()
        .enumerate()
        .map(|(index, block)| (block, index))
        .collect();
    sets.into_iter()
        .map(|(block, mut members)| {
            members.sort_unstable_by_key(|member| positions[member]);
            (block, members)
        })
        .collect()
}
