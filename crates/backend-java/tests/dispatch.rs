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
    assert_eq!(body.matches("case ").count(), 4, "{body}");
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

struct ChainHooks {
    numbers: HashMap<BlockId, usize>,
    callbacks: Vec<String>,
}
impl DispatchEmitter for ChainHooks {
    fn instructions(&mut self, block: BlockId) -> Result<String, String> {
        let number = self.numbers[&block];
        self.callbacks.push(format!("i{number}"));
        Ok(format!(
            "int scratch = {number}; trace += scratch + \",\"; x += scratch; if(fail && scratch==5) throw new IllegalArgumentException(trace);"
        ))
    }
    fn terminal(&mut self, block: BlockId) -> Result<String, String> {
        self.callbacks.push(format!("t{}", self.numbers[&block]));
        Ok("return trace + \":\" + x + \":\" + scratch;".into())
    }
    fn condition(&mut self, _: BlockId) -> Result<String, String> {
        Err("no condition".into())
    }
    fn selector(&mut self, _: BlockId) -> Result<String, String> {
        Err("no selector".into())
    }
    fn copies(&mut self, _: EdgeId) -> Result<String, String> {
        self.callbacks.push("copy".into());
        Ok("trace += \">\";".into())
    }
}

#[test]
fn linear_states_fuse_with_original_scopes_copy_order_and_abrupt_completion() {
    let mut cfg = Cfg::new(());
    let mut blocks = vec![cfg.entry()];
    for _ in 1..17 {
        let next = cfg.add_block(());
        cfg.add_edge(blocks[blocks.len() - 1], next, EdgeKind::Unconditional)
            .unwrap();
        blocks.push(next);
    }
    let mut hooks = ChainHooks {
        numbers: blocks.iter().enumerate().map(|(i, b)| (*b, i)).collect(),
        callbacks: vec![],
    };
    let body = render(&cfg, &plan(&cfg).unwrap(), "merged", &mut hooks).unwrap();
    assert_eq!(body.matches("case ").count(), 1, "{body}");
    assert!(!body.contains("merged_pc = 1;"));
    let mut expected = vec![];
    for i in 0..17 {
        expected.push(format!("i{i}"));
        expected.push(if i == 16 { "t16".into() } else { "copy".into() });
    }
    assert_eq!(hooks.callbacks, expected);
    run_java(
        "MergedDispatch",
        &format!(
            r#"public class MergedDispatch {{
static String generated(boolean fail) {{ String trace="";int x=0;{body} }}
static String reference(boolean fail) {{ String trace="";int x=0;for(int i=0;i<17;i++){{trace+=i+",";x+=i;if(fail&&i==5)throw new IllegalArgumentException(trace);if(i<16)trace+=">";}}return trace+":"+x+":16";}}
static String observe(boolean generated,boolean fail){{try{{return generated?generated(fail):reference(fail);}}catch(IllegalArgumentException e){{return "throw:"+e.getMessage();}}}}
public static void main(String[] args){{for(boolean fail:new boolean[]{{false,true}})if(!observe(true,fail).equals(observe(false,fail)))throw new AssertionError();}}
}}"#
        ),
    );
}

struct RegionHooks {
    names: HashMap<BlockId, String>,
    instruction_text: HashMap<BlockId, String>,
    terminal_text: HashMap<BlockId, String>,
    conditions: HashMap<BlockId, String>,
    copies: HashMap<EdgeId, String>,
    edge_names: HashMap<EdgeId, String>,
    callbacks: Vec<String>,
}
impl DispatchEmitter for RegionHooks {
    fn instructions(&mut self, block: BlockId) -> Result<String, String> {
        self.callbacks.push(format!("i{}", self.names[&block]));
        self.instruction_text
            .get(&block)
            .cloned()
            .ok_or_else(|| "missing block instructions".into())
    }
    fn terminal(&mut self, block: BlockId) -> Result<String, String> {
        self.callbacks.push(format!("t{}", self.names[&block]));
        self.terminal_text
            .get(&block)
            .cloned()
            .ok_or_else(|| "missing terminal".into())
    }
    fn condition(&mut self, block: BlockId) -> Result<String, String> {
        self.callbacks.push(format!("c{}", self.names[&block]));
        self.conditions
            .get(&block)
            .cloned()
            .ok_or_else(|| "missing condition".into())
    }
    fn selector(&mut self, _: BlockId) -> Result<String, String> {
        Err("no selector".into())
    }
    fn copies(&mut self, edge: EdgeId) -> Result<String, String> {
        self.callbacks.push(format!("x{}", self.edge_names[&edge]));
        Ok(self.copies.get(&edge).cloned().unwrap_or_default())
    }
}

