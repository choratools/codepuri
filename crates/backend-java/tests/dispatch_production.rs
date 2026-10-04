//! Full typed backend dispatcher integration, preserving ordinary SSA semantics.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (name, parameters, return_type) = match r.index() {
            0 => ("a", vec![], JavaType::Boolean),
            1 => ("b", vec![], JavaType::Boolean),
            3 => ("select", vec![JavaType::Boolean], JavaType::Int),
            2 => ("result", vec![JavaType::Int, JavaType::Int], JavaType::Int),
            _ => return None,
        };
        Some(JavaMethod {
            owner: "DispatchProduction".into(),
            name: name.into(),
            parameters,
            return_type,
            is_static: true,
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
struct Build {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
    parameters: Vec<ValueId>,
    parameter_blocks: Vec<BlockId>,
}
impl Build {
    fn new() -> Self {
        let mut insns = InsnArena::with_key();
        let id = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(id),
            insns,
            values: ValueArena::with_key(),
            parameters: vec![],
            parameter_blocks: vec![],
        }
    }
    fn param(&mut self, ty: ManagedType) -> ValueId {
        let mut result = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, ty));
            result = Some(v);
            ManagedInsn::Opaque {
                family_tag: 0x8001,
                reads: vec![],
                writes: vec![v],
                effects: managed_ir::effects::EffectFlags::NONE,
            }
        });
        self.parameter_blocks.push(self.cfg.add_block(id));
        let value = result.unwrap();
        self.parameters.push(value);
        value
    }
    fn block(&mut self) -> BlockId {
        self.cfg
            .add_block(self.insns.insert(ManagedInsn::Return { value: None }))
    }
    fn value(&mut self, b: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(b).unwrap().payload(), ty))
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        let id = *self.cfg.block(b).unwrap().payload();
        self.insns[id] = i;
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> EdgeId {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap()
    }
    fn branch(&mut self, b: BlockId, v: ValueId, a: BlockId, z: BlockId) -> (EdgeId, EdgeId) {
        let yes = self.cfg.add_edge(b, a, EdgeKind::BranchTrue).unwrap();
        let no = self.cfg.add_edge(b, z, EdgeKind::BranchFalse).unwrap();
        self.set(
            b,
            ManagedInsn::Branch {
                condition: Some(v),
                then_edge: yes,
                else_edge: Some(no),
            },
        );
        (yes, no)
    }
}
fn fixture(
    bad_type: bool,
    unbound: bool,
    switching: bool,
) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut f = Build::new();
    let left = f.param(ManagedType::Boolean);
    let x = f.param(ManagedType::Int);
    let y = f.param(if bad_type {
        ManagedType::Float
    } else {
        ManagedType::Int
    });
    let a = f.block();
    let ay = f.block();
    let aeffect = f.block();
    let abranch = f.block();
    let ar = f.block();
    let aret = f.block();
    let b = f.block();
    let by = f.block();
    let beffect = f.block();
    let bbranch = f.block();
    let br = f.block();
    let bret = f.block();
    let ax = f.value(
        a,
        if bad_type {
            ManagedType::Float
        } else {
            ManagedType::Int
        },
    );
    let av = f.value(ay, ManagedType::Int);
    let bx = f.value(b, ManagedType::Int);
    let bv = f.value(by, ManagedType::Int);
    let ac = f.value(aeffect, ManagedType::Boolean);
    let bc = f.value(beffect, ManagedType::Boolean);
    let ra = f.value(ar, ManagedType::Int);
    let rb = f.value(br, ManagedType::Int);
    let entry = f.block();
    let mut previous = f.cfg.entry();
    for block in f.parameter_blocks.clone().into_iter().chain([entry]) {
        let edge = f.link(previous, block);
        if previous == f.cfg.entry() {
            f.set(
                previous,
                ManagedInsn::Branch {
                    condition: None,
                    then_edge: edge,
                    else_edge: None,
                },
            );
        }
        previous = block;
    }
    let selector = f.value(entry, ManagedType::Int);
    f.set(
        entry,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(selector),
            method: MethodRef::new(3),
            args: vec![left],
        },
    );
    let decision = f.block();
    f.link(entry, decision);
    let (entry_a, entry_b) = if switching {
        let yes = f.cfg.add_edge(decision, a, EdgeKind::Switch(1)).unwrap();
        let no = f
            .cfg
            .add_edge(decision, b, EdgeKind::Unconditional)
            .unwrap();
        f.set(
            decision,
            ManagedInsn::Switch {
                key: selector,
                default_edge: no,
                cases: vec![(1, yes)],
            },
        );
        (yes, no)
    } else {
        f.branch(decision, left, a, b)
    };
    f.link(a, ay);
    f.link(ay, aeffect);
    f.link(aeffect, abranch);
    f.link(ar, aret);
    f.link(b, by);
    f.link(by, beffect);
    f.link(beffect, bbranch);
    f.link(br, bret);
    let (a_b, _) = f.branch(abranch, ac, b, ar);
    let (b_a, _) = f.branch(bbranch, bc, a, br);
    f.set(
        a,
        ManagedInsn::Phi {
            result: ax,
            inputs: vec![(entry_a, x), (b_a, bv)],
        },
    );
    f.set(
        ay,
        ManagedInsn::Phi {
            result: av,
            inputs: vec![(entry_a, y), (b_a, bx)],
        },
    );
    f.set(
        b,
        ManagedInsn::Phi {
            result: bx,
            inputs: vec![(entry_b, x), (a_b, ax)],
        },
    );
    f.set(
        by,
        ManagedInsn::Phi {
            result: bv,
            inputs: vec![(entry_b, y), (a_b, av)],
        },
    );
    for (block, result, index, args) in [
        (aeffect, ac, 0, vec![]),
        (beffect, bc, 1, vec![]),
        (ar, ra, 2, vec![ax, av]),
        (br, rb, 2, vec![bx, bv]),
    ] {
        f.set(
            block,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(index),
                args,
            },
        );
    }
    f.set(aret, ManagedInsn::Return { value: Some(ra) });
    f.set(bret, ManagedInsn::Return { value: Some(rb) });
    if unbound {
        f.parameters.pop();
    }
    (
        MethodIr::<SsaForm>::from_parts(f.cfg, f.insns, f.values),
        ParameterBindings {
            parameters: f.parameters,
            ..Default::default()
        },
    )
}
fn emit(ir: &MethodIr<SsaForm>, bindings: &ParameterBindings) -> SourceResult {
    backend_java::emit_method(
        ir,
        &JavaMethod {
            owner: "DispatchProduction".into(),
            name: "generated".into(),
            parameters: bindings
                .parameters
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    if i == 0 {
                        JavaType::Boolean
                    } else if ir.value(bindings.parameters[i]).unwrap().ty() == &ManagedType::Float
                    {
                        JavaType::Float
                    } else {
                        JavaType::Int
                    }
                })
                .collect(),
            return_type: JavaType::Int,
            is_static: true,
            throws: vec![],
        },
        bindings,
        &Resolver,
    )
}
#[test]
fn full_backend_irreducible_phi_swaps_default_locals_never_observed() {
    execute_production(false);
    execute_production(true);
}
fn execute_production(switching: bool) {
    let (ir, bindings) = fixture(false, false, switching);
    let diagnostics = managed_ir::verify::SsaVerifier::new().verify(&ir);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(decomp_structure::structure_extended(ir.cfg()).is_err());
    let emitted = emit(&ir, &bindings);
    let SourceResult::Complete { source, .. } = emitted else {
        panic!("{emitted:?}")
    };
    assert!(source.contains("switch ("), "{source}");
    let src = format!(
        r#"public class DispatchProduction {{static int remaining, failAt, visits;static String trace;
static boolean tick(String tag){{trace+=tag;visits++;if(visits==failAt)throw new IllegalStateException(trace);return --remaining>0;}}
static int select(boolean left){{trace+="E";return left?1:0;}}
static boolean a(){{return tick("A");}}static boolean b(){{return tick("B");}}
static int result(int x,int y){{trace+="R";return x*100+y;}}
{source}
static int reference(boolean left,int x,int y){{trace+="E";boolean atA=left;while(true){{if(!tick(atA?"A":"B"))return result(x,y);if(!atA){{int t=x;x=y;y=t;}}atA=!atA;}}}}
static String observe(boolean generated,boolean left,int count,int fail){{remaining=count;failAt=fail;visits=0;trace="";try{{int value=generated?generated(left,17,29):reference(left,17,29);return value+":"+trace;}}catch(IllegalStateException e){{return "throw:"+e.getMessage();}}}}
public static void main(String[] args){{for(boolean left:new boolean[]{{false,true}})for(int count=0;count<12;count++)for(int fail=0;fail<5;fail++){{String a=observe(true,left,count,fail),b=observe(false,left,count,fail);if(!a.equals(b))throw new AssertionError(a+" != "+b);}}}}
}}"#
    );
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let dir = std::env::temp_dir().join(format!(
        "decomphify-dispatch-production-{}-{switching}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("DispatchProduction.java"), src).unwrap();
    for (tool, args) in [
        ("javac", vec!["--release", "17", "DispatchProduction.java"]),
        ("java", vec!["-cp", ".", "DispatchProduction"]),
    ] {
        let out = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
#[test]
fn full_backend_rejects_phi_type_conflict_and_unbound_initial_read() {
    for (bad_type, unbound) in [(true, false), (false, true)] {
        let (ir, bindings) = fixture(bad_type, unbound, false);
        let result = emit(&ir, &bindings);
        assert!(
            matches!(result, SourceResult::Unsupported(_)),
            "bad_type={bad_type} unbound={unbound}: {result:?}"
        );
    }
}

#[test]
fn entry_phi_without_initial_edge_rejects_self_cycle_and_default_read() {
    use managed_ir::ops::{CompareOp, Constant};
    for terminal_form in [false, true] {
        let mut f = Build::new();
        let entry = f.cfg.entry();
        let selected = f.value(entry, ManagedType::Int);
        if terminal_form {
            let constant_block = f.block();
            let comparison_block = f.block();
            let branch_block = f.block();
            let return_block = f.block();
            let zero = f.value(constant_block, ManagedType::Int);
            let condition = f.value(comparison_block, ManagedType::Boolean);
            f.link(entry, constant_block);
            f.link(constant_block, comparison_block);
            f.link(comparison_block, branch_block);
            let (backedge, _) = f.branch(branch_block, condition, entry, return_block);
            f.set(
                entry,
                ManagedInsn::Phi {
                    result: selected,
                    inputs: vec![(backedge, selected)],
                },
            );
            f.set(
                constant_block,
                ManagedInsn::Const {
                    result: zero,
                    value: Constant::Int(0),
                },
            );
            f.set(
                comparison_block,
                ManagedInsn::Compare {
                    op: CompareOp::Equal,
                    result: condition,
                    lhs: selected,
                    rhs: zero,
                },
            );
            f.set(
                return_block,
                ManagedInsn::Return {
                    value: Some(selected),
                },
            );
        } else {
            let backedge = f.link(entry, entry);
            f.set(
                entry,
                ManagedInsn::Phi {
                    result: selected,
                    inputs: vec![(backedge, selected)],
                },
            );
        }
        let ir = MethodIr::<SsaForm>::from_parts(f.cfg, f.insns, f.values);
        // Structural dominance alone permits these cyclic definitions; there is
        // nevertheless no original incoming edge selecting a first-iteration value.
        let diagnostics = managed_ir::verify::SsaVerifier::new().verify(&ir);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(
            backend_java::branching::plan_dispatch_checked(
                &ir,
                &[],
                &std::collections::HashSet::new()
            )
            .is_err()
        );
        assert!(matches!(
            emit(&ir, &ParameterBindings::default()),
            SourceResult::Unsupported(_)
        ));
    }
}
