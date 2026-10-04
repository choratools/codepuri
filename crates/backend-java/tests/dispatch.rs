//! Verbatim Java17 dispatcher semantic checks; no original body stubs.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use backend_java::dispatch::{DispatchEmitter, render};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use decomp_structure::dispatch::plan;
use std::{collections::HashMap, process::Command};
struct Hooks {
    blocks: HashMap<BlockId, usize>,
    copies: HashMap<EdgeId, String>,
    unknown: bool,
}
impl DispatchEmitter for Hooks {
    fn instructions(&mut self, b: BlockId) -> Result<String, String> {
        if self.unknown {
            return Err("unknown read".into());
        }
        Ok(match self.blocks[&b]{0=>"trace += \"E\";",1=>"trace += \"A\"; steps++;",2=>"trace += \"B\"; steps++; if (fail && steps == 2) throw new IllegalArgumentException(trace);",_=>"trace += \"Z\";"}.into())
    }
    fn terminal(&mut self, _: BlockId) -> Result<String, String> {
        Ok("return trace + \":\" + x + \":\" + y;".into())
    }
    fn condition(&mut self, b: BlockId) -> Result<String, String> {
        Ok(if self.blocks[&b] == 0 {
            "left"
        } else {
            "steps < count"
        }
        .into())
    }
    fn selector(&mut self, _: BlockId) -> Result<String, String> {
        Err("no selector".into())
    }
    fn copies(&mut self, e: EdgeId) -> Result<String, String> {
        Ok(self.copies.get(&e).cloned().unwrap_or_default())
    }
}
fn fixture() -> (Cfg<()>, Hooks) {
    let mut c = Cfg::new(());
    let mut b = vec![c.entry()];
    for _ in 1..4 {
        b.push(c.add_block(()));
    }
    let mut copies = HashMap::new();
    for (s, t, k) in [
        (0, 1, EdgeKind::BranchTrue),
        (0, 2, EdgeKind::BranchFalse),
        (1, 2, EdgeKind::Unconditional),
        (2, 1, EdgeKind::BranchTrue),
        (2, 3, EdgeKind::BranchFalse),
    ] {
        let edge = c.add_edge(b[s], b[t], k).unwrap();
        if s == 2 && t == 1 {
            copies.insert(
                edge,
                "{ int first = y; int second = x; x = first; y = second; }".into(),
            );
        }
    }
    (
        c,
        Hooks {
            blocks: b.into_iter().enumerate().map(|(i, b)| (b, i)).collect(),
            copies,
            unknown: false,
        },
    )
}
#[test]
fn irreducible_phi_swap_effect_and_throw_traces_match_java17() {
    let (cfg, mut hooks) = fixture();
    let body = render(&cfg, &plan(&cfg).unwrap(), "dispatch", &mut hooks).unwrap();
    let source = format!(
        r#"public class DispatchSmoke {{
static String generated(boolean left,int count,boolean fail) {{ String trace=""; int steps=0,x=3,y=7; {body} }}
static String reference(boolean left,int count,boolean fail) {{
String trace="E"; int steps=0,x=3,y=7; boolean atA=left;
while(true) {{
 if(atA) {{trace+="A";steps++;}}
 trace+="B";steps++;if(fail&&steps==2)throw new IllegalArgumentException(trace);
 if(steps>=count)return trace+"Z:"+x+":"+y;
 int first=y,second=x;x=first;y=second;atA=true;
}}
}}
static String observe(boolean generated,boolean left,int n,boolean fail) {{try {{return generated?generated(left,n,fail):reference(left,n,fail);}}catch(IllegalArgumentException e){{return "throw:"+e.getMessage();}}}}
public static void main(String[] args) {{for(boolean left:new boolean[]{{false,true}})for(int n=0;n<12;n++)for(boolean fail:new boolean[]{{false,true}}){{String a=observe(true,left,n,fail),b=observe(false,left,n,fail);if(!a.equals(b))throw new AssertionError(a+" != "+b);}}}}
}}"#
    );
    run_java("DispatchSmoke", &source);
}
fn run_java(name: &str, source: &str) {
    let dir =
        std::env::temp_dir().join(format!("decomphify-dispatch-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.java"));
    std::fs::write(&path, source).unwrap();
    let javac = Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&dir)
        .arg(&path)
        .output()
        .expect("installed JDK required");
    assert!(
        javac.status.success(),
        "{}",
        String::from_utf8_lossy(&javac.stderr)
    );
    let java = Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg(name)
        .output()
        .unwrap();
    assert!(
        java.status.success(),
        "{}",
        String::from_utf8_lossy(&java.stderr)
    );
}
#[test]
fn canonical_snapshot_unknown_read_and_identifier_fail_closed() {
    let (cfg, mut hooks) = fixture();
    let mut snapshot = plan(&cfg).unwrap();
    snapshot.states.reverse();
    assert!(render(&cfg, &snapshot, "dispatch", &mut hooks).is_err());
    let snapshot = plan(&cfg).unwrap();
    assert!(render(&cfg, &snapshot, "class-name", &mut hooks).is_err());
    hooks.unknown = true;
    assert!(render(&cfg, &snapshot, "dispatch", &mut hooks).is_err());
}
struct SwitchHooks {
    terminal: BlockId,
    edge_values: HashMap<EdgeId, i32>,
}
impl DispatchEmitter for SwitchHooks {
    fn instructions(&mut self, _: BlockId) -> Result<String, String> {
        Ok(String::new())
    }
    fn terminal(&mut self, b: BlockId) -> Result<String, String> {
        if b != self.terminal {
            return Err("unknown terminal".into());
        }
        Ok("return value + 100 * evaluations;".into())
    }
    fn condition(&mut self, _: BlockId) -> Result<String, String> {
        Err("no condition".into())
    }
    fn selector(&mut self, _: BlockId) -> Result<String, String> {
        Ok("select(key)".into())
    }
    fn copies(&mut self, e: EdgeId) -> Result<String, String> {
        Ok(format!("value = {};", self.edge_values[&e]))
    }
}
#[test]
fn parallel_switch_arms_select_edge_copies_and_evaluate_once() {
    let mut cfg = Cfg::new(());
    let end = cfg.add_block(());
    let entry = cfg.entry();
    let mut edge_values = HashMap::new();
    for (kind, value) in [
        (EdgeKind::Switch(u64::MAX), 11),
        (EdgeKind::Switch(2), 22),
        (EdgeKind::Unconditional, 33),
    ] {
        let id = cfg.add_edge(entry, end, kind).unwrap();
        edge_values.insert(id, value);
    }
    let mut bad_terminal = SwitchHooks {
        terminal: entry,
        edge_values: edge_values.clone(),
    };
    assert!(render(&cfg, &plan(&cfg).unwrap(), "terminal", &mut bad_terminal).is_err());
    let mut hooks = SwitchHooks {
        terminal: end,
        edge_values,
    };
    let body = render(&cfg, &plan(&cfg).unwrap(), "switching", &mut hooks).unwrap();
    run_java(
        "DispatchSwitch",
        &format!(
            r#"public class DispatchSwitch {{static int evaluations;static int select(int key){{evaluations++;return key;}}static int generated(int key){{int value=0;{body}}}public static void main(String[] args){{for(int key:new int[]{{-1,2,3}}){{evaluations=0;int actual=generated(key);int expected=100+(key==-1?11:key==2?22:33);if(actual!=expected)throw new AssertionError(actual);}}}}}}"#
        ),
    );
    // Distinct raw u64 values cannot produce duplicate Java int labels.
    let mut cfg = Cfg::new(());
    let end = cfg.add_block(());
    let entry = cfg.entry();
    for k in [
        EdgeKind::Switch(u64::MAX),
        EdgeKind::Switch(u64::from(u32::MAX)),
        EdgeKind::Unconditional,
    ] {
        cfg.add_edge(entry, end, k).unwrap();
    }
    let edge_values = cfg.edge_ids().map(|id| (id, 0)).collect();
    assert!(
        render(
            &cfg,
            &plan(&cfg).unwrap(),
            "duplicate",
            &mut SwitchHooks {
                terminal: end,
                edge_values
            }
        )
        .is_err()
    );
}
