//! Property tests for dominance and mutation invariants.

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;

use decomp_core::analysis::{dominance_frontier, dominators, post_dominators};
use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::BlockId;
use proptest::prelude::*;

fn must<T, E: Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("unexpected error: {error:?}"),
    }
}

fn graph_from_bits(node_count: usize, bits: &[bool]) -> Cfg<()> {
    let mut cfg = Cfg::new(());
    let mut blocks = vec![cfg.entry()];
    blocks.extend((1..node_count).map(|_| cfg.add_block(())));
    for (index, present) in bits.iter().copied().enumerate() {
        if present {
            let source = blocks[index / node_count];
            let target = blocks[index % node_count];
            must(cfg.add_edge(source, target, EdgeKind::Unconditional));
        }
    }
    cfg
}

fn reachable(cfg: &Cfg<()>) -> HashSet<BlockId> {
    let mut seen = HashSet::from([cfg.entry()]);
    let mut stack = vec![cfg.entry()];
    while let Some(block) = stack.pop() {
        for edge_id in must(cfg.block(block)).outgoing() {
            let target = must(cfg.edge(*edge_id)).target();
            if seen.insert(target) {
                stack.push(target);
            }
        }
    }
    seen
}

fn brute_dominators(cfg: &Cfg<()>) -> HashMap<BlockId, HashSet<BlockId>> {
    let nodes = reachable(cfg);
    let mut sets: HashMap<_, _> = nodes
        .iter()
        .copied()
        .map(|block| {
            let initial = if block == cfg.entry() {
                HashSet::from([block])
            } else {
                nodes.clone()
            };
            (block, initial)
        })
        .collect();
    loop {
        let mut changed = false;
        for block in nodes.iter().copied().filter(|block| *block != cfg.entry()) {
            let predecessors: Vec<_> = must(cfg.block(block))
                .incoming()
                .iter()
                .map(|edge_id| must(cfg.edge(*edge_id)).source())
                .filter(|source| nodes.contains(source))
                .collect();
            let mut next = nodes.clone();
            for predecessor in predecessors {
                next.retain(|candidate| sets[&predecessor].contains(candidate));
            }
            next.insert(block);
            if sets[&block] != next {
                sets.insert(block, next);
                changed = true;
            }
        }
        if !changed {
            return sets;
        }
    }
}

fn brute_frontier(
    cfg: &Cfg<()>,
    dom: &HashMap<BlockId, HashSet<BlockId>>,
) -> HashMap<BlockId, HashSet<BlockId>> {
    dom.keys()
        .copied()
        .map(|candidate| {
            let members = dom
                .keys()
                .copied()
                .filter(|block| {
                    must(cfg.block(*block))
                        .incoming()
                        .iter()
                        .map(|edge_id| must(cfg.edge(*edge_id)).source())
                        .any(|predecessor| {
                            dom.get(&predecessor)
                                .is_some_and(|set| set.contains(&candidate))
                        })
                        && (*block == candidate || !dom[block].contains(&candidate))
                })
                .collect();
            (candidate, members)
        })
        .collect()
}

fn brute_post_dominators(cfg: &Cfg<()>) -> HashMap<BlockId, HashSet<BlockId>> {
    let reachable = reachable(cfg);
    let successors: HashMap<_, Vec<_>> = reachable
        .iter()
        .copied()
        .map(|block| {
            let next = must(cfg.block(block))
                .outgoing()
                .iter()
                .map(|edge| must(cfg.edge(*edge)).target())
                .collect();
            (block, next)
        })
        .collect();
    let mut live: HashSet<_> = successors
        .iter()
        .filter_map(|(&block, next)| next.is_empty().then_some(block))
        .collect();
    loop {
        let previous = live.len();
        for (&block, next) in &successors {
            if next.iter().any(|target| live.contains(target)) {
                live.insert(block);
            }
        }
        if live.len() == previous {
            break;
        }
    }
    let mut sets: HashMap<_, _> = live
        .iter()
        .copied()
        .map(|block| (block, live.clone()))
        .collect();
    loop {
        let mut changed = false;
        for &block in &live {
            let mut next = if successors[&block].is_empty() {
                HashSet::new()
            } else {
                live.clone()
            };
            for successor in successors[&block]
                .iter()
                .filter(|block| live.contains(block))
            {
                next.retain(|candidate| sets[successor].contains(candidate));
            }
            next.insert(block);
            if next != sets[&block] {
                sets.insert(block, next);
                changed = true;
            }
        }
        if !changed {
            return sets;
        }
    }
}

