//! Standalone lossless control-region and irreducibility checks.
#![allow(clippy::unwrap_used)]
pub use decomp_structure::{EdgeProvenance, StructureError};
#[path = "../src/loop_control.rs"]
mod control;
use control::{ControlRegion, TransferKind};
use decomp_core::cfg::{Cfg, EdgeKind};
#[test]
fn pretest_early_break_and_conditional_continue_keep_all_edges() {
    let mut c = Cfg::new(());
    let h = c.add_block(());
    let body = c.add_block(());
    let update = c.add_block(());
    let exit = c.add_block(());
    c.add_edge(c.entry(), h, EdgeKind::Unconditional).unwrap();
    c.add_edge(h, body, EdgeKind::BranchTrue).unwrap();
    c.add_edge(h, exit, EdgeKind::BranchFalse).unwrap();
    c.add_edge(body, exit, EdgeKind::BranchTrue).unwrap();
    c.add_edge(body, update, EdgeKind::BranchFalse).unwrap();
    c.add_edge(update, h, EdgeKind::Unconditional).unwrap();
    let r = control::structure(&c).unwrap();
    assert_eq!(r.edges.len(), c.edge_ids().count());
    let text = format!("{:?}", r.root);
    assert!(text.contains("Break"));
    assert!(text.contains("Continue"));
}
#[test]
fn posttest_self_loop_tests_after_entry() {
    let mut c = Cfg::new(());
    let exit = c.add_block(());
    let h = c.entry();
    let back = c.add_edge(h, h, EdgeKind::BranchTrue).unwrap();
    c.add_edge(h, exit, EdgeKind::BranchFalse).unwrap();
    let r = control::structure(&c).unwrap();
    fn find(r: &ControlRegion, e: decomp_core::ids::EdgeId) -> bool {
        match r {
            ControlRegion::Transfer {
                edge,
                kind: TransferKind::Continue { .. },
            } => *edge == e,
            ControlRegion::Sequence(rs) => rs.iter().any(|r| find(r, e)),
            ControlRegion::Loop { body, .. } => find(body, e),
            ControlRegion::If {
                then_region,
                else_region,
                ..
            } => find(then_region, e) || find(else_region, e),
            _ => false,
        }
    }
    assert!(find(&r.root, back));
}
#[test]
fn irreducible_two_entry_cycle_is_rejected() {
    let mut c = Cfg::new(());
    let a = c.add_block(());
    let b = c.add_block(());
    c.add_edge(c.entry(), a, EdgeKind::BranchTrue).unwrap();
    c.add_edge(c.entry(), b, EdgeKind::BranchFalse).unwrap();
    c.add_edge(a, b, EdgeKind::Unconditional).unwrap();
    c.add_edge(b, a, EdgeKind::Unconditional).unwrap();
    assert!(matches!(
        control::structure(&c),
        Err(StructureError::CyclicControlFlow(_))
    ));
}
