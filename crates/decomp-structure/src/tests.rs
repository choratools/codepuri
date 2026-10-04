#![allow(clippy::unwrap_used)]

use super::*;

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

fn occurrences(region: &Region, result: &mut Vec<BlockId>) {
    match region {
        Region::While {
            evaluation,
            condition,
            body,
            ..
        } => {
            occurrences(evaluation, result);
            result.push(*condition);
            occurrences(body, result);
        }
        Region::Block(b) => result.push(*b),
        Region::Sequence(items) => {
            for item in items {
                occurrences(item, result);
            }
        }
        Region::If {
            header,
            then_region,
            else_region,
            ..
        } => {
            result.push(*header);
            occurrences(then_region, result);
            occurrences(else_region, result);
        }
    }
}

fn complete(cfg: &Cfg<()>, result: &StructuredCfg) {
    let mut blocks = Vec::new();
    occurrences(&result.root, &mut blocks);
    assert_eq!(blocks.len(), cfg.block_count());
    assert_eq!(
        blocks.into_iter().collect::<HashSet<_>>(),
        cfg.block_ids().collect()
    );
    assert_eq!(result.revision, cfg.revision());
    assert_eq!(result.entry, cfg.entry());
    assert_eq!(result.edges.len(), cfg.edge_count());
    for (id, record) in cfg.edge_ids().zip(&result.edges) {
        let edge = cfg.edge(id).unwrap();
        assert_eq!(record.id, id);
        assert_eq!(record.source, edge.source());
        assert_eq!(record.target, edge.target());
        assert_eq!(&record.kind, edge.kind());
    }
}

#[test]
fn terminal_and_straightline() {
    let cfg = Cfg::new(std::sync::Mutex::new(())); // payload needs no Clone
    assert_eq!(structure(&cfg).unwrap().root, Region::Block(cfg.entry()));
    let (cfg, ids) = graph(
        3,
        &[
            (0, 1, EdgeKind::Unconditional),
            (1, 2, EdgeKind::Unconditional),
        ],
    );
    let result = structure(&cfg).unwrap();
    assert_eq!(
        result.root,
        Region::Sequence(ids.iter().copied().map(Region::Block).collect())
    );
    complete(&cfg, &result);
}

#[test]
fn diamond_uses_polarity_not_insertion_order() {
    let (cfg, b) = graph(
        4,
        &[
            (0, 2, EdgeKind::BranchFalse),
            (0, 1, EdgeKind::BranchTrue),
            (1, 3, EdgeKind::Unconditional),
            (2, 3, EdgeKind::Unconditional),
        ],
    );
    let result = structure(&cfg).unwrap();
    assert_eq!(
        result.root,
        Region::Sequence(vec![
            Region::If {
                header: b[0],
                then_region: Box::new(Region::Block(b[1])),
                else_region: Box::new(Region::Block(b[2])),
                join: Some(b[3])
            },
            Region::Block(b[3])
        ])
    );
    complete(&cfg, &result);
}

#[test]
fn nested_diamonds_have_local_and_outer_joins() {
    let (cfg, b) = graph(
        7,
        &[
            (0, 1, EdgeKind::BranchTrue),
            (0, 5, EdgeKind::BranchFalse),
            (1, 2, EdgeKind::BranchTrue),
            (1, 3, EdgeKind::BranchFalse),
            (2, 4, EdgeKind::Unconditional),
            (3, 4, EdgeKind::Unconditional),
            (4, 6, EdgeKind::Unconditional),
            (5, 6, EdgeKind::Unconditional),
        ],
    );
    let result = structure(&cfg).unwrap();
    let inner = Region::If {
        header: b[1],
        then_region: Box::new(Region::Block(b[2])),
        else_region: Box::new(Region::Block(b[3])),
        join: Some(b[4]),
    };
    let outer = Region::If {
        header: b[0],
        then_region: Box::new(Region::Sequence(vec![inner, Region::Block(b[4])])),
        else_region: Box::new(Region::Block(b[5])),
        join: Some(b[6]),
    };
    assert_eq!(
        result.root,
        Region::Sequence(vec![outer, Region::Block(b[6])])
    );
    complete(&cfg, &result);
}

#[test]
fn empty_arm_and_parallel_branch_edges_are_preserved() {
    for target in [1, 2] {
        let mut edges = vec![
            (0, 1, EdgeKind::BranchTrue),
            (0, target, EdgeKind::BranchFalse),
        ];
        if target == 2 {
            edges.push((1, 2, EdgeKind::Unconditional));
        }
        let (cfg, b) = graph(target + 1, &edges);
        let result = structure(&cfg).unwrap();
        complete(&cfg, &result);
        let Region::Sequence(items) = result.root else {
            panic!("expected continuation");
        };
        let Region::If {
            else_region, join, ..
        } = &items[0]
        else {
            panic!("expected if");
        };
        assert_eq!(**else_region, Region::Sequence(vec![]));
        assert_eq!(*join, Some(b[target]));
    }
}

#[test]
fn distinct_terminal_arms_and_nested_early_return() {
    let (cfg, b) = graph(
        5,
        &[
            (0, 1, EdgeKind::BranchTrue),
            (0, 4, EdgeKind::BranchFalse),
            (1, 2, EdgeKind::BranchTrue),
            (1, 3, EdgeKind::BranchFalse),
        ],
    );
    let result = structure(&cfg).unwrap();
    complete(&cfg, &result);
    let Region::If {
        join,
        then_region,
        else_region,
        ..
    } = result.root
    else {
        panic!("expected if");
    };
    assert_eq!(join, None);
    assert_eq!(*else_region, Region::Block(b[4]));
    assert!(matches!(*then_region, Region::If { join: None, .. }));
}

