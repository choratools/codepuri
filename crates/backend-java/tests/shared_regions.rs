//! Compile and execute the forward-label parent API, without registering it.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "../src/shared_regions.rs"]
mod shared_regions;

use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use decomp_structure::{
    StructureError,
    extended::{FlowStructuredCfg, structure_flow},
};
use shared_regions::{ForwardEmitter, render};
use std::{collections::HashMap, process::Command};

fn overlapping() -> (Cfg<()>, Vec<BlockId>) {
    let mut cfg = Cfg::new(());
    let mut b = vec![cfg.entry()];
    for _ in 1..5 {
        b.push(cfg.add_block(()));
    }
    for (a, z, kind) in [
        (0, 1, EdgeKind::BranchTrue),
        (0, 2, EdgeKind::BranchFalse),
        (1, 3, EdgeKind::BranchTrue),
        (1, 4, EdgeKind::BranchFalse),
        (2, 3, EdgeKind::Unconditional),
    ] {
        cfg.add_edge(b[a], b[z], kind).unwrap();
    }
    (cfg, b)
}

struct JavaEmitter {
    blocks: HashMap<BlockId, usize>,
    edges: HashMap<EdgeId, usize>,
}
impl ForwardEmitter for JavaEmitter {
    fn instructions(&mut self, block: BlockId) -> Result<String, String> {
        Ok(format!("trace += \"b{};\";", self.blocks[&block]))
    }
    fn terminal(&mut self, block: BlockId) -> Result<String, String> {
        Ok(if self.blocks[&block] == 3 {
            "if (fail) throw new IllegalStateException(trace); return trace + \"shared\";".into()
        } else {
            "return trace + \"early\";".into()
        })
    }
    fn condition(&mut self, block: BlockId) -> Result<String, String> {
        Ok(if self.blocks[&block] == 0 { "a" } else { "b" }.into())
    }
    fn selector(&mut self, _: BlockId) -> Result<String, String> {
        Err("no switch".into())
    }
    fn copies(&mut self, edge: EdgeId) -> Result<String, String> {
        Ok(format!("trace += \"e{};\";", self.edges[&edge]))
    }
}

