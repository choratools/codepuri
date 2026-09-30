use std::collections::HashSet;

use crate::cfg::Cfg;

use super::dominators;
use super::traversal::reverse_postorder;

/// Returns whether every reachable cyclic region has one entry header.
pub fn is_reducible<T>(cfg: &Cfg<T>) -> bool {
    let order = reverse_postorder(cfg);
    let reachable: HashSet<_> = order.iter().copied().collect();
    let positions: std::collections::HashMap<_, _> = order
        .iter()
        .copied()
        .enumerate()
        .map(|(position, block)| (block, position))
        .collect();
    let dominance = dominators(cfg);
    cfg.edge_ids()
        .filter_map(|edge| cfg.edge(edge).ok())
        .filter(|edge| reachable.contains(&edge.source()) && reachable.contains(&edge.target()))
        .filter(|edge| positions[&edge.target()] <= positions[&edge.source()])
        .all(|edge| dominance.dominates(edge.target(), edge.source()))
}
