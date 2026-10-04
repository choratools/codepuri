//! Canonical normal dispatcher graph validation.
#![allow(clippy::unwrap_used)]
use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_structure::{StructureError, dispatch::plan};
#[test]
fn irreducible_edges_preserved_and_snapshot_changes() {
    let mut cfg = Cfg::new(());
    let e = cfg.entry();
    let a = cfg.add_block(());
    let b = cfg.add_block(());
    let z = cfg.add_block(());
    for (s, t, k) in [
        (e, a, EdgeKind::BranchTrue),
        (e, b, EdgeKind::BranchFalse),
        (a, b, EdgeKind::Unconditional),
        (b, a, EdgeKind::BranchTrue),
        (b, z, EdgeKind::BranchFalse),
    ] {
        cfg.add_edge(s, t, k).unwrap();
    }
    let p = plan(&cfg).unwrap();
    assert_eq!(p.states.len(), 4);
    assert_eq!(p.edges.len(), 5);
    for id in cfg.edge_ids() {
        assert_eq!(p.edges.iter().filter(|x| x.id == id).count(), 1);
    }
    cfg.redirect_edge(p.edges[2].id, z).unwrap();
    assert_ne!(plan(&cfg).unwrap(), p);
}
#[test]
fn malformed_semantics_are_not_dispatchable() {
    let mut cfg = Cfg::new(());
    let z = cfg.add_block(());
    let e = cfg.entry();
    cfg.add_edge(e, z, EdgeKind::BranchTrue).unwrap();
    assert!(matches!(
        plan(&cfg),
        Err(StructureError::UnsupportedBranch { .. })
    ));
    let mut cfg = Cfg::new(());
    let z = cfg.add_block(());
    let e = cfg.entry();
    cfg.add_edge(e, z, EdgeKind::Exception("X".into())).unwrap();
    assert!(matches!(
        plan(&cfg),
        Err(StructureError::UnsupportedEdge { .. })
    ));
    let mut cfg = Cfg::new(());
    cfg.add_block(());
    assert!(matches!(
        plan(&cfg),
        Err(StructureError::UnreachableBlocks(_))
    ));
}
#[test]
fn parallel_switch_edges_retain_distinct_identity() {
    let mut cfg = Cfg::new(());
    let z = cfg.add_block(());
    let e = cfg.entry();
    for k in [
        EdgeKind::Switch(1),
        EdgeKind::Switch(2),
        EdgeKind::Unconditional,
    ] {
        cfg.add_edge(e, z, k).unwrap();
    }
    let p = plan(&cfg).unwrap();
    assert_eq!(p.states[0].outgoing.len(), 3);
    assert_eq!(p.edges.len(), 3);
}
