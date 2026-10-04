//! Standalone module integration and JDK17 execution, without shared exports.
#![allow(clippy::unwrap_used)]
pub use decomp_structure::{EdgeProvenance, StructureError};
#[path = "../src/branching.rs"]
pub mod branching;
#[path = "../../decomp-structure/src/loop_control.rs"]
pub mod loop_control_regions;
#[path = "../src/loop_control.rs"]
pub mod lowering;
pub use lowering as loop_control;
#[path = "../src/switching.rs"]
pub mod switching;
use branching::{Copy, CopyAction, Step};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use lowering::{ControlStep, Environment, Lowering};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, ValueArena},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
use std::collections::HashMap;

struct Harness {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
    text: HashMap<InsnId, String>,
    conditions: HashMap<BlockId, ValueId>,
    names: HashMap<ValueId, String>,
    copies: HashMap<EdgeId, Vec<Copy>>,
}
impl Harness {
    fn new() -> Self {
        let mut insns = InsnArena::with_key();
        let id = insns.insert(ManagedInsn::Return { value: None });
        let mut s = Self {
            cfg: Cfg::new(id),
            insns,
            values: ValueArena::with_key(),
            text: HashMap::new(),
            conditions: HashMap::new(),
            names: HashMap::new(),
            copies: HashMap::new(),
        };
        s.text.insert(id, String::new());
        s
    }
    fn block(&mut self, text: &str) -> BlockId {
        let i = self.insns.insert(ManagedInsn::Return { value: None });
        self.text.insert(i, text.into());
        self.cfg.add_block(i)
    }
    fn value(&mut self, name: &str) -> ValueId {
        let i = self.insns.insert(ManagedInsn::Return { value: None });
        let v = self.values.insert(Value::new(i, ManagedType::Int));
        self.names.insert(v, name.into());
        v
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> EdgeId {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap()
    }
    fn branch(&mut self, b: BlockId, expression: &str, yes: BlockId, no: BlockId) {
        let v = self.value(expression);
        self.conditions.insert(b, v);
        self.cfg.add_edge(b, yes, EdgeKind::BranchTrue).unwrap();
        self.cfg.add_edge(b, no, EdgeKind::BranchFalse).unwrap();
    }
    fn source(&mut self, name: &str) -> String {
        let region = loop_control_regions::structure(&self.cfg).unwrap();
        assert_eq!(region.edges.len(), self.cfg.edge_ids().count());
        let env = self.names.keys().copied().collect();
        let (steps, _) = lowering::lower(&region.root, self, &mut { env }).unwrap();
        let mut labels = HashMap::new();
        let mut serial = 0;
        let body = self.render(&steps, &mut labels, &mut serial);
        format!(
            "static int {name}(int n,int limit) {{ int count=0,a=3,b=8,j=0; boolean c=false; {body} }}"
        )
    }
    fn render(
        &self,
        steps: &[ControlStep],
        labels: &mut HashMap<BlockId, String>,
        serial: &mut usize,
    ) -> String {
        let mut out = String::new();
        for s in steps {
            match s {
                ControlStep::Ordinary(Step::Instruction(i)) => out.push_str(&self.text[i]),
                ControlStep::Ordinary(Step::Copies { copies, .. }) => {
                    let batch = *serial;
                    *serial += 1;
                    for a in branching::schedule_copies(copies) {
                        match a {
                            CopyAction::Snapshot { slot, source } => out
                                .push_str(&format!("int t{batch}_{slot}={};", self.names[&source])),
                            CopyAction::Assign { destination, slot } => out.push_str(&format!(
                                "{}=t{batch}_{slot};",
                                self.names[&destination]
                            )),
                        }
                    }
                }
                ControlStep::Ordinary(other) => panic!("unexpected {other:?}"),
                ControlStep::If {
                    condition,
                    then_steps,
                    else_steps,
                } => {
                    out.push_str(&format!("if ({}) {{", self.names[condition]));
                    out.push_str(&self.render(then_steps, labels, serial));
                    out.push_str("} else {");
                    out.push_str(&self.render(else_steps, labels, serial));
                    out.push('}');
                }
                ControlStep::Loop { header, body } => {
                    let label = format!("loop{}", labels.len());
                    labels.insert(*header, label.clone());
                    out.push_str(&format!("{label}: while(true) {{"));
                    out.push_str(&self.render(body, labels, serial));
                    out.push('}');
                }
                ControlStep::Transfer { kind, .. } => {
                    use loop_control_regions::TransferKind::*;
                    match kind {
                        Break { header } => out.push_str(&format!("break {};", labels[header])),
                        Continue { header } => {
                            out.push_str(&format!("continue {};", labels[header]))
                        }
                    }
                }
            }
        }
        out
    }
}
impl Lowering for Harness {
    type Error = String;
    fn block(&mut self, b: BlockId, _: &mut Environment) -> Result<(Vec<Step>, bool), String> {
        let id = *self.cfg.block(b).unwrap().payload();
        Ok((
            vec![Step::Instruction(id)],
            !self.cfg.block(b).unwrap().outgoing().is_empty(),
        ))
    }
    fn condition(&mut self, b: BlockId, _: &Environment) -> Result<ValueId, String> {
        Ok(self.conditions[&b])
    }
    fn edge(&mut self, e: EdgeId, _: &mut Environment) -> Result<Vec<Step>, String> {
        Ok(self
            .copies
            .get(&e)
            .map(|c| {
                vec![Step::Copies {
                    edge: e,
                    copies: c.clone(),
                }]
            })
            .unwrap_or_default())
    }
    fn outgoing(&mut self, b: BlockId) -> Result<Option<EdgeId>, String> {
        Ok(self.cfg.block(b).unwrap().outgoing().first().copied())
    }
}
fn fixture(mode: u8) -> Harness {
    let mut f = Harness::new();
    let entry = f.cfg.entry();
    let test = f.block("");
    let choose = f.block("");
    let decrement = f.block("n--; effects++;");
    let parity = f.block("");
    let count = f.block("count++;");
    let exit = f.block("return count*100+a;");
    f.link(entry, test);
    f.branch(test, "n>0", choose, exit);
    let ret = f.block(if mode == 1 {
        "effects+=1000; return count*100+a;"
    } else {
        ""
    });
    f.branch(choose, "n==limit", ret, decrement);
    let break_edge = if mode != 1 {
        Some(f.link(ret, exit))
    } else {
        None
    };
    f.link(decrement, parity);
    f.branch(parity, "(n%2)==0", test, count);
    let back = f.link(count, test);
    let a = f.value("a");
    let b = f.value("b");
    let swaps = vec![
        Copy {
            destination: a,
            source: b,
        },
        Copy {
            destination: b,
            source: a,
        },
    ];
    f.copies.insert(back, swaps.clone());
    if let Some(edge) = break_edge {
        f.copies.insert(edge, swaps);
    }
    f
}
#[test]
fn jdk17_break_initial_continue_return_effects_and_phi_swaps() {
    let mut methods = String::new();
    for (name, mode) in [("early", 0), ("abrupt", 1)] {
        methods.push_str(&fixture(mode).source(name));
    }
    // A real posttest CFG: body effect precedes the first conditional test.
    let mut f = Harness::new();
    let entry = f.cfg.entry();
    let body = f.block("n--; count++; effects++;");
    let test = f.block("");
    let exit = f.block("return count;");
    f.link(entry, body);
    f.link(body, test);
    f.branch(test, "n>0", body, exit);
    methods.push_str(&f.source("post"));
    let mut f = Harness::new();
    let entry = f.cfg.entry();
    let outer = f.block("");
    let initial = f.block("j=n;");
    let inner = f.block("");
    let dec = f.block("j--; effects++;");
    let choose = f.block("");
    let count = f.block("count++;");
    let next = f.block("n--;");
    let exit = f.block("return count;");
    f.link(entry, outer);
    f.branch(outer, "n>0", initial, exit);
    f.link(initial, inner);
    f.branch(inner, "j>0", dec, next);
    f.link(dec, choose);
    f.branch(choose, "j==limit", next, count);
    f.link(count, inner);
    f.link(next, outer);
    methods.push_str(&f.source("nested"));
    let mut f = Harness::new();
    let entry = f.cfg.entry();
    let evaluation = f.block("tests++; c=n>0;");
    let test = f.block("");
    let body = f.block("n--; count++;");
    let exit = f.block("return count;");
    f.link(entry, evaluation);
    f.link(evaluation, test);
    f.branch(test, "c", body, exit);
    f.link(body, evaluation);
    methods.push_str(&f.source("evaluated"));
    let source = format!(
        r#"public class LoopControlMain {{ static int effects,tests; {methods}
static void eq(int a,int b) {{if(a!=b)throw new AssertionError(a+" != "+b);}}
static int reference(int n,int limit,boolean ret) {{int count=0,a=3,b=8;while(n>0){{if(n==limit){{if(ret)effects+=1000;else{{int t=a;a=b;b=t;}}break;}}n--;effects++;if(n%2==0)continue;count++;int t=a;a=b;b=t;}}return count*100+a;}}
public static void main(String[] args) {{for(int n=-2;n<12;n++)for(int limit=-1;limit<13;limit++){{effects=0;int expected=reference(n,limit,false),e=effects;effects=0;eq(early(n,limit),expected);eq(effects,e);effects=0;expected=reference(n,limit,true);e=effects;effects=0;eq(abrupt(n,limit),expected);eq(effects,e);int nc=0,ne=0;for(int x=n;x>0;x--)for(int j=x;j>0;){{j--;ne++;if(j==limit)break;nc++;}}effects=0;eq(nested(n,limit),nc);eq(effects,ne);}}for(int n=-3;n<10;n++){{effects=0;eq(post(n,0),Math.max(1,n));eq(effects,Math.max(1,n));tests=0;eq(evaluated(n,0),Math.max(0,n));eq(tests,Math.max(0,n)+1);}}System.out.println("loop control assertions passed");}}
}}"#
    );
    let dir = std::env::temp_dir().join(format!("decomphify-control-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("LoopControlMain.java"), &source).unwrap();
    let result = std::process::Command::new("javac")
        .args(["--release", "17", "LoopControlMain.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = std::process::Command::new("java")
        .args(["-cp", ".", "LoopControlMain"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "loop control assertions passed\n"
    );
}
