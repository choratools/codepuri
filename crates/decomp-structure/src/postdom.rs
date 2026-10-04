//! DAG-specific immediate postdominators. Successors are already attached to
//! the tree when their predecessor is processed in reverse topological order.
//! The intersection of their ancestor chains is their LCA. A virtual exit
//! parents every terminal. Binary lifting bounds each intersection by log B,
//! independent of chain depth; no transitive node sets are materialized.
use decomp_core::ids::BlockId;
use std::collections::HashMap;

pub(crate) fn joins(
    order: &[BlockId],
    successors: &HashMap<BlockId, Vec<BlockId>>,
) -> HashMap<BlockId, Option<BlockId>> {
    let n = order.len();
    let index: HashMap<_, _> = order.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let levels = (usize::BITS - n.leading_zeros()) as usize;
    let mut up = vec![vec![n; n + 1]; levels];
    let mut depth = vec![0usize; n + 1];
    let mut result = HashMap::with_capacity(n);
    for (i, &block) in order.iter().enumerate().rev() {
        let mut targets = successors[&block].iter().map(|b| index[b]);
        let mut parent = targets.next().unwrap_or(n);
        for target in targets {
            parent = lca(parent, target, &depth, &up);
        }
        depth[i] = depth[parent] + 1;
        up[0][i] = parent;
        for level in 1..levels {
            up[level][i] = up[level - 1][up[level - 1][i]];
        }
        result.insert(block, order.get(parent).copied());
    }
    result
}

fn lca(mut a: usize, mut b: usize, depth: &[usize], up: &[Vec<usize>]) -> usize {
    if depth[a] < depth[b] {
        std::mem::swap(&mut a, &mut b);
    }
    let delta = depth[a] - depth[b];
    for (level, ancestors) in up.iter().enumerate() {
        if delta & (1usize << level) != 0 {
            a = ancestors[a];
        }
    }
    if a == b {
        return a;
    }
    for ancestors in up.iter().rev() {
        if ancestors[a] != ancestors[b] {
            a = ancestors[a];
            b = ancestors[b];
        }
    }
    up[0][a]
}

/// Original set-based join selection, retained only as a differential oracle.
#[cfg(test)]
pub(crate) fn original_joins(
    order: &[BlockId],
    successors: &HashMap<BlockId, Vec<BlockId>>,
) -> HashMap<BlockId, Option<BlockId>> {
    use std::collections::HashSet;
    let mut postdom: HashMap<BlockId, HashSet<Option<BlockId>>> = HashMap::new();
    for &block in order.iter().rev() {
        let targets = &successors[&block];
        let mut set = targets
            .first()
            .map_or_else(|| HashSet::from([None]), |first| postdom[first].clone());
        for target in targets.iter().skip(1) {
            set.retain(|candidate| postdom[target].contains(candidate));
        }
        set.insert(Some(block));
        postdom.insert(block, set);
    }
    order
        .iter()
        .map(|&block| {
            let join = postdom[&block]
                .iter()
                .filter_map(|b| *b)
                .filter(|b| *b != block)
                .max_by_key(|b| postdom[b].len());
            (block, join)
        })
        .collect()
}
