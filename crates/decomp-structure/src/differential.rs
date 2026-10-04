#![allow(clippy::unwrap_used)]
use super::*;
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize % n
    }
}

fn check(cfg: &Cfg<()>) -> (bool, bool) {
    let original = structure_using(cfg, postdom::original_joins, original_build);
    let optimized = structure(cfg);
    assert_eq!(optimized, original, "basic graph: {cfg:?}");
    let original = extended::original_structure(cfg);
    let optimized_extended = structure_extended(cfg);
    assert_eq!(optimized_extended, original, "extended graph: {cfg:?}");
    if let Ok(result) = &optimized {
        let mut stack = vec![&result.root];
        let mut blocks = HashSet::new();
        while let Some(region) = stack.pop() {
            match region {
                Region::While {
                    evaluation,
                    condition,
                    body,
                    ..
                } => {
                    assert!(blocks.insert(*condition));
                    stack.extend([evaluation.as_ref(), body.as_ref()]);
                }
                Region::Block(b) => assert!(blocks.insert(*b)),
                Region::Sequence(items) => stack.extend(items),
                Region::If {
                    header,
                    then_region,
                    else_region,
                    ..
                } => {
                    assert!(blocks.insert(*header));
                    stack.extend([then_region.as_ref(), else_region.as_ref()]);
                }
            }
        }
        assert_eq!(blocks, cfg.block_ids().collect());
    }
    if let Ok(result) = &optimized_extended {
        let mut stack = vec![&result.root];
        let mut blocks = HashSet::new();
        while let Some(region) = stack.pop() {
            match region {
                ExtendedRegion::Block(b) => assert!(blocks.insert(*b)),
                ExtendedRegion::Sequence(items) => stack.extend(items),
                ExtendedRegion::If {
                    header,
                    then_region,
                    else_region,
                    ..
                } => {
                    assert!(blocks.insert(*header));
                    stack.extend([then_region.as_ref(), else_region.as_ref()]);
                }
                ExtendedRegion::Switch { header, arms, .. } => {
                    assert!(blocks.insert(*header));
                    stack.extend(arms.iter().map(|a| &a.region));
                }
            }
        }
        assert_eq!(blocks, cfg.block_ids().collect());
    }
    (optimized.is_ok(), optimized_extended.is_ok())
}

#[test]
fn randomized_dags_match_original_trees_and_exact_errors() {
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let mut basic_ok = 0;
    let mut extended_ok = 0;
    for trial in 0..6000 {
        let n = 2 + rng.next(30);
        let mut cfg = Cfg::new(());
        let mut blocks = vec![cfg.entry()];
        blocks.extend((1..n).map(|_| cfg.add_block(())));
        for i in 0..n - 1 {
            let target = i + 1 + rng.next(n - i - 1);
            match rng.next(6) {
                0 => {}
                1 | 2 => {
                    cfg.add_edge(blocks[i], blocks[i + 1], EdgeKind::Unconditional)
                        .unwrap();
                }
                3 | 4 => {
                    // Reverse insertion order and coincident targets are intentional.
                    cfg.add_edge(blocks[i], blocks[target], EdgeKind::BranchFalse)
                        .unwrap();
                    cfg.add_edge(blocks[i], blocks[i + 1], EdgeKind::BranchTrue)
                        .unwrap();
                }
                _ => {
                    for label in 0..1 + rng.next(6) {
                        let t = if label == 0 {
                            i + 1
                        } else {
                            i + 1 + rng.next(n - i - 1)
                        };
                        cfg.add_edge(blocks[i], blocks[t], EdgeKind::Switch(label as u64))
                            .unwrap();
                    }
                    if trial % 2 == 0 {
                        cfg.add_edge(blocks[i], blocks[target], EdgeKind::Unconditional)
                            .unwrap();
                    }
                }
            }
        }
        // Join selection is tested even when validation/grammar would reject
        // the graph, so unreachable/overlapping cases cannot hide LCA errors.
        let successors = blocks
            .iter()
            .map(|&b| {
                let targets = cfg
                    .block(b)
                    .unwrap()
                    .outgoing()
                    .iter()
                    .map(|&id| cfg.edge(id).unwrap().target())
                    .collect();
                (b, targets)
            })
            .collect();
        assert_eq!(
            postdom::joins(&blocks, &successors),
            postdom::original_joins(&blocks, &successors)
        );
        let (a, b) = check(&cfg);
        basic_ok += usize::from(a);
        extended_ok += usize::from(b);
    }
    assert!(basic_ok > 100, "{basic_ok}");
    assert!(extended_ok > 100, "{extended_ok}");
    eprintln!(
        "6000 random DAGs: basic accepted={basic_ok}, extended accepted={extended_ok}; exact trees/errors matched"
    );
}

