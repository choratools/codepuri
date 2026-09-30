use std::collections::HashSet;

use crate::cfg::Cfg;
use crate::ids::BlockId;

pub(crate) fn successors<T>(cfg: &Cfg<T>, block: BlockId) -> Vec<BlockId> {
    cfg.block(block)
        .into_iter()
        .flat_map(|record| record.outgoing())
        .filter_map(|edge| cfg.edge(*edge).ok().map(|record| record.target()))
        .collect()
}

pub(crate) fn predecessors<T>(cfg: &Cfg<T>, block: BlockId) -> Vec<BlockId> {
    cfg.block(block)
        .into_iter()
        .flat_map(|record| record.incoming())
        .filter_map(|edge| cfg.edge(*edge).ok().map(|record| record.source()))
        .collect()
}

/// Returns entry-reachable blocks in depth-first postorder.
pub fn postorder<T>(cfg: &Cfg<T>) -> Vec<BlockId> {
    let mut seen = HashSet::from([cfg.entry()]);
    let mut stack = vec![(cfg.entry(), false)];
    let mut order = Vec::new();
    while let Some((block, expanded)) = stack.pop() {
        if expanded {
            order.push(block);
            continue;
        }
        stack.push((block, true));
        if let Ok(record) = cfg.block(block) {
            for edge in record.outgoing().iter().rev() {
                if let Ok(edge) = cfg.edge(*edge) {
                    let successor = edge.target();
                    if seen.insert(successor) {
                        stack.push((successor, false));
                    }
                }
            }
        }
    }
    order
}

/// Returns entry-reachable blocks in reverse depth-first postorder.
pub fn reverse_postorder<T>(cfg: &Cfg<T>) -> Vec<BlockId> {
    let mut order = postorder(cfg);
    order.reverse();
    order
}
