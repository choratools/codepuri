//! CFG mutation and structural invariant tests.

use std::fmt::Debug;

use decomp_core::cfg::{Cfg, EdgeKind};

fn must<T, E: Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("unexpected error: {error:?}"),
    }
}

#[test]
fn mutations_preserve_adjacency_and_advance_revision() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let tail = cfg.add_block("tail");
    let edge = must(cfg.add_edge(entry, tail, EdgeKind::BranchTrue));
    let before_split = cfg.revision();

    let prefix = must(cfg.split_block(tail, "prefix"));

    assert_eq!(cfg.revision(), before_split + 1);
    assert_eq!(must(cfg.edge(edge)).target(), prefix);
    assert_eq!(must(cfg.block(prefix)).incoming(), &[edge]);
    assert!(cfg.validate().is_ok());
}

#[test]
fn split_entry_promotes_new_prefix_to_the_only_entry() {
    let mut cfg = Cfg::new("suffix");
    let suffix = cfg.entry();

    let prefix = must(cfg.split_block(suffix, "prefix"));

    assert_eq!(cfg.entry(), prefix);
    assert_eq!(must(cfg.block(prefix)).incoming(), &[]);
    assert_eq!(must(cfg.block(suffix)).incoming().len(), 1);
    assert!(cfg.validate().is_ok());
}

#[test]
fn redirect_replace_and_insert_between_keep_edges_first_class() {
    let mut cfg = Cfg::new(0);
    let entry = cfg.entry();
    let left = cfg.add_block(1);
    let right = cfg.add_block(2);
    let exit = cfg.add_block(3);
    let branch = must(cfg.add_edge(entry, left, EdgeKind::BranchTrue));
    must(cfg.add_edge(entry, right, EdgeKind::BranchFalse));
    must(cfg.redirect_edge(branch, right));

    let replaced = must(cfg.replace_all_targets(right, &EdgeKind::BranchTrue, exit));
    let middle = must(cfg.insert_between(branch, 4));

    assert_eq!(replaced, 1);
    assert_eq!(must(cfg.edge(branch)).target(), middle);
    assert_eq!(must(cfg.edge(branch)).kind(), &EdgeKind::BranchTrue);
    let outgoing = must(cfg.block(middle)).outgoing();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(must(cfg.edge(outgoing[0])).kind(), &EdgeKind::Unconditional);
    assert_eq!(must(cfg.edge(outgoing[0])).target(), exit);
    assert!(cfg.validate().is_ok());
}

#[test]
fn removing_a_block_cleans_incident_edges_but_rejects_entry() {
    let mut cfg = Cfg::new(());
    let entry = cfg.entry();
    let middle = cfg.add_block(());
    let exit = cfg.add_block(());
    must(cfg.add_edge(entry, middle, EdgeKind::Unconditional));
    must(cfg.add_edge(middle, exit, EdgeKind::Unconditional));
    must(cfg.add_edge(middle, middle, EdgeKind::Exception("retry".to_owned())));

    must(cfg.remove_block(middle));

    assert!(cfg.remove_block(entry).is_err());
    assert_eq!(cfg.edge_count(), 0);
    assert!(cfg.validate().is_ok());
}
