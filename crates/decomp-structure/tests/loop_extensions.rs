//! Lossless nested/multi-latch loop grammar and bounded execution checks.
#![allow(clippy::unwrap_used)]
use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::BlockId;
use decomp_structure::{Region, StructureError, structure};
use std::collections::HashSet;

fn graph(n: usize, edges: &[(usize, usize, EdgeKind)]) -> (Cfg<()>, Vec<BlockId>) {
    let mut cfg = Cfg::new(());
    let mut ids = vec![cfg.entry()];
    for _ in 1..n {
        ids.push(cfg.add_block(()));
    }
    for (a, b, kind) in edges {
        cfg.add_edge(ids[*a], ids[*b], kind.clone()).unwrap();
    }
    (cfg, ids)
}

fn blocks(region: &Region, out: &mut Vec<BlockId>) {
    match region {
        Region::Block(b) => out.push(*b),
        Region::Sequence(rs) => {
            for r in rs {
                blocks(r, out);
            }
        }
        Region::If {
            header,
            then_region,
            else_region,
            ..
        } => {
            out.push(*header);
            blocks(then_region, out);
            blocks(else_region, out);
        }
        Region::While {
            evaluation,
            condition,
            body,
            ..
        } => {
            blocks(evaluation, out);
            out.push(*condition);
            blocks(body, out);
        }
    }
}

fn complete(cfg: &Cfg<()>, root: &Region) {
    let mut emitted = Vec::new();
    blocks(root, &mut emitted);
    assert_eq!(emitted.len(), cfg.block_count());
    assert_eq!(
        emitted.into_iter().collect::<HashSet<_>>(),
        cfg.block_ids().collect()
    );
}

#[test]
fn nested_loops_with_two_outer_latches_preserve_every_edge() {
    use EdgeKind::*;
    let (cfg, ids) = graph(
        8,
        &[
            (0, 1, BranchTrue),
            (0, 7, BranchFalse),
            (1, 2, BranchTrue),
            (1, 3, BranchFalse),
            (2, 1, Unconditional),
            (3, 4, BranchTrue),
            (3, 5, BranchFalse),
            (4, 0, Unconditional),
            (5, 6, Unconditional),
            (6, 0, Unconditional),
        ],
    );
    let revision = cfg.revision();
    let result = structure(&cfg).unwrap();
    complete(&cfg, &result.root);
    assert_eq!(cfg.revision(), revision);
    assert_eq!(result.revision, revision);
    assert_eq!(result.edges.len(), cfg.edge_count());
    for (id, provenance) in cfg.edge_ids().zip(&result.edges) {
        let edge = cfg.edge(id).unwrap();
        assert_eq!(provenance.id, id);
        assert_eq!(provenance.source, edge.source());
        assert_eq!(provenance.target, edge.target());
        assert_eq!(&provenance.kind, edge.kind());
    }
    let Region::Sequence(top) = &result.root else {
        panic!("outer sequence")
    };
    assert_eq!(top[1], Region::Block(ids[7]));
    let Region::While { body, header, .. } = &top[0] else {
        panic!("outer loop")
    };
    assert_eq!(*header, ids[0]);
    let Region::Sequence(body) = body.as_ref() else {
        panic!("outer body")
    };
    assert!(matches!(&body[0], Region::While { header, .. } if *header == ids[1]));
    // Distinct latch sources must stay distinct for header Phi inputs.
    let incoming: HashSet<_> = result
        .edges
        .iter()
        .filter(|e| e.target == ids[0])
        .map(|e| e.source)
        .collect();
    assert_eq!(incoming, HashSet::from([ids[4], ids[6]]));
    // Compare actual visit order and termination against the original CFG.
    // Header visits model Phi evaluation once per entry, not once per latch
    // spelling; every adjacent trace pair identifies its original predecessor.
    let mut raw_counts = std::collections::HashMap::new();
    let mut raw = Vec::new();
    let mut current = cfg.entry();
    loop {
        assert!(raw.len() < 100, "CFG must terminate");
        raw.push(current);
        let choice = decision(current, &ids, &mut raw_counts);
        let outgoing = cfg.block(current).unwrap().outgoing();
        if outgoing.is_empty() {
            break;
        }
        current = outgoing
            .iter()
            .map(|e| cfg.edge(*e).unwrap())
            .find(|e| {
                *e.kind() == Unconditional
                    || *e.kind() == if choice { BranchTrue } else { BranchFalse }
            })
            .unwrap()
            .target();
    }
    let mut structured = Vec::new();
    execute(
        &result.root,
        &ids,
        &mut std::collections::HashMap::new(),
        &mut structured,
    );
    assert_eq!(structured, raw);
    assert_eq!(raw_counts[&ids[0]], 4);
    assert_eq!(raw_counts[&ids[1]], 6);
}

