//! Explicit-scope coverage and rejection evidence, without crate-root edits.
#![allow(clippy::unwrap_used)]
#[path = "../src/exceptions.rs"]
mod exceptions;
use decomp_core::cfg::{Cfg, EdgeKind};
use exceptions::{Catch, Scope, structure};

fn fixture() -> (Cfg<()>, Scope) {
    let mut cfg = Cfg::new(());
    let protected = cfg.add_block(());
    let commit = cfg.add_block(());
    let handler = cfg.add_block(());
    let continuation = cfg.add_block(());
    cfg.add_edge(cfg.entry(), protected, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(protected, commit, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(commit, continuation, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(handler, continuation, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(
        protected,
        handler,
        EdgeKind::Exception("pool-type-42".into()),
    )
    .unwrap();
    (
        cfg,
        Scope {
            protected: vec![protected, commit],
            catches: vec![Catch {
                key: "pool-type-42".into(),
                entry: handler,
            }],
            continuation,
            has_unproven_finally: false,
        },
    )
}
#[test]
fn retains_source_identities_and_nonthrowing_commit_in_exact_scope() {
    let (cfg, scope) = fixture();
    let result = structure(&cfg, std::slice::from_ref(&scope)).unwrap();
    assert_eq!(result.scope, scope);
    assert_eq!(result.revision, cfg.revision());
    assert_eq!(result.prefix, vec![cfg.entry()]);
    assert_eq!(result.handlers, vec![vec![scope.catches[0].entry]]);
    assert_eq!(result.suffix, vec![scope.continuation]);
    assert_eq!(result.exceptional.len(), 1);
    assert_eq!(
        cfg.edge(result.exceptional[0]).unwrap().source(),
        scope.protected[0]
    );
}
#[test]
fn rejects_duplicate_dispatch_and_exception_outside_scope() {
    let (mut cfg, scope) = fixture();
    cfg.add_edge(
        scope.protected[0],
        scope.catches[0].entry,
        EdgeKind::Exception("pool-type-42".into()),
    )
    .unwrap();
    assert!(structure(&cfg, &[scope]).is_err());
    let (mut cfg, scope) = fixture();
    cfg.add_edge(
        cfg.entry(),
        scope.catches[0].entry,
        EdgeKind::Exception("pool-type-42".into()),
    )
    .unwrap();
    assert!(structure(&cfg, &[scope]).is_err());
}
#[test]
fn rejects_incomplete_dispatch_and_shared_handlers() {
    let (cfg, mut scope) = fixture();
    scope.catches.push(Catch {
        key: "another-type".into(),
        entry: scope.catches[0].entry,
    });
    assert!(structure(&cfg, &[scope]).is_err());
    let (mut cfg, mut scope) = fixture();
    let h2 = cfg.add_block(());
    cfg.add_edge(h2, scope.continuation, EdgeKind::Unconditional)
        .unwrap();
    scope.catches.push(Catch {
        key: "another-type".into(),
        entry: h2,
    });
    assert!(structure(&cfg, &[scope]).is_err());
}
#[test]
fn rejects_handler_cycle_and_unreachable_blocks() {
    let (mut cfg, scope) = fixture();
    cfg.add_edge(
        scope.catches[0].entry,
        scope.catches[0].entry,
        EdgeKind::Unconditional,
    )
    .unwrap();
    assert!(structure(&cfg, &[scope]).is_err());
    let (mut cfg, scope) = fixture();
    cfg.add_block(());
    assert!(structure(&cfg, &[scope]).is_err());
}

#[test]
fn shared_typed_handler_keeps_original_scope_edges_and_rejects_normal_entry() {
    use exceptions::{RegionScope, structure_disjoint, structure_disjoint_shared_handlers};
    let mut cfg = Cfg::new(());
    let first = cfg.entry();
    let gap = cfg.add_block(());
    let second = cfg.add_block(());
    let normal_return = cfg.add_block(());
    let handler = cfg.add_block(());
    let handler_return = cfg.add_block(());
    cfg.add_edge(first, gap, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(gap, second, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(second, normal_return, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(handler, handler_return, EdgeKind::Unconditional)
        .unwrap();
    let original_first = cfg
        .add_edge(first, handler, EdgeKind::Exception("typed".into()))
        .unwrap();
    let original_second = cfg
        .add_edge(second, handler, EdgeKind::Exception("typed".into()))
        .unwrap();
    let scope = |entry, next| RegionScope {
        protected: vec![entry],
        catches: vec![Catch {
            key: "typed".into(),
            entry: handler,
        }],
        continuation: Some(next),
    };
    let scopes = [scope(first, gap), scope(second, normal_return)];
    assert!(structure_disjoint(&cfg, &scopes).is_err());
    let proved = structure_disjoint_shared_handlers(&cfg, &scopes).unwrap();
    assert_eq!(proved.scopes[0].exceptional, vec![original_first]);
    assert_eq!(proved.scopes[1].exceptional, vec![original_second]);
    assert_eq!(proved.scopes[0].handlers, proved.scopes[1].handlers);
    cfg.add_edge(normal_return, handler, EdgeKind::Unconditional)
        .unwrap();
    assert!(structure_disjoint_shared_handlers(&cfg, &scopes).is_err());
}
