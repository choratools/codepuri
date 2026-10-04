//! Repeated multiple-latch plans preserve the original common return and effects.
#![allow(clippy::unwrap_used)]
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::BlockId,
};
use decomp_structure::loop_control::{self, ControlRegion, TransferKind};
use std::{collections::HashMap, process::Command};

fn retry() -> (Cfg<&'static str>, BlockId) {
    let mut c = Cfg::new("start");
    let h = c.add_block("attempt");
    let choose = c.add_block("choose");
    let first = c.add_block("retry_first");
    let first_exit = c.add_block("first_exit");
    let early = c.add_block("early_test");
    let early_return = c.add_block("early_return");
    let second = c.add_block("retry_second");
    let second_exit = c.add_block("second_exit");
    let common = c.add_block("common_return");
    c.add_edge(c.entry(), h, EdgeKind::Unconditional).unwrap();
    c.add_edge(h, choose, EdgeKind::Unconditional).unwrap();
    c.add_edge(choose, first, EdgeKind::BranchFalse).unwrap();
    c.add_edge(choose, early, EdgeKind::BranchTrue).unwrap();
    c.add_edge(first, h, EdgeKind::BranchTrue).unwrap();
    c.add_edge(first, first_exit, EdgeKind::BranchFalse)
        .unwrap();
    c.add_edge(first_exit, common, EdgeKind::Unconditional)
        .unwrap();
    c.add_edge(early, early_return, EdgeKind::BranchTrue)
        .unwrap();
    c.add_edge(early, second, EdgeKind::BranchFalse).unwrap();
    c.add_edge(second, h, EdgeKind::BranchTrue).unwrap();
    c.add_edge(second, second_exit, EdgeKind::BranchFalse)
        .unwrap();
    c.add_edge(second_exit, common, EdgeKind::Unconditional)
        .unwrap();
    (c, common)
}
fn placements(r: &ControlRegion, counts: &mut HashMap<BlockId, usize>) {
    match r {
        ControlRegion::Block(b) => *counts.entry(*b).or_default() += 1,
        ControlRegion::If {
            header,
            then_region,
            else_region,
            ..
        } => {
            *counts.entry(*header).or_default() += 1;
            placements(then_region, counts);
            placements(else_region, counts);
        }
        ControlRegion::Sequence(rs) => {
            for r in rs {
                placements(r, counts);
            }
        }
        ControlRegion::Loop { body, .. } => placements(body, counts),
        ControlRegion::Transfer { .. } => (),
    }
}
#[test]
fn multiple_retry_latches_and_private_return_are_stable() {
    let (c, common) = retry();
    let first = loop_control::structure(&c).unwrap();
    for _ in 0..100 {
        let plan = loop_control::structure(&c).unwrap();
        assert_eq!(plan, first);
        let mut counts = HashMap::new();
        placements(&plan.root, &mut counts);
        assert_eq!(counts.len(), c.block_ids().count());
        assert!(counts.values().all(|n| *n == 1));
        assert_eq!(counts[&common], 1);
        assert_eq!(plan.edges.len(), c.edge_ids().count());
    }
}
#[test]
fn irreducible_cycle_is_always_rejected() {
    let mut c = Cfg::new(());
    let a = c.add_block(());
    let b = c.add_block(());
    c.add_edge(c.entry(), a, EdgeKind::BranchTrue).unwrap();
    c.add_edge(c.entry(), b, EdgeKind::BranchFalse).unwrap();
    c.add_edge(a, b, EdgeKind::Unconditional).unwrap();
    c.add_edge(b, a, EdgeKind::Unconditional).unwrap();
    for _ in 0..100 {
        assert!(loop_control::structure(&c).is_err());
    }
}
fn emit(r: &ControlRegion, cfg: &Cfg<&str>) -> String {
    match r {
        ControlRegion::Block(b) => match cfg.block(*b).unwrap().payload() {
            &"start" => "StringBuilder s=new StringBuilder(); int n=0;".into(),
            &"attempt" => "s.append('A'); n++;".into(),
            &"first_exit" => "s.append('F');".into(),
            &"second_exit" => "s.append('S');".into(),
            &"early_return" => "s.append('E'); return s.toString();".into(),
            &"common_return" => "s.append('R'); return s.toString();".into(),
            name => panic!("unexpected {name}"),
        },
        ControlRegion::Sequence(rs) => rs.iter().map(|r| emit(r, cfg)).collect(),
        ControlRegion::Loop { header, body, .. } => format!(
            "L{}:while(true){{{}}}",
            label(*header, cfg),
            emit(body, cfg)
        ),
        ControlRegion::Transfer { kind, .. } => match kind {
            TransferKind::Break { header } => format!("break L{};", label(*header, cfg)),
            TransferKind::Continue { header } => format!("continue L{};", label(*header, cfg)),
        },
        ControlRegion::If {
            header,
            then_region,
            else_region,
            ..
        } => {
            let cond = match cfg.block(*header).unwrap().payload() {
                &"choose" => "second",
                &"early_test" => "early && n==stop",
                &"retry_first" | &"retry_second" => "n<stop",
                name => panic!("unexpected {name}"),
            };
            format!(
                "if({cond}){{{}}}else{{{}}}",
                emit(then_region, cfg),
                emit(else_region, cfg)
            )
        }
    }
}
fn label(b: BlockId, cfg: &Cfg<&str>) -> usize {
    cfg.block_ids().position(|id| id == b).unwrap()
}
#[test]
fn java_retry_trace_preserves_early_return_and_single_common_tail() {
    let (cfg, _) = retry();
    let plan = loop_control::structure(&cfg).unwrap();
    let body = emit(&plan.root, &cfg);
    let source = format!(
        r#"public class RetryProof {{
      static String recovered(boolean second,boolean early,int stop){{{body}}}
      static String reference(boolean second,boolean early,int stop){{
        StringBuilder s=new StringBuilder();int n=0;
        while(true){{s.append('A');n++;if(second){{if(early&&n==stop){{s.append('E');return s.toString();}}if(n<stop)continue;s.append('S');}}
        else{{if(n<stop)continue;s.append('F');}}s.append('R');return s.toString();}}
      }}
      public static void main(String[] args){{for(int n=1;n<=20;n++)for(boolean a:new boolean[]{{false,true}})for(boolean b:new boolean[]{{false,true}})
        if(!recovered(a,b,n).equals(reference(a,b,n)))throw new AssertionError();}}
    }}"#
    );
    let root = std::env::temp_dir().join(format!("loop-retry-proof-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("RetryProof.java"), source).unwrap();
    let compile = Command::new("javac")
        .arg(root.join("RetryProof.java"))
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new("java")
        .args(["-cp", root.to_str().unwrap(), "RetryProof"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
