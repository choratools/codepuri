//! Canonical control-flow shape tests for graph analyses.

use std::collections::HashSet;

use decomp_core::analysis::{
    dominance_frontier, dominators, is_reducible, natural_loops, post_dominators, postorder,
    reverse_postorder, strongly_connected_components,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::BlockId;

fn add_edge(cfg: &mut Cfg<&'static str>, source: BlockId, target: BlockId) {
    if let Err(error) = cfg.add_edge(source, target, EdgeKind::Unconditional) {
        panic!("fixture edge failed: {error:?}");
    }
}

#[test]
fn deep_chain_interval_queries_keep_reachability_and_do_not_recurse() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let mut previous = entry;
    let mut blocks = vec![entry];
    for _ in 0..8192 {
        let next = cfg.add_block("chain");
        add_edge(&mut cfg, previous, next);
        previous = next;
        blocks.push(next);
    }
    let dead = cfg.add_block("unreachable");
    let dom = dominators(&cfg);
    let post = post_dominators(&cfg);
    for (index, &block) in blocks.iter().enumerate() {
        assert!(dom.dominates(entry, block));
        assert!(post.post_dominates(previous, block));
        assert!(!dom.dominates(dead, block));
        assert!(!post.post_dominates(dead, block));
        if index > 0 {
            assert_eq!(dom.immediate_dominator(block), Some(blocks[index - 1]));
            assert!(!dom.dominates(block, blocks[index - 1]));
            assert!(!post.post_dominates(blocks[index - 1], block));
        }
    }
    assert!(natural_loops(&cfg, &dom).is_empty());
    assert!(!dom.dominates(dead, dead));
    assert!(!post.post_dominates(dead, dead));
}

#[test]
#[ignore = "controlled dominance-query scaling microbenchmark; not whole-APK throughput"]
fn repeated_dominance_query_scaling() {
    use std::time::Instant;
    for count in [1024usize, 4096, 16384] {
        let mut cfg = Cfg::new("entry");
        let mut blocks = vec![cfg.entry()];
        for _ in 1..count {
            let block = cfg.add_block("chain");
            add_edge(&mut cfg, blocks[blocks.len() - 1], block);
            blocks.push(block);
        }
        let dom = dominators(&cfg);
        let mut steps = 0u64;
        let start = Instant::now();
        for pair in blocks.windows(2) {
            let mut cursor = Some(pair[0]);
            while let Some(block) = cursor {
                assert_ne!(block, pair[1]);
                steps += 1;
                cursor = dom.immediate_dominator(block);
            }
        }
        let parent_walk = start.elapsed();
        let start = Instant::now();
        for pair in blocks.windows(2) {
            assert!(!std::hint::black_box(&dom).dominates(pair[1], pair[0]));
        }
        let indexed = start.elapsed();
        assert_eq!(steps, count as u64 * (count as u64 - 1) / 2);
        eprintln!(
            "chain-query {count}: parent_steps={steps}, indexed_queries={}, parent_walk={parent_walk:?}, indexed={indexed:?}",
            count - 1
        );
    }
}

#[test]
fn traversal_orders_a_diamond_in_depth_first_postorder() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let left = cfg.add_block("left");
    let right = cfg.add_block("right");
    let exit = cfg.add_block("exit");
    add_edge(&mut cfg, entry, left);
    add_edge(&mut cfg, entry, right);
    add_edge(&mut cfg, left, exit);
    add_edge(&mut cfg, right, exit);

    let po = postorder(&cfg);
    let rpo = reverse_postorder(&cfg);

    assert_eq!(po, vec![exit, left, right, entry]);
    assert_eq!(rpo, vec![entry, right, left, exit]);
}

