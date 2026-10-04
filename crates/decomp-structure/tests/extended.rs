//! Coverage for the registered extended structuring API.
#![allow(clippy::unwrap_used)]

use decomp_structure::extended;
pub use decomp_structure::{EdgeProvenance, StructureError};

use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::BlockId;
use extended::{ExtendedRegion, ExtendedStructuredCfg, structure_extended};
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

fn occurrences(r: &ExtendedRegion, blocks: &mut Vec<BlockId>) {
    match r {
        ExtendedRegion::Block(b) => blocks.push(*b),
        ExtendedRegion::Sequence(items) => {
            for r in items {
                occurrences(r, blocks);
            }
        }
        ExtendedRegion::If {
            header,
            then_region,
            else_region,
            ..
        } => {
            blocks.push(*header);
            occurrences(then_region, blocks);
            occurrences(else_region, blocks);
        }
        ExtendedRegion::Switch { header, arms, .. } => {
            blocks.push(*header);
            for arm in arms {
                occurrences(&arm.region, blocks);
            }
        }
    }
}

fn complete(cfg: &Cfg<()>, result: &ExtendedStructuredCfg) {
    let mut blocks = Vec::new();
    occurrences(&result.root, &mut blocks);
    assert_eq!(blocks.len(), cfg.block_count());
    assert_eq!(
        blocks.into_iter().collect::<HashSet<_>>(),
        cfg.block_ids().collect()
    );
    assert_eq!(result.entry, cfg.entry());
    assert_eq!(result.revision, cfg.revision());
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
fn grouped_labels_default_and_empty_arm_preserve_edges() {
    let (cfg, b) = graph(
        4,
        &[
            (0, 1, EdgeKind::Switch(9)),
            (0, 1, EdgeKind::Switch(2)),
            (0, 3, EdgeKind::Switch(7)),
            (0, 2, EdgeKind::Unconditional),
            (1, 3, EdgeKind::Unconditional),
            (2, 3, EdgeKind::Unconditional),
        ],
    );
    let result = structure_extended(&cfg).unwrap();
    complete(&cfg, &result);
    let ExtendedRegion::Sequence(items) = &result.root else {
        panic!("expected sequence")
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[1], ExtendedRegion::Block(b[3]));
    let ExtendedRegion::Switch { header, arms, join } = &items[0] else {
        panic!("expected switch")
    };
    assert_eq!(*header, b[0]);
    assert_eq!(*join, Some(b[3]));
    assert_eq!(arms.len(), 3);
    assert_eq!(arms[0].target, b[1]);
    assert_eq!(arms[0].labels, vec![9, 2]);
    assert!(!arms[0].is_default);
    assert_eq!(arms[1].region, ExtendedRegion::Sequence(vec![]));
    assert!(arms[2].is_default);
    assert!(arms[2].labels.is_empty());
}

#[test]
fn default_can_share_a_case_target() {
    let (cfg, _) = graph(
        2,
        &[
            (0, 1, EdgeKind::Switch(u64::MAX)),
            (0, 1, EdgeKind::Unconditional),
        ],
    );
    let result = structure_extended(&cfg).unwrap();
    complete(&cfg, &result);
    let ExtendedRegion::Sequence(items) = result.root else {
        panic!("expected sequence")
    };
    let ExtendedRegion::Switch { arms, .. } = &items[0] else {
        panic!("expected switch")
    };
    assert_eq!(arms.len(), 1);
    assert_eq!(arms[0].labels, vec![u64::MAX]);
    assert!(arms[0].is_default);
}

#[test]
fn nested_conditional_and_separate_terminal_switch_arms() {
    let (cfg, b) = graph(
        6,
        &[
            (0, 1, EdgeKind::Switch(0)),
            (0, 5, EdgeKind::Switch(1)),
            (1, 3, EdgeKind::BranchFalse),
            (1, 2, EdgeKind::BranchTrue),
            (2, 4, EdgeKind::Unconditional),
            (3, 4, EdgeKind::Unconditional),
        ],
    );
    let result = structure_extended(&cfg).unwrap();
    complete(&cfg, &result);
    let ExtendedRegion::Switch { arms, join, .. } = &result.root else {
        panic!("expected switch")
    };
    assert_eq!(*join, None);
    assert!(arms.iter().all(|a| !a.is_default));
    let ExtendedRegion::Sequence(items) = &arms[0].region else {
        panic!("expected sequence")
    };
    assert_eq!(
        items[0],
        ExtendedRegion::If {
            header: b[1],
            then_region: Box::new(ExtendedRegion::Block(b[2])),
            else_region: Box::new(ExtendedRegion::Block(b[3])),
            join: Some(b[4])
        }
    );
    assert_eq!(arms[1].region, ExtendedRegion::Block(b[5]));
}

#[test]
fn malformed_dispatch_returns_typed_error_without_dropping_edges() {
    for kinds in [
        vec![EdgeKind::Switch(3), EdgeKind::Switch(3)],
        vec![EdgeKind::Switch(3), EdgeKind::BranchTrue],
        vec![
            EdgeKind::Switch(3),
            EdgeKind::Unconditional,
            EdgeKind::Unconditional,
        ],
    ] {
        let edges: Vec<_> = kinds
            .into_iter()
            .enumerate()
            .map(|(i, k)| (0, i + 1, k))
            .collect();
        let (cfg, b) = graph(edges.len() + 1, &edges);
        let revision = cfg.revision();
        assert_eq!(
            structure_extended(&cfg),
            Err(StructureError::UnsupportedBranch {
                block: b[0],
                edges: cfg.block(b[0]).unwrap().outgoing().to_vec(),
            })
        );
        assert_eq!(cfg.revision(), revision);
    }
}

#[test]
fn partial_arm_overlap_is_rejected() {
    let (cfg, _) = graph(
        5,
        &[
            (0, 1, EdgeKind::Switch(1)),
            (0, 2, EdgeKind::Switch(2)),
            (0, 4, EdgeKind::Switch(3)),
            (1, 3, EdgeKind::Unconditional),
            (2, 3, EdgeKind::Unconditional),
        ],
    );
    assert!(matches!(
        structure_extended(&cfg),
        Err(StructureError::OverlappingRegion(_))
    ));
}

#[test]
fn exceptions_reducible_and_irreducible_cycles_are_explicit_errors() {
    let (cfg, _) = graph(2, &[(0, 1, EdgeKind::Exception("E".into()))]);
    assert!(matches!(
        structure_extended(&cfg),
        Err(StructureError::UnsupportedEdge {
            kind: EdgeKind::Exception(_),
            ..
        })
    ));
    let (cfg, _) = graph(
        3,
        &[
            (0, 1, EdgeKind::BranchTrue),
            (0, 2, EdgeKind::BranchFalse),
            (1, 0, EdgeKind::Unconditional),
        ],
    );
    assert!(matches!(
        structure_extended(&cfg),
        Err(StructureError::CyclicControlFlow(_))
    ));
    let (cfg, _) = graph(
        4,
        &[
            (0, 1, EdgeKind::BranchTrue),
            (0, 2, EdgeKind::BranchFalse),
            (1, 2, EdgeKind::Unconditional),
            (2, 1, EdgeKind::BranchTrue),
            (2, 3, EdgeKind::BranchFalse),
        ],
    );
    assert!(matches!(
        structure_extended(&cfg),
        Err(StructureError::CyclicControlFlow(_))
    ));
}

#[test]
fn unreachable_blocks_and_payload_without_clone() {
    let cfg = Cfg::new(std::sync::Mutex::new(()));
    assert_eq!(
        structure_extended(&cfg).unwrap().root,
        ExtendedRegion::Block(cfg.entry())
    );
    let (cfg, b) = graph(2, &[]);
    assert_eq!(
        structure_extended(&cfg),
        Err(StructureError::UnreachableBlocks(vec![b[1]]))
    );
}
