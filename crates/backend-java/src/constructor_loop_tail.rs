//! Bounded structural placements of shared tails inside one natural loop.
//!
//! This does not clone SSA definitions. Virtual placements project to the
//! canonical original block/instruction and selected incoming edge. A header
//! is never copied, so each executed virtual path is an original CFG path.
use super::*;
use crate::loop_control_regions::{ControlCfg, ControlRegion, TransferKind};
use decomp_core::{
    analysis::{dominators, is_reducible, natural_loops},
    cfg::Cfg,
    ids::{BlockId, EdgeId},
};

struct Placements {
    graph: Cfg<BlockId>,
    blocks: HashMap<BlockId, BlockId>,
    edges: HashMap<EdgeId, EdgeId>,
}

fn expand<T>(original: &Cfg<T>, suffix: &HashSet<BlockId>) -> Result<Placements, Unsupported> {
    let fail =
        || Unsupported::ControlFlow("unproved constructor natural-loop tail placement".into());
    original.validate().map_err(|_| fail())?;
    if original.block_count() > 512
        || !is_reducible(original)
        || original.edge_ids().any(|e| {
            original
                .edge(e)
                .is_ok_and(|x| matches!(x.kind(), EdgeKind::Exception(_)))
        })
    {
        return Err(fail());
    }
    let loops = natural_loops(original, &dominators(original));
    let headers: HashSet<_> = loops.iter().map(|l| l.header()).collect();
    // Nested/multiple loops require a separate placement proof.
    if headers.len() != 1 {
        return Err(fail());
    }
    let header = *headers.iter().next().ok_or_else(fail)?;
    let members: HashSet<_> = loops
        .iter()
        .flat_map(|l| l.blocks().iter().copied())
        .collect();
    if !suffix.contains(&header) || !members.is_subset(suffix) {
        return Err(fail());
    }
    for &b in &members {
        if b == header {
            continue;
        }
        if original
            .block(b)
            .map_err(|_| fail())?
            .incoming()
            .iter()
            .any(|e| {
                original
                    .edge(*e)
                    .is_ok_and(|x| !members.contains(&x.source()))
            })
        {
            return Err(fail());
        }
    }
    let mut graph = Cfg::new(original.entry());
    let mut copies = HashMap::from([(original.entry(), graph.entry())]);
    let mut blocks = HashMap::from([(graph.entry(), original.entry())]);
    for b in original.block_ids().filter(|b| *b != original.entry()) {
        let copy = graph.add_block(b);
        copies.insert(b, copy);
        blocks.insert(copy, b);
    }
    let mut edges = HashMap::new();
    for e in original.edge_ids() {
        let x = original.edge(e).map_err(|_| fail())?;
        let copy = graph
            .add_edge(copies[&x.source()], copies[&x.target()], x.kind().clone())
            .map_err(|_| fail())?;
        edges.insert(copy, e);
    }
    let mut count = 0;
    loop {
        // Stable graph order, not HashSet order. Header/backedges are boundaries.
        let shared = graph.block_ids().find(|b| {
            let origin = blocks[b];
            origin != header
                && members.contains(&origin)
                && graph.block(*b).is_ok_and(|n| n.incoming().len() > 1)
        });
        let Some(shared) = shared else {
            break;
        };
        let incoming = graph.block(shared).map_err(|_| fail())?.incoming().to_vec();
        let outgoing = graph.block(shared).map_err(|_| fail())?.outgoing().to_vec();
        for selected in incoming.into_iter().skip(1) {
            count += 1;
            if count > 512 || graph.edge_ids().count() > 4096 {
                return Err(fail());
            }
            let origin = blocks[&shared];
            let copy = graph.add_block(origin);
            blocks.insert(copy, origin);
            graph.redirect_edge(selected, copy).map_err(|_| fail())?;
            for &e in &outgoing {
                let x = graph.edge(e).map_err(|_| fail())?;
                let target = x.target();
                let kind = x.kind().clone();
                let copied = graph.add_edge(copy, target, kind).map_err(|_| fail())?;
                edges.insert(copied, edges[&e]);
            }
        }
    }
    graph.validate().map_err(|_| fail())?;
    // Verify every placement retains the entire original outgoing sequence.
    // Incoming selected-edge identity is retained even at cloned Phi blocks.
    for b in graph.block_ids() {
        let source = blocks[&b];
        let outgoing = graph.block(b).map_err(|_| fail())?.outgoing();
        let original_out = original.block(source).map_err(|_| fail())?.outgoing();
        if outgoing.len() != original_out.len() {
            return Err(fail());
        }
        for (&copy, &origin) in outgoing.iter().zip(original_out) {
            let x = graph.edge(copy).map_err(|_| fail())?;
            let y = original.edge(origin).map_err(|_| fail())?;
            if edges[&copy] != origin || blocks[&x.target()] != y.target() || x.kind() != y.kind() {
                return Err(fail());
            }
        }
    }
    Ok(Placements {
        graph,
        blocks,
        edges,
    })
}

pub(super) fn structure<T>(
    original: &Cfg<T>,
    suffix: &HashSet<BlockId>,
) -> Result<ControlCfg, Unsupported> {
    let placements = expand(original, suffix)?;
    let mut plan = crate::loop_control_regions::structure(&placements.graph)
        .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
    fn restore(
        r: &mut ControlRegion,
        bs: &HashMap<BlockId, BlockId>,
        es: &HashMap<EdgeId, EdgeId>,
    ) {
        match r {
            ControlRegion::Block(b) => *b = bs[b],
            ControlRegion::Sequence(items) => {
                for item in items {
                    restore(item, bs, es);
                }
            }
            ControlRegion::If {
                header,
                then_edge,
                then_region,
                else_edge,
                else_region,
            } => {
                *header = bs[header];
                *then_edge = es[then_edge];
                *else_edge = es[else_edge];
                restore(then_region, bs, es);
                restore(else_region, bs, es);
            }
            ControlRegion::Loop { header, body, exit } => {
                *header = bs[header];
                *exit = exit.map(|b| bs[&b]);
                restore(body, bs, es);
            }
            ControlRegion::Transfer { edge, kind } => {
                *edge = es[edge];
                match kind {
                    TransferKind::Break { header } | TransferKind::Continue { header } => {
                        *header = bs[header]
                    }
                }
            }
        }
    }
    restore(&mut plan.root, &placements.blocks, &placements.edges);
    plan.entry = original.entry();
    plan.revision = original.revision();
    plan.edges = original
        .edge_ids()
        .map(|id| {
            let e = original
                .edge(id)
                .map_err(|_| Unsupported::ControlFlow("missing original loop edge".into()))?;
            Ok(decomp_structure::EdgeProvenance {
                id,
                source: e.source(),
                target: e.target(),
                kind: e.kind().clone(),
            })
        })
        .collect::<Result<_, Unsupported>>()?;
    Ok(plan)
}

#[cfg(test)]
#[path = "constructor_loop_tail_tests.rs"]
mod tests;