#[test]
fn unreachable_is_reported_even_without_edges() {
    let (cfg, b) = graph(2, &[]);
    assert_eq!(
        structure(&cfg),
        Err(StructureError::UnreachableBlocks(vec![b[1]]))
    );
}

#[test]
fn exceptions_and_switches_are_not_silently_dropped() {
    for kind in [EdgeKind::Exception("error".into()), EdgeKind::Switch(42)] {
        let (cfg, _) = graph(2, &[(0, 1, kind.clone())]);
        assert_eq!(
            structure(&cfg),
            Err(StructureError::UnsupportedEdge {
                edge: cfg.edge_ids().next().unwrap(),
                kind
            })
        );
    }
}

#[test]
fn invalid_branch_shapes_are_rejected() {
    for edges in [
        vec![(0, 1, EdgeKind::BranchTrue)],
        vec![(0, 1, EdgeKind::BranchTrue), (0, 2, EdgeKind::BranchTrue)],
        vec![
            (0, 1, EdgeKind::Unconditional),
            (0, 2, EdgeKind::Unconditional),
        ],
    ] {
        let (cfg, b) = graph(if edges.len() == 1 { 2 } else { 3 }, &edges);
        assert_eq!(
            structure(&cfg),
            Err(StructureError::UnsupportedBranch {
                block: b[0],
                edges: cfg.block(b[0]).unwrap().outgoing().to_vec()
            })
        );
    }
}

#[test]
fn unsupported_cycles_are_rejected() {
    for (n, edges) in [
        (1, vec![(0, 0, EdgeKind::Unconditional)]),
        (
            4,
            vec![
                (0, 1, EdgeKind::BranchTrue),
                (0, 2, EdgeKind::BranchFalse),
                (1, 2, EdgeKind::Unconditional),
                (2, 1, EdgeKind::BranchTrue),
                (2, 3, EdgeKind::BranchFalse),
            ],
        ),
    ] {
        let (cfg, _) = graph(n, &edges);
        assert!(
            matches!(structure(&cfg),Err(StructureError::CyclicControlFlow(nodes)) if !nodes.is_empty())
        );
    }
}

#[test]
fn pretest_loops_preserve_polarity_and_coverage() {
    for polarity in [true, false] {
        let (cfg, b) = graph(
            5,
            &[
                (0, 1, EdgeKind::Unconditional),
                (1, 2, EdgeKind::Unconditional),
                (
                    2,
                    3,
                    if polarity {
                        EdgeKind::BranchTrue
                    } else {
                        EdgeKind::BranchFalse
                    },
                ),
                (
                    2,
                    4,
                    if polarity {
                        EdgeKind::BranchFalse
                    } else {
                        EdgeKind::BranchTrue
                    },
                ),
                (3, 1, EdgeKind::Unconditional),
            ],
        );
        let result = structure(&cfg).unwrap();
        complete(&cfg, &result);
        let Region::Sequence(items) = result.root else {
            panic!("sequence")
        };
        assert!(
            matches!(&items[1], Region::While { header, condition, body_on_true, exit, .. }
            if *header == b[1] && *condition == b[2] && *body_on_true == polarity && *exit == b[4])
        );
    }
}

#[test]
fn break_continue_multiple_entries_and_posttest_are_unsupported() {
    for (n, edges) in [
        // A second body exit is a break.
        (
            5,
            vec![
                (0, 1, EdgeKind::Unconditional),
                (1, 2, EdgeKind::BranchTrue),
                (1, 4, EdgeKind::BranchFalse),
                (2, 3, EdgeKind::BranchTrue),
                (2, 4, EdgeKind::BranchFalse),
                (3, 1, EdgeKind::Unconditional),
            ],
        ),
        // Two backedge sources require continue/multiple-latch handling.
        (
            5,
            vec![
                (0, 1, EdgeKind::Unconditional),
                (1, 2, EdgeKind::BranchTrue),
                (1, 4, EdgeKind::BranchFalse),
                (2, 3, EdgeKind::BranchTrue),
                (2, 1, EdgeKind::BranchFalse),
                (3, 1, EdgeKind::Unconditional),
            ],
        ),
        // External entry bypasses the loop header.
        (
            5,
            vec![
                (0, 1, EdgeKind::BranchTrue),
                (0, 2, EdgeKind::BranchFalse),
                (1, 2, EdgeKind::BranchTrue),
                (1, 4, EdgeKind::BranchFalse),
                (2, 3, EdgeKind::Unconditional),
                (3, 1, EdgeKind::Unconditional),
            ],
        ),
        // Posttest loop has a conditional latch.
        (
            4,
            vec![
                (0, 1, EdgeKind::Unconditional),
                (1, 2, EdgeKind::Unconditional),
                (2, 1, EdgeKind::BranchTrue),
                (2, 3, EdgeKind::BranchFalse),
            ],
        ),
    ] {
        let (cfg, _) = graph(n, &edges);
        assert!(
            matches!(structure(&cfg), Err(StructureError::CyclicControlFlow(_))),
            "{cfg:?}"
        );
    }
}

#[test]
fn acyclic_cross_branch_overlap_is_rejected() {
    let (cfg, _) = graph(
        5,
        &[
            (0, 1, EdgeKind::BranchTrue),
            (0, 2, EdgeKind::BranchFalse),
            (1, 3, EdgeKind::BranchTrue),
            (1, 4, EdgeKind::BranchFalse),
            (2, 3, EdgeKind::Unconditional),
        ],
    );
    assert!(matches!(
        structure(&cfg),
        Err(StructureError::OverlappingRegion(_))
    ));
}
