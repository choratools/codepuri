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

#[test]
fn nested_child_exit_retests_parent_once_and_preserves_edge_copies_and_exception_identity() {
    let mut f = Harness::new();
    let entry = f.cfg.entry();
    let outer = f.block("");
    let select = f.block("");
    let init = f.block("j=0;");
    let inner = f.block("");
    let body = f.block("count++; effects++;");
    let simple = f.block("count+=2; effects+=10;");
    let done = f.block("return count*100+a;");
    f.link(entry, outer);
    f.branch(outer, "outerTest(n-- >0)", select, done);
    f.branch(select, "n%2==0", init, simple);
    f.link(init, inner);
    f.branch(inner, "innerTest(j++<limit)", body, outer);
    f.link(body, inner);
    f.link(simple, outer);
    let exit = *f.cfg.block(inner).unwrap().outgoing().last().unwrap();
    let a = f.value("a");
    let b = f.value("b");
    f.copies.insert(
        exit,
        vec![
            Copy {
                destination: a,
                source: b,
            },
            Copy {
                destination: b,
                source: a,
            },
        ],
    );
    let generated = f.source("generated");
    let source = format!(
        r#"public class ParentExitMain {{ static int effects,trace,calls,failAt; static final RuntimeException ERROR=new RuntimeException("same");
 static boolean outerTest(boolean value){{trace=trace*31+(value?3:4);if(++calls==failAt)throw ERROR;return value;}}
 static boolean innerTest(boolean value){{trace=trace*31+(value?5:6);if(++calls==failAt)throw ERROR;return value;}}
 {generated}
 static int original(int n,int limit){{int count=0,a=3,b=8,j=0;while(outerTest(n-- >0)){{if(n%2==0){{j=0;while(innerTest(j++<limit)){{count++;effects++;}}int tmp=a;a=b;b=tmp;}}else{{count+=2;effects+=10;}}}}return count*100+a;}}
 public static void main(String[]args){{int checks=0;for(int n=-1;n<9;n++)for(int limit=0;limit<5;limit++)for(int fail=0;fail<80;fail++){{effects=trace=calls=0;failAt=fail;int expected=0;RuntimeException error=null;try{{expected=original(n,limit);}}catch(RuntimeException e){{error=e;}}int efx=effects,tr=trace,cs=calls;effects=trace=calls=0;int actual=0;RuntimeException got=null;try{{actual=generated(n,limit);}}catch(RuntimeException e){{got=e;}}if(actual!=expected||got!=error||effects!=efx||trace!=tr||calls!=cs)throw new AssertionError(n+"/"+limit+"/"+fail);checks++;}}System.out.println(checks);}}
}}"#
    );
    for (release, home) in [
        ("17", "/home/jtjisgod/tools/jdk-17.0.13+11"),
        ("25", "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1"),
    ] {
        let dir = std::env::temp_dir().join(format!(
            "parent-child-exit-{}-{release}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ParentExitMain.java"), &source).unwrap();
        for (tool, args) in [
            ("javac", vec!["--release", release, "ParentExitMain.java"]),
            ("java", vec!["-cp", ".", "ParentExitMain"]),
        ] {
            let out = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{source}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            if tool == "java" {
                assert_eq!(String::from_utf8_lossy(&out.stdout), "4000\n");
            }
        }
    }
}