#[test]
fn dominators_and_frontier_match_a_diamond() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let left = cfg.add_block("left");
    let right = cfg.add_block("right");
    let join = cfg.add_block("join");
    add_edge(&mut cfg, entry, left);
    add_edge(&mut cfg, entry, right);
    add_edge(&mut cfg, left, join);
    add_edge(&mut cfg, right, join);

    let dom = dominators(&cfg);
    let frontier = dominance_frontier(&cfg, &dom);

    assert_eq!(dom.immediate_dominator(join), Some(entry));
    assert_eq!(frontier.get(&left), Some(&vec![join]));
    assert_eq!(frontier.get(&right), Some(&vec![join]));
    assert!(dom.dominates(entry, join));
}

#[test]
fn post_dominators_match_if_else_join() {
    let mut cfg = Cfg::new("condition");
    let condition = cfg.entry();
    let yes = cfg.add_block("yes");
    let no = cfg.add_block("no");
    let join = cfg.add_block("join");
    let exit = cfg.add_block("exit");
    add_edge(&mut cfg, condition, yes);
    add_edge(&mut cfg, condition, no);
    add_edge(&mut cfg, yes, join);
    add_edge(&mut cfg, no, join);
    add_edge(&mut cfg, join, exit);

    let post_dom = post_dominators(&cfg);

    assert_eq!(post_dom.immediate_post_dominator(condition), Some(join));
    assert_eq!(post_dom.immediate_post_dominator(join), Some(exit));
    assert!(post_dom.post_dominates(join, yes));
}

#[test]
fn scc_and_natural_loops_find_single_loop_and_self_loop() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let header = cfg.add_block("header");
    let body = cfg.add_block("body");
    let exit = cfg.add_block("exit");
    add_edge(&mut cfg, entry, header);
    add_edge(&mut cfg, header, body);
    add_edge(&mut cfg, body, header);
    add_edge(&mut cfg, header, exit);
    add_edge(&mut cfg, exit, exit);

    let components = strongly_connected_components(&cfg);
    let loops = natural_loops(&cfg, &dominators(&cfg));

    assert!(components.iter().any(|component| {
        component.iter().copied().collect::<HashSet<_>>() == HashSet::from([header, body])
    }));
    assert!(components.iter().any(|component| component == &[exit]));
    assert!(
        loops
            .iter()
            .any(|natural| { natural.header() == header && natural.blocks().contains(&body) })
    );
    assert!(
        loops
            .iter()
            .any(|natural| natural.header() == exit && natural.latch() == exit)
    );
}

#[test]
fn nested_loops_have_distinct_headers_and_are_reducible() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let outer = cfg.add_block("outer");
    let inner = cfg.add_block("inner");
    let inner_body = cfg.add_block("inner body");
    let outer_latch = cfg.add_block("outer latch");
    let exit = cfg.add_block("exit");
    add_edge(&mut cfg, entry, outer);
    add_edge(&mut cfg, outer, inner);
    add_edge(&mut cfg, outer, exit);
    add_edge(&mut cfg, inner, inner_body);
    add_edge(&mut cfg, inner, outer_latch);
    add_edge(&mut cfg, inner_body, inner);
    add_edge(&mut cfg, outer_latch, outer);

    let loops = natural_loops(&cfg, &dominators(&cfg));

    assert!(loops.iter().any(|natural| natural.header() == inner));
    assert!(loops.iter().any(|natural| natural.header() == outer));
    assert!(is_reducible(&cfg));
}

#[test]
fn two_entry_cycle_is_irreducible() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let left = cfg.add_block("left");
    let right = cfg.add_block("right");
    add_edge(&mut cfg, entry, left);
    add_edge(&mut cfg, entry, right);
    add_edge(&mut cfg, left, right);
    add_edge(&mut cfg, right, left);

    assert!(!is_reducible(&cfg));
}

#[test]
fn entry_self_loop_is_reducible() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    add_edge(&mut cfg, entry, entry);

    assert!(is_reducible(&cfg));
}