#[test]
fn single_entry_diamond_is_structured_with_exact_copies_and_bfs_callbacks() {
    let mut cfg = Cfg::new(());
    let entry = cfg.entry();
    let yes_block = cfg.add_block(());
    let no_block = cfg.add_block(());
    let join = cfg.add_block(());
    let yes_edge = cfg
        .add_edge(entry, yes_block, EdgeKind::BranchTrue)
        .unwrap();
    let no_edge = cfg
        .add_edge(entry, no_block, EdgeKind::BranchFalse)
        .unwrap();
    let yes_join = cfg
        .add_edge(yes_block, join, EdgeKind::Unconditional)
        .unwrap();
    let no_join = cfg
        .add_edge(no_block, join, EdgeKind::Unconditional)
        .unwrap();
    let mut hooks = RegionHooks {
        names: [
            (entry, "E".into()),
            (yes_block, "A".into()),
            (no_block, "B".into()),
            (join, "J".into()),
        ]
        .into(),
        instruction_text: [
            (entry, "int scoped = input; trace += \"E\";".into()),
            (yes_block, "int scoped = 20; trace += \"A\"; x++;".into()),
            (no_block, "int scoped = 30; trace += \"B\"; y--;".into()),
            (join, String::new()),
        ]
        .into(),
        terminal_text: [(join, "return trace + \":\" + x + \":\" + y;".into())].into(),
        conditions: [(entry, "choose(flag)".into())].into(),
        copies: [
            (
                yes_edge,
                "{ int fromX = scoped; int fromY = scoped + 10; x = fromX; y = fromY; trace += \"ta\"; }".into(),
            ),
            (
                no_edge,
                "{ int fromX = scoped + 2; int fromY = scoped + 20; x = fromX; y = fromY; trace += \"tb\"; }".into(),
            ),
            (
                yes_join,
                "{ int oldX = x; int oldY = y; x = oldY; y = oldX; trace += \"aJ\"; }".into(),
            ),
            (
                no_join,
                "{ int oldX = x; int oldY = y; x = oldY; y = oldX; trace += \"bJ\"; }".into(),
            ),
        ]
        .into(),
        edge_names: [
            (yes_edge, "EA".into()),
            (no_edge, "EB".into()),
            (yes_join, "AJ".into()),
            (no_join, "BJ".into()),
        ]
        .into(),
        callbacks: vec![],
    };
    let body = render(&cfg, &plan(&cfg).unwrap(), "diamond", &mut hooks).unwrap();
    assert_eq!(body.matches("case ").count(), 2, "{body}");
    assert!(!body.contains("diamond_pc = 1;\ncontinue diamond_loop;"));
    assert!(!body.contains("diamond_pc = 2;\ncontinue diamond_loop;"));
    assert_eq!(
        hooks.callbacks,
        [
            "iE", "xEA", "xEB", "cE", "iA", "xAJ", "iB", "xBJ", "iJ", "tJ"
        ]
    );
    run_java(
        "DispatchDiamond",
        &format!(
            r#"public class DispatchDiamond {{
static String trace;static int evaluations;
static boolean choose(boolean flag){{evaluations++;trace+="C";return flag;}}
static String generated(boolean flag,int input){{trace="";evaluations=0;int x=3,y=8;{body}}}
static String reference(boolean flag,int input){{trace="";evaluations=0;int x=3,y=8;boolean selected;{{int scoped=input;trace+="E";if(choose(flag)){{{{int fromX=scoped,fromY=scoped+10;x=fromX;y=fromY;trace+="ta";}}selected=true;}}else{{{{int fromX=scoped+2,fromY=scoped+20;x=fromX;y=fromY;trace+="tb";}}selected=false;}}}}if(selected){{int scoped=20;trace+="A";x++;{{int oldX=x,oldY=y;x=oldY;y=oldX;trace+="aJ";}}}}else{{int scoped=30;trace+="B";y--;{{int oldX=x,oldY=y;x=oldY;y=oldX;trace+="bJ";}}}}return trace+":"+x+":"+y;}}
static String observe(boolean generated,boolean flag,int input){{String value=generated?generated(flag,input):reference(flag,input);return value+":"+evaluations;}}
public static void main(String[] args){{for(boolean flag:new boolean[]{{false,true}})for(int input=-2;input<4;input++){{String a=observe(true,flag,input),b=observe(false,flag,input);if(!a.equals(b))throw new AssertionError(a+" != "+b);}}}}
}}"#
        ),
    );
}