prop_compose! {
    fn graph_strategy()(node_count in 1usize..=12)
        (bits in prop::collection::vec(any::<bool>(), node_count * node_count), node_count in Just(node_count))
        -> Cfg<()> {
        graph_from_bits(node_count, &bits)
    }
}

proptest! {
    #[test]
    fn post_dominators_match_brute_force(cfg in graph_strategy()) {
        let expected = brute_post_dominators(&cfg);
        let actual = post_dominators(&cfg);
        for block in cfg.block_ids() {
            for candidate in cfg.block_ids() {
                prop_assert_eq!(actual.post_dominates(candidate, block),
                    expected.get(&block).is_some_and(|set| set.contains(&candidate)));
            }
            let parent = expected.get(&block).and_then(|set| {
                set.iter().copied().filter(|&candidate| candidate != block)
                    .find(|candidate| set.iter().all(|other| {
                        *other == block || other == candidate || expected[candidate].contains(other)
                    }))
            });
            prop_assert_eq!(actual.immediate_post_dominator(block), parent);
        }
    }

    #[test]
    fn dominators_match_brute_force(cfg in graph_strategy()) {
        let expected = brute_dominators(&cfg);
        let actual = dominators(&cfg);

        for (&block, expected_set) in &expected {
            for &candidate in expected.keys() {
                prop_assert_eq!(actual.dominates(candidate, block), expected_set.contains(&candidate));
            }
        }
    }

    #[test]
    fn dominance_frontiers_match_brute_force(cfg in graph_strategy()) {
        let expected_dom = brute_dominators(&cfg);
        let expected = brute_frontier(&cfg, &expected_dom);
        let actual = dominance_frontier(&cfg, &dominators(&cfg));

        for (&block, expected_set) in &expected {
            let actual_set: HashSet<_> = actual.get(&block).into_iter().flatten().copied().collect();
            prop_assert_eq!(actual_set, expected_set.clone());
        }
    }

    #[test]
    fn random_mutations_preserve_cfg_validation(operations in prop::collection::vec((any::<u8>(), any::<usize>()), 1..80)) {
        let mut cfg = Cfg::new(0usize);
        for (opcode, selector) in operations {
            let blocks: Vec<_> = cfg.block_ids().collect();
            match opcode % 4 {
                0 => {
                    let new_block = cfg.add_block(selector);
                    let source = blocks[selector % blocks.len()];
                    must(cfg.add_edge(source, new_block, EdgeKind::Unconditional));
                }
                1 => {
                    let block = blocks[selector % blocks.len()];
                    must(cfg.split_block(block, selector));
                }
                2 => {
                    let edges: Vec<_> = cfg.edge_ids().collect();
                    if let Some(&edge) = edges.get(selector % edges.len().max(1)) {
                        let target = blocks[(selector / 3) % blocks.len()];
                        must(cfg.redirect_edge(edge, target));
                    }
                }
                3 => {
                    let removable: Vec<_> = blocks.into_iter().filter(|block| *block != cfg.entry()).collect();
                    if let Some(&block) = removable.get(selector % removable.len().max(1)) {
                        must(cfg.remove_block(block));
                    }
                }
                _ => unreachable!("modulo four is exhaustive"),
            }
            prop_assert!(cfg.validate().is_ok());
        }
    }
}