#[test]
fn dominated_region_with_an_inner_two_entry_cycle_is_irreducible() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let header = cfg.add_block("header");
    let left = cfg.add_block("left");
    let right = cfg.add_block("right");
    add_edge(&mut cfg, entry, header);
    add_edge(&mut cfg, header, left);
    add_edge(&mut cfg, header, right);
    add_edge(&mut cfg, left, right);
    add_edge(&mut cfg, right, left);
    add_edge(&mut cfg, left, header);

    assert!(!is_reducible(&cfg));
}

#[test]
fn exception_parallel_edges_and_nonterminating_regions_preserve_boundaries() {
    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let normal = cfg.add_block("normal exit");
    let handler = cfg.add_block("handler exit");
    let endless = cfg.add_block("endless");
    let dead = cfg.add_block("unreachable");
    add_edge(&mut cfg, entry, normal);
    add_edge(&mut cfg, entry, normal);
    cfg.add_edge(entry, handler, EdgeKind::Exception("E".into()))
        .unwrap_or_else(|error| panic!("fixture edge failed: {error:?}"));
    add_edge(&mut cfg, entry, endless);
    add_edge(&mut cfg, endless, endless);
    add_edge(&mut cfg, dead, handler);
    add_edge(&mut cfg, handler, entry);

    let dom = dominators(&cfg);
    let frontier = dominance_frontier(&cfg, &dom);
    let post = post_dominators(&cfg);
    assert_eq!(dom.immediate_dominator(handler), Some(entry));
    assert!(!dom.dominates(dead, dead));
    assert!(!frontier.contains_key(&dead));
    assert_eq!(frontier[&entry], vec![entry]);
    assert_eq!(frontier[&handler], vec![entry]);
    assert_eq!(frontier[&endless], vec![endless]);
    assert_eq!(post.immediate_post_dominator(entry), Some(normal));
    assert!(!post.post_dominates(endless, endless));
    assert!(!post.post_dominates(dead, dead));
    assert!(post.post_dominates(normal, handler));

    let mut cfg = Cfg::new("entry");
    let entry = cfg.entry();
    let normal = cfg.add_block("normal exit");
    let handler = cfg.add_block("handler exit");
    add_edge(&mut cfg, entry, normal);
    cfg.add_edge(entry, handler, EdgeKind::Exception("E".into()))
        .unwrap_or_else(|error| panic!("fixture edge failed: {error:?}"));
    let post = post_dominators(&cfg);
    assert_eq!(post.immediate_post_dominator(entry), None);
    assert!(post.post_dominates(handler, handler));
    assert!(!post.post_dominates(normal, entry));
}

/// Run with `cargo test -p decomp-core --release --test analysis_cases
/// large_chain_frontier_scaling -- --ignored --nocapture`.
#[test]
#[ignore = "large graph scaling evidence"]
fn large_chain_frontier_scaling() {
    use std::time::Instant;
    for count in [4_000, 16_000, 64_000] {
        let mut cfg = Cfg::new("entry");
        let mut previous = cfg.entry();
        for _ in 1..count {
            let next = cfg.add_block("chain");
            add_edge(&mut cfg, previous, next);
            previous = next;
        }
        let started = Instant::now();
        let dom = dominators(&cfg);
        let dominance_time = started.elapsed();
        let started = Instant::now();
        let frontier = dominance_frontier(&cfg, &dom);
        let frontier_time = started.elapsed();
        let started = Instant::now();
        let post = post_dominators(&cfg);
        let post_time = started.elapsed();
        assert_eq!(frontier.len(), count);
        assert!(frontier.values().all(Vec::is_empty));
        assert!(dom.dominates(cfg.entry(), previous));
        assert!(post.post_dominates(previous, cfg.entry()));
        eprintln!(
            "chain {count}: dom={dominance_time:?}, frontier={frontier_time:?}, post={post_time:?}; old frontier ordering={} membership probes, new ordering={} keys + 0 members",
            count as u64 * count as u64,
            count
        );
    }
}