#[test]
fn distinct_terminal_branches_are_inlined_without_replaying_the_condition() {
    let mut cfg = Cfg::new(());
    let entry = cfg.entry();
    let yes_block = cfg.add_block(());
    let no_block = cfg.add_block(());
    let yes_edge = cfg
        .add_edge(entry, yes_block, EdgeKind::BranchTrue)
        .unwrap();
    let no_edge = cfg
        .add_edge(entry, no_block, EdgeKind::BranchFalse)
        .unwrap();
    let mut hooks = RegionHooks {
        names: [
            (entry, "E".into()),
            (yes_block, "A".into()),
            (no_block, "B".into()),
        ]
        .into(),
        instruction_text: [
            (entry, "trace += \"E\";".into()),
            (yes_block, "trace += \"A\";".into()),
            (no_block, "trace += \"B\";".into()),
        ]
        .into(),
        terminal_text: [
            (yes_block, "return trace + \":yes\";".into()),
            (no_block, "return trace + \":no\";".into()),
        ]
        .into(),
        conditions: [(entry, "choose(flag)".into())].into(),
        copies: [
            (yes_edge, "trace += \"ta\";".into()),
            (no_edge, "trace += \"tb\";".into()),
        ]
        .into(),
        edge_names: [(yes_edge, "EA".into()), (no_edge, "EB".into())].into(),
        callbacks: vec![],
    };
    let body = render(&cfg, &plan(&cfg).unwrap(), "terminal", &mut hooks).unwrap();
    assert_eq!(body.matches("case ").count(), 1, "{body}");
    assert!(!body.contains("terminal_pc = 1;\ncontinue terminal_loop;"));
    assert!(!body.contains("terminal_pc = 2;\ncontinue terminal_loop;"));
    run_java(
        "DispatchTerminalArms",
        &format!(
            r#"public class DispatchTerminalArms {{static String trace;static int evaluations;static boolean choose(boolean flag){{evaluations++;trace+="C";return flag;}}
static String generated(boolean flag){{trace="";evaluations=0;{body}}}
static String reference(boolean flag){{trace="E";evaluations=0;if(choose(flag)){{trace+="ta";trace+="A";return trace+":yes";}}else{{trace+="tb";trace+="B";return trace+":no";}}}}
static String observe(boolean generated,boolean flag){{String value=generated?generated(flag):reference(flag);return value+":"+evaluations;}}
public static void main(String[] args){{for(boolean flag:new boolean[]{{false,true}})if(!observe(true,flag).equals(observe(false,flag)))throw new AssertionError();}}
}}"#
        ),
    );
}

#[test]
fn single_entry_linear_natural_loop_is_structured_with_copy_and_throw_order() {
    let mut cfg = Cfg::new(());
    let header = cfg.entry();
    let body_block = cfg.add_block(());
    let exit = cfg.add_block(());
    let body_edge = cfg
        .add_edge(header, body_block, EdgeKind::BranchFalse)
        .unwrap();
    let exit_edge = cfg.add_edge(header, exit, EdgeKind::BranchTrue).unwrap();
    let latch = cfg
        .add_edge(body_block, header, EdgeKind::Unconditional)
        .unwrap();
    let mut hooks = RegionHooks {
        names: [
            (header, "H".into()),
            (body_block, "B".into()),
            (exit, "X".into()),
        ]
        .into(),
        instruction_text: [
            (header, "trace += \"H\";".into()),
            (
                body_block,
                "trace += \"B\"; y++; if (failAt == x) throw new IllegalStateException(trace + \":\" + x + \":\" + y);".into(),
            ),
            (exit, String::new()),
        ]
        .into(),
        terminal_text: [(exit, "return trace + \":\" + x + \":\" + y;".into())].into(),
        conditions: [(header, "again(limit, x)".into())].into(),
        copies: [
            (
                body_edge,
                "{ int oldX = x; int oldY = y; x = oldY; y = oldX; trace += \"hb\"; }".into(),
            ),
            (exit_edge, "trace += \"hx\";".into()),
            (
                latch,
                "{ int oldX = x; int oldY = y; x = oldY; y = oldX + 1; trace += \"bh\"; }".into(),
            ),
        ]
        .into(),
        edge_names: [
            (body_edge, "HB".into()),
            (exit_edge, "HX".into()),
            (latch, "BH".into()),
        ]
        .into(),
        callbacks: vec![],
    };
    let body = render(&cfg, &plan(&cfg).unwrap(), "natural", &mut hooks).unwrap();
    assert_eq!(body.matches("case ").count(), 2, "{body}");
    assert!(body.contains("while (true)"), "{body}");
    assert!(!body.contains("natural_pc = 1;"));
    run_java(
        "DispatchNaturalLoop",
        &format!(
            r#"public class DispatchNaturalLoop {{static String trace;static int conditions;
static boolean again(int limit,int x){{conditions++;trace+="C";return x>=limit;}}
static String generated(int limit,int failAt){{trace="";conditions=0;int x=0,y=10;{body}}}
static String reference(int limit,int failAt){{trace="";conditions=0;int x=0,y=10;while(true){{trace+="H";if(!again(limit,x)){{{{int oldX=x,oldY=y;x=oldY;y=oldX;trace+="hb";}}trace+="B";y++;if(failAt==x)throw new IllegalStateException(trace+":"+x+":"+y);{{int oldX=x,oldY=y;x=oldY;y=oldX+1;trace+="bh";}}}}else{{trace+="hx";break;}}}}return trace+":"+x+":"+y;}}
static String observe(boolean generated,int limit,int failAt){{try{{String value=generated?generated(limit,failAt):reference(limit,failAt);return value+":"+conditions;}}catch(IllegalStateException e){{return "throw:"+e.getMessage()+":"+conditions;}}}}
public static void main(String[] args){{for(int limit=0;limit<7;limit++)for(int failAt=-1;failAt<7;failAt++){{String a=observe(true,limit,failAt),b=observe(false,limit,failAt);if(!a.equals(b))throw new AssertionError(a+" != "+b);}}}}
}}"#
        ),
    );
}