#[test]
fn nested_shared_return_effects_and_exceptions_execute_on_jdk() {
    let (cfg, b) = overlapping();
    assert!(matches!(
        decomp_structure::structure_extended(&cfg),
        Err(StructureError::OverlappingRegion(_))
    ));
    let FlowStructuredCfg::Shared(regions) = structure_flow(&cfg).unwrap() else {
        panic!("expected shared regions")
    };
    assert_eq!(regions.blocks.len(), cfg.block_count());
    assert_eq!(regions.edges.len(), cfg.edge_count());
    assert_eq!(regions.blocks.iter().filter(|r| r.block == b[3]).count(), 1);
    let mut emitter = JavaEmitter {
        blocks: b.into_iter().enumerate().map(|(i, b)| (b, i)).collect(),
        edges: cfg.edge_ids().enumerate().map(|(i, e)| (e, i)).collect(),
    };
    let body = render(&cfg, &regions, "shared", &mut emitter).unwrap();
    let source = format!(
        r#"public class SharedRegionsSmoke {{
static String generated(boolean a, boolean b, boolean fail) {{ String trace = ""; {body} }}
static String reference(boolean a, boolean b, boolean fail) {{
String trace = "b0;";
if (a) {{ trace += "e0;b1;";
  if (b) {{ trace += "e2;"; }} else {{ return trace + "e3;b4;early"; }}
}} else {{ trace += "e1;b2;e4;"; }}
trace += "b3;";
if (fail) throw new IllegalStateException(trace);
return trace + "shared";
}}
static String outcome(boolean generated, boolean a, boolean b, boolean fail) {{
try {{ return generated ? generated(a,b,fail) : reference(a,b,fail); }}
catch (IllegalStateException e) {{ return e.getClass().getName() + ":" + e.getMessage(); }}
}}
public static void main(String[] args) {{
for (int i=0;i<8;i++) {{
boolean a=(i&1)!=0, b=(i&2)!=0, fail=(i&4)!=0;
String expected=outcome(false,a,b,fail), actual=outcome(true,a,b,fail);
if (!expected.equals(actual)) throw new AssertionError(expected + " != " + actual);
}}
System.out.println("8 exact return/effect/exception traces passed");
}}
}}"#
    );
    let dir =
        std::env::temp_dir().join(format!("decomphify-shared-regions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("SharedRegionsSmoke.java");
    std::fs::write(&file, source).unwrap();
    let jdk =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let javac = Command::new(format!("{jdk}/bin/javac"))
        .arg(&file)
        .output()
        .expect("JDK is required; this test is not ignored");
    assert!(
        javac.status.success(),
        "{}",
        String::from_utf8_lossy(&javac.stderr)
    );
    let java = Command::new(format!("{jdk}/bin/java"))
        .args(["-cp", dir.to_str().unwrap(), "SharedRegionsSmoke"])
        .output()
        .unwrap();
    assert!(
        java.status.success(),
        "{}",
        String::from_utf8_lossy(&java.stderr)
    );
    assert!(String::from_utf8_lossy(&java.stdout).contains("8 exact"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn irreducible_and_exception_graphs_remain_explicitly_unsupported() {
    let (mut cfg, b) = overlapping();
    cfg.add_edge(b[3], b[1], EdgeKind::Unconditional).unwrap();
    assert!(matches!(
        structure_flow(&cfg),
        Err(StructureError::CyclicControlFlow(_))
    ));
    let (mut cfg, b) = overlapping();
    cfg.add_edge(b[2], b[4], EdgeKind::Exception("E".into()))
        .unwrap();
    assert!(matches!(
        structure_flow(&cfg),
        Err(StructureError::UnsupportedEdge { .. })
    ));
}

#[test]
fn old_accepted_regions_remain_identical() {
    for n in [1, 2, 128, 8192] {
        let mut cfg = Cfg::new(());
        let mut last = cfg.entry();
        for _ in 1..n {
            let next = cfg.add_block(());
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        let strict = decomp_structure::structure_extended(&cfg).unwrap();
        assert_eq!(
            structure_flow(&cfg).unwrap(),
            FlowStructuredCfg::Strict(strict)
        );
    }
}

#[test]
fn altered_snapshot_is_rejected_before_any_instruction_callback() {
    struct Never;
    impl ForwardEmitter for Never {
        fn instructions(&mut self, _: BlockId) -> Result<String, String> {
            panic!("invalid snapshot reached renderer")
        }
        fn terminal(&mut self, _: BlockId) -> Result<String, String> {
            panic!("terminal")
        }
        fn condition(&mut self, _: BlockId) -> Result<String, String> {
            panic!("condition")
        }
        fn selector(&mut self, _: BlockId) -> Result<String, String> {
            panic!("selector")
        }
        fn copies(&mut self, _: EdgeId) -> Result<String, String> {
            panic!("copies")
        }
    }
    let (cfg, _) = overlapping();
    let FlowStructuredCfg::Shared(mut regions) = structure_flow(&cfg).unwrap() else {
        panic!("shared")
    };
    regions.blocks.pop();
    assert_eq!(
        render(&cfg, &regions, "shared", &mut Never).unwrap_err(),
        "noncanonical forward region snapshot"
    );
}

#[test]
fn high_scale_shared_tail_has_linear_placement_and_edge_coverage() {
    let mut cfg = Cfg::new(());
    let tail = cfg.add_block(());
    let mut header = cfg.entry();
    for _ in 0..20_000 {
        let next = cfg.add_block(());
        cfg.add_edge(header, tail, EdgeKind::BranchTrue).unwrap();
        cfg.add_edge(header, next, EdgeKind::BranchFalse).unwrap();
        header = next;
    }
    // Distinct terminal keeps the shared tail outside a common real postdominator.
    let FlowStructuredCfg::Shared(regions) = structure_flow(&cfg).unwrap() else {
        panic!("expected overlap")
    };
    assert_eq!(regions.blocks.len(), cfg.block_count());
    assert_eq!(regions.edges.len(), cfg.edge_count());
    let distinct: std::collections::HashSet<_> = regions.blocks.iter().map(|b| b.block).collect();
    assert_eq!(distinct.len(), cfg.block_count());
}
