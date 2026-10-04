//! A child-loop exit retains its edge and resumes its active parent's test.
#![allow(clippy::unwrap_used)]
use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::{BlockId, EdgeId};
use decomp_structure::loop_control::{ControlRegion, TransferKind, structure};
use std::collections::HashMap;
fn fixture(foreign: bool) -> (Cfg<()>, BlockId, BlockId, EdgeId) {
    let mut c = Cfg::new(());
    let outer = c.add_block(());
    let select = c.add_block(());
    let init = c.add_block(());
    let inner = c.add_block(());
    let body = c.add_block(());
    let simple = c.add_block(());
    let done = c.add_block(());
    c.add_edge(
        c.entry(),
        outer,
        if foreign {
            EdgeKind::BranchTrue
        } else {
            EdgeKind::Unconditional
        },
    )
    .unwrap();
    c.add_edge(outer, select, EdgeKind::BranchTrue).unwrap();
    c.add_edge(outer, done, EdgeKind::BranchFalse).unwrap();
    c.add_edge(select, init, EdgeKind::BranchTrue).unwrap();
    c.add_edge(select, simple, EdgeKind::BranchFalse).unwrap();
    c.add_edge(init, inner, EdgeKind::Unconditional).unwrap();
    c.add_edge(inner, body, EdgeKind::BranchTrue).unwrap();
    let exit = c.add_edge(inner, outer, EdgeKind::BranchFalse).unwrap();
    c.add_edge(body, inner, EdgeKind::Unconditional).unwrap();
    c.add_edge(simple, outer, EdgeKind::Unconditional).unwrap();
    if foreign {
        c.add_edge(c.entry(), body, EdgeKind::BranchFalse).unwrap();
    }
    (c, outer, inner, exit)
}
fn audit(
    r: &ControlRegion,
    blocks: &mut HashMap<BlockId, usize>,
    transfers: &mut Vec<(EdgeId, TransferKind)>,
) {
    match r {
        ControlRegion::Block(b) => *blocks.entry(*b).or_default() += 1,
        ControlRegion::If {
            header,
            then_region,
            else_region,
            ..
        } => {
            *blocks.entry(*header).or_default() += 1;
            audit(then_region, blocks, transfers);
            audit(else_region, blocks, transfers);
        }
        ControlRegion::Sequence(rs) => {
            for r in rs {
                audit(r, blocks, transfers)
            }
        }
        ControlRegion::Loop { body, .. } => audit(body, blocks, transfers),
        ControlRegion::Transfer { edge, kind } => transfers.push((*edge, *kind)),
    }
}
#[test]
fn child_exit_to_parent_test_has_one_placement_and_one_transfer() {
    let (c, outer, inner, exit) = fixture(false);
    let p = structure(&c).unwrap();
    assert_eq!(p.edges.len(), c.edge_count());
    let mut blocks = HashMap::new();
    let mut transfers = vec![];
    audit(&p.root, &mut blocks, &mut transfers);
    assert_eq!(blocks.len(), c.block_count());
    assert!(blocks.values().all(|n| *n == 1));
    assert_eq!(blocks[&outer], 1);
    assert_eq!(transfers.iter().filter(|(e, _)| *e == exit).count(), 1);
    assert!(transfers.contains(&(exit, TransferKind::Break { header: inner })));
}
#[test]
fn foreign_child_entry_remains_irreducible() {
    let (c, _, _, _) = fixture(true);
    assert!(structure(&c).is_err());
}