fn fixture(n: usize, diamonds: bool, nested: bool) -> Cfg<()> {
    let mut cfg = Cfg::new(());
    let mut current = cfg.entry();
    let terminal = nested.then(|| cfg.add_block(()));
    for _ in 0..n {
        let next = cfg.add_block(());
        if diamonds {
            let yes = cfg.add_block(());
            let no = cfg.add_block(());
            cfg.add_edge(current, yes, EdgeKind::BranchTrue).unwrap();
            cfg.add_edge(current, no, EdgeKind::BranchFalse).unwrap();
            cfg.add_edge(yes, next, EdgeKind::Unconditional).unwrap();
            cfg.add_edge(no, next, EdgeKind::Unconditional).unwrap();
        } else if let Some(terminal) = terminal {
            cfg.add_edge(current, next, EdgeKind::BranchTrue).unwrap();
            cfg.add_edge(current, terminal, EdgeKind::BranchFalse)
                .unwrap();
        } else {
            cfg.add_edge(current, next, EdgeKind::Unconditional)
                .unwrap();
        }
        current = next;
    }
    if let Some(terminal) = terminal {
        cfg.add_edge(current, terminal, EdgeKind::Unconditional)
            .unwrap();
    }
    cfg
}

#[test]
fn nested_and_diamond_oracle_coverage() {
    for n in [1, 2, 8, 32, 128] {
        assert_eq!(check(&fixture(n, true, false)), (true, true));
        assert_eq!(check(&fixture(n, false, true)), (true, true));
        let mut cfg = Cfg::new(());
        let terminal = cfg.add_block(());
        let mut current = cfg.entry();
        for i in 0..n {
            let next = cfg.add_block(());
            cfg.add_edge(current, next, EdgeKind::Switch(i as u64))
                .unwrap();
            cfg.add_edge(current, terminal, EdgeKind::Unconditional)
                .unwrap();
            cfg.add_edge(current, next, EdgeKind::Switch(i as u64 + 1000))
                .unwrap();
            current = next;
        }
        cfg.add_edge(current, terminal, EdgeKind::Unconditional)
            .unwrap();
        assert_eq!(check(&cfg), (false, true));
    }
}

#[test]
fn explicit_rejects_match_original() {
    for kind in [
        EdgeKind::Unconditional,
        EdgeKind::BranchTrue,
        EdgeKind::Switch(1),
        EdgeKind::Exception("handler".into()),
    ] {
        let mut cfg = Cfg::new(());
        cfg.add_edge(cfg.entry(), cfg.entry(), kind).unwrap();
        assert_eq!(check(&cfg), (false, false));
    }
    let mut cfg = Cfg::new(());
    let a = cfg.add_block(());
    let b = cfg.add_block(());
    cfg.add_edge(cfg.entry(), a, EdgeKind::Switch(7)).unwrap();
    cfg.add_edge(cfg.entry(), b, EdgeKind::Switch(7)).unwrap();
    assert_eq!(check(&cfg), (false, false));
}

// Public region enums still have recursive derived Drop/Clone/Eq. Drain them
// iteratively here so the deep construction test isolates construction depth.
fn drain(region: Region) {
    let mut stack = vec![region];
    while let Some(region) = stack.pop() {
        match region {
            Region::While {
                evaluation, body, ..
            } => stack.extend([*evaluation, *body]),
            Region::Block(_) => {}
            Region::Sequence(items) => stack.extend(items),
            Region::If {
                then_region,
                else_region,
                ..
            } => stack.extend([*then_region, *else_region]),
        }
    }
}
fn drain_extended(region: ExtendedRegion) {
    let mut stack = vec![region];
    while let Some(region) = stack.pop() {
        match region {
            ExtendedRegion::Block(_) => {}
            ExtendedRegion::Sequence(items) => stack.extend(items),
            ExtendedRegion::If {
                then_region,
                else_region,
                ..
            } => stack.extend([*then_region, *else_region]),
            ExtendedRegion::Switch { arms, .. } => stack.extend(arms.into_iter().map(|a| a.region)),
        }
    }
}

#[test]
fn deeply_nested_construction_uses_explicit_frames() {
    let cfg = fixture(20_000, false, true);
    drain(structure(&cfg).unwrap().root);
    drain_extended(structure_extended(&cfg).unwrap().root);
}

#[test]
#[ignore = "release scaling benchmark; run with --release --ignored --nocapture"]
fn scaling_benchmark() {
    for diamonds in [false, true] {
        for n in [256, 1024, 4096, 16_384, 65_536] {
            let cfg = fixture(n, diamonds, false);
            let start = Instant::now();
            let result = structure(&cfg).unwrap();
            let optimized = start.elapsed();
            drain(result.root);
            let start = Instant::now();
            let result = structure_extended(&cfg).unwrap();
            let extended = start.elapsed();
            drain_extended(result.root);
            let original = if n <= 4096 {
                let start = Instant::now();
                let result =
                    structure_using(&cfg, postdom::original_joins, original_build).unwrap();
                let elapsed = start.elapsed();
                drain(result.root);
                Some(elapsed)
            } else {
                None
            };
            let b = cfg.block_count();
            let levels = (usize::BITS - b.leading_zeros()) as usize;
            eprintln!(
                "diamonds={diamonds} B={b} E={} optimized={optimized:?} extended={extended:?} original={original:?} ancestor_slots={}",
                cfg.edge_count(),
                (b + 1) * levels
            );
        }
    }
}
