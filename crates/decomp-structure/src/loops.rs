//! Reduce supported natural loops before invoking the DAG postdominator code.
//! The opt-in `loop_control` module handles explicit body transfers and tests
//! at their original locations. Parent integration should use that typed API
//! for abrupt/posttest loops, rather than relaxing this legacy While grammar.
use super::*;
use decomp_core::analysis::{dominators, natural_loops};

fn order(successors: &HashMap<BlockId, Vec<BlockId>>) -> Option<Vec<BlockId>> {
    let mut degree: HashMap<_, usize> = successors.keys().map(|&b| (b, 0)).collect();
    for targets in successors.values() {
        for t in targets {
            *degree.get_mut(t)? += 1;
        }
    }
    let mut queue: VecDeque<_> = degree
        .iter()
        .filter_map(|(&b, &d)| (d == 0).then_some(b))
        .collect();
    let mut result = Vec::new();
    while let Some(b) = queue.pop_front() {
        result.push(b);
        for t in &successors[&b] {
            let d = degree.get_mut(t)?;
            *d -= 1;
            if *d == 0 {
                queue.push_back(*t);
            }
        }
    }
    (result.len() == successors.len()).then_some(result)
}

pub(super) fn structure<T>(
    cfg: &Cfg<T>,
    blocks: &[BlockId],
    successors: &HashMap<BlockId, Vec<BlockId>>,
    edges: Vec<EdgeProvenance>,
) -> Result<StructuredCfg, StructureError> {
    let unsupported = || StructureError::CyclicControlFlow(blocks.to_vec());
    let loops = natural_loops(cfg, &dominators(cfg));
    if loops.is_empty() {
        return Err(unsupported());
    }
    // Analysis returns one natural loop per backedge. Union equal headers
    // before checking nesting: disjoint latch paths are one loop, not overlap.
    let mut grouped: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();
    for lp in loops {
        grouped.entry(lp.header()).or_default().extend(lp.blocks());
    }
    let mut loops: Vec<_> = grouped.into_iter().collect();
    for (i, (_, a)) in loops.iter().enumerate() {
        for (_, b) in &loops[i + 1..] {
            if !a.is_disjoint(b) && !a.is_subset(b) && !b.is_subset(a) {
                return Err(unsupported());
            }
        }
    }
    // Inner loops become atomic nodes in the enclosing body's DAG.
    loops.sort_by_key(|(_, members)| members.len());
    let mut replacements = HashMap::new();
    let mut collapsed = successors.clone();
    for (header, members) in loops {
        for &b in &members {
            for &e in cfg.block(b)?.incoming() {
                if !members.contains(&cfg.edge(e)?.source()) && b != header {
                    return Err(unsupported());
                }
            }
        }
        let mut chain = Vec::new();
        let mut prefix = HashSet::new();
        let mut condition = header;
        while let [next] = collapsed[&condition].as_slice() {
            if !members.contains(next)
                || prefix.contains(next)
                || *next == header
                || cfg.block(*next)?.incoming().len() != 1
            {
                return Err(unsupported());
            }
            chain.push(condition);
            prefix.insert(condition);
            condition = *next;
        }
        let [yes, no] = collapsed[&condition].as_slice() else {
            return Err(unsupported());
        };
        let (body_entry, exit, body_on_true) = match (members.contains(yes), members.contains(no)) {
            (true, false) => (*yes, *no, true),
            (false, true) => (*no, *yes, false),
            _ => return Err(unsupported()),
        };
        let evaluation_members: HashSet<_> = chain.iter().copied().chain([condition]).collect();
        if evaluation_members.contains(&body_entry) {
            return Err(unsupported());
        }
        let mut body_graph = HashMap::new();
        for &b in &members {
            if evaluation_members.contains(&b) || !collapsed.contains_key(&b) {
                continue;
            }
            let targets = &collapsed[&b];
            if targets.as_slice() == [header] {
                // Keep each original latch block. Its original edge identity
                // remains in provenance, including the Phi predecessor source.
                body_graph.insert(b, Vec::new());
            } else {
                if targets.is_empty()
                    || targets
                        .iter()
                        .any(|t| !members.contains(t) || evaluation_members.contains(t))
                {
                    return Err(unsupported());
                }
                body_graph.insert(b, targets.clone());
            }
        }
        let body_order = order(&body_graph).ok_or_else(unsupported)?;
        let joins = postdom::joins(&body_order, &body_graph);
        let mut seen = HashSet::new();
        let mut body = build(body_entry, None, &body_graph, &joins, &mut seen)?;
        if seen.len() != body_graph.len() {
            return Err(unsupported());
        }
        substitute(&mut body, &mut replacements);
        replacements.insert(
            header,
            Region::While {
                header,
                evaluation: Box::new(Region::Sequence(
                    chain.into_iter().map(Region::Block).collect(),
                )),
                condition,
                body_on_true,
                body: Box::new(body),
                exit,
            },
        );
        for &b in &members {
            collapsed.remove(&b);
        }
        collapsed.insert(header, vec![exit]);
    }
    let outer_order = order(&collapsed).ok_or_else(unsupported)?;
    let joins = postdom::joins(&outer_order, &collapsed);
    let mut seen = HashSet::new();
    let mut root = build(cfg.entry(), None, &collapsed, &joins, &mut seen)?;
    if seen.len() != collapsed.len() {
        return Err(unsupported());
    }
    substitute(&mut root, &mut replacements);
    if !replacements.is_empty() {
        return Err(unsupported());
    }
    Ok(StructuredCfg {
        entry: cfg.entry(),
        revision: cfg.revision(),
        root,
        edges,
    })
}

fn substitute(root: &mut Region, replacements: &mut HashMap<BlockId, Region>) {
    let mut pending = vec![root];
    while let Some(region) = pending.pop() {
        match region {
            Region::Block(b) => {
                if let Some(r) = replacements.remove(b) {
                    *region = r;
                }
            }
            Region::Sequence(rs) => {
                pending.extend(rs);
            }
            Region::If {
                then_region,
                else_region,
                ..
            } => {
                pending.extend([then_region.as_mut(), else_region.as_mut()]);
            }
            // Replacements already contain fully expanded inner bodies.
            Region::While { .. } => {}
        }
    }
}