fn decision(
    b: BlockId,
    ids: &[BlockId],
    visits: &mut std::collections::HashMap<BlockId, usize>,
) -> bool {
    let n = visits.entry(b).or_default();
    *n += 1;
    if b == ids[0] { *n <= 3 } else { *n % 2 == 1 }
}

fn execute(
    r: &Region,
    ids: &[BlockId],
    visits: &mut std::collections::HashMap<BlockId, usize>,
    trace: &mut Vec<BlockId>,
) {
    assert!(trace.len() < 100, "structured execution must terminate");
    match r {
        Region::Block(b) => {
            trace.push(*b);
            decision(*b, ids, visits);
        }
        Region::Sequence(rs) => {
            for r in rs {
                execute(r, ids, visits, trace);
            }
        }
        Region::If {
            header,
            then_region,
            else_region,
            ..
        } => {
            trace.push(*header);
            execute(
                if decision(*header, ids, visits) {
                    then_region
                } else {
                    else_region
                },
                ids,
                visits,
                trace,
            );
        }
        Region::While {
            evaluation,
            condition,
            body_on_true,
            body,
            ..
        } => loop {
            assert!(trace.len() < 100, "loop must terminate");
            execute(evaluation, ids, visits, trace);
            trace.push(*condition);
            if decision(*condition, ids, visits) != *body_on_true {
                break;
            }
            execute(body, ids, visits, trace);
        },
    }
}

#[test]
fn unsupported_cycles_and_abrupt_edges_are_not_silently_truncated() {
    use EdgeKind::*;
    for (n, edges) in [
        (
            4,
            vec![
                (0, 1, BranchTrue),
                (0, 2, BranchFalse),
                (1, 2, Unconditional),
                (2, 1, BranchTrue),
                (2, 3, BranchFalse),
            ],
        ),
        // Conditional continue: needs an explicit edge-transfer region.
        (
            4,
            vec![
                (0, 1, BranchTrue),
                (0, 3, BranchFalse),
                (1, 0, BranchTrue),
                (1, 2, BranchFalse),
                (2, 0, Unconditional),
            ],
        ),
        // Body break shares the normal exit.
        (
            4,
            vec![
                (0, 1, BranchTrue),
                (0, 3, BranchFalse),
                (1, 3, BranchTrue),
                (1, 2, BranchFalse),
                (2, 0, Unconditional),
            ],
        ),
        // Body return is outside the natural-loop membership.
        (
            4,
            vec![
                (0, 1, BranchTrue),
                (0, 3, BranchFalse),
                (1, 2, BranchTrue),
                (1, 0, BranchFalse),
            ],
        ),
        // Posttest loop is deliberately outside Region::While semantics.
        (
            3,
            vec![
                (0, 1, Unconditional),
                (1, 0, BranchTrue),
                (1, 2, BranchFalse),
            ],
        ),
    ] {
        let (cfg, _) = graph(n, &edges);
        assert!(matches!(
            structure(&cfg),
            Err(StructureError::CyclicControlFlow(_))
        ));
    }
    for kind in [Switch(42), Exception("E".into())] {
        let (cfg, _) = graph(3, &[(0, 1, BranchTrue), (0, 2, BranchFalse), (1, 0, kind)]);
        assert!(matches!(
            structure(&cfg),
            Err(StructureError::UnsupportedEdge { .. })
        ));
    }
}
