//! Execution-model checks for structured branch and Phi lowering.
// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]
use backend_java::branching;

use branching::{Copy, CopyAction, PlanError, Step, plan, schedule_copies};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::BlockId,
};
use decomp_structure::{Region, StructureError, structure};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::{Constant, InvokeKind},
    refs::{ClassRef, MethodRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
use std::collections::HashMap;

fn scaling_fixture(joins: usize, phis: usize) -> (MethodIr<SsaForm>, Vec<ValueId>) {
    let mut f = Fixture::new();
    let c = f.parameter(ManagedType::Boolean);
    let mut source = f.parameter(ManagedType::Int);
    let mut header = f.cfg.entry();
    for _ in 0..joins {
        let head = f.block(ManagedInsn::Return { value: None });
        f.branch(header, c, head, head);
        let edges = f.cfg.block(head).unwrap().incoming().to_vec();
        let mut member = head;
        for i in 0..phis {
            let result = f.value(member);
            f.set(
                member,
                ManagedInsn::Phi {
                    result,
                    inputs: edges.iter().map(|&e| (e, source)).collect(),
                },
            );
            if i + 1 < phis {
                let next = f.block(ManagedInsn::Return { value: None });
                f.link(member, next);
                member = next;
            }
            if i + 1 == phis {
                source = result;
            }
        }
        header = f.block(ManagedInsn::Return { value: None });
        f.link(member, header);
    }
    f.set(
        header,
        ManagedInsn::Return {
            value: Some(source),
        },
    );
    f.finish()
}

#[test]
fn chain_phi_many_joins_scaling() {
    use backend_java::{JavaMethod, JavaType, ParameterBindings, SourceResult};
    for (joins, phis) in [(1, 128), (1, 512), (64, 8), (256, 8)] {
        let (ir, bindings) = scaling_fixture(joins, phis);
        let region = structure(ir.cfg()).unwrap().root;
        let start = std::time::Instant::now();
        let p = plan(&ir, &region, &bindings).unwrap();
        let planning = start.elapsed();
        assert_eq!(p.locals.len(), joins * phis);
        for condition in [0, 1] {
            let mut values = HashMap::from([(bindings[0], condition), (bindings[1], 37)]);
            assert_eq!(
                execute(&ir, &p.steps, &mut values, &mut Vec::new()),
                Some(37)
            );
        }
        let start = std::time::Instant::now();
        let result = backend_java::emit_method(
            &ir,
            &JavaMethod {
                owner: "Fixture".into(),
                name: "scale".into(),
                is_static: true,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Boolean, JavaType::Int],
                throws: vec![],
            },
            &ParameterBindings {
                parameters: bindings,
                ..Default::default()
            },
            &SourceResolver,
        );
        let emitting = start.elapsed();
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(source.matches(" = edge").count(), 2 * joins * phis);
        eprintln!(
            "joins={joins} phis={phis} inputs={} plan={planning:?} emit={emitting:?}",
            2 * joins * phis
        );
    }
}

struct Fixture {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
    bindings: Vec<ValueId>,
}

#[test]
fn production_checked_phi_families_compile_execute_and_reject_mixed_escapes() {
    use backend_java::{JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult};
    struct Typed(JavaType);
    impl JavaResolver for Typed {
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            if r.index() == 0 {
                return Some(JavaMethod {
                    owner: "Effects".into(),
                    name: "incompatibleEscape".into(),
                    is_static: true,
                    return_type: JavaType::Void,
                    parameters: vec![self.0.clone()],
                    throws: vec![],
                });
            }
            SourceResolver.method(r)
        }
        fn field(&self, _: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
            None
        }
        fn class(&self, _: managed_ir::refs::ClassRef) -> Option<String> {
            Some("java.lang.String".into())
        }
        fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    let mut sources = Vec::new();
    for (name, managed, java) in [
        ("integer", ManagedType::Int, JavaType::Int),
        ("boolean", ManagedType::Boolean, JavaType::Boolean),
        (
            "reference",
            ManagedType::Reference(managed_ir::refs::ClassRef::new(0)),
            JavaType::Class("java.lang.String".into()),
        ),
    ] {
        let mut f = Fixture::new();
        let condition = f.parameter(ManagedType::Boolean);
        let selected = f.parameter(managed);
        let entry = f.cfg.entry();
        let zero_block = f.block(ManagedInsn::Return { value: None });
        let zero = f.values.insert(Value::new(
            *f.cfg.block(zero_block).unwrap().payload(),
            ManagedType::Zero,
        ));
        f.set(
            zero_block,
            ManagedInsn::Const {
                result: zero,
                value: Constant::Zero,
            },
        );
        let yes = f.block(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            method: MethodRef::new(1),
            args: vec![],
            result: None,
        });
        let no = f.block(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            method: MethodRef::new(2),
            args: vec![],
            result: None,
        });
        let join = f.block(ManagedInsn::Return { value: None });
        let phi = f.values.insert(Value::new(
            *f.cfg.block(join).unwrap().payload(),
            ManagedType::Unknown,
        ));
        f.link(entry, zero_block);
        let branch = f.block(ManagedInsn::Return { value: None });
        f.link(zero_block, branch);
        f.branch(branch, condition, yes, no);
        let a = f.link(yes, join);
        let b = f.link(no, join);
        f.set(
            join,
            ManagedInsn::Phi {
                result: phi,
                inputs: vec![(a, selected), (b, zero)],
            },
        );
        let copy_block = f.block(ManagedInsn::Return { value: None });
        let copy = f.values.insert(Value::new(
            *f.cfg.block(copy_block).unwrap().payload(),
            ManagedType::Unknown,
        ));
        f.set(
            copy_block,
            ManagedInsn::Move {
                result: copy,
                value: phi,
            },
        );
        f.link(join, copy_block);
        let ret = f.block(ManagedInsn::Return { value: Some(copy) });
        f.link(copy_block, ret);
        // Synthetic entry's placeholder is a genuine unconditional branch.
        let edge = f.cfg.block(entry).unwrap().outgoing()[0];
        f.set(
            entry,
            ManagedInsn::Branch {
                condition: None,
                then_edge: edge,
                else_edge: None,
            },
        );
        let (ir, bindings) = f.finish();
        let signature = JavaMethod {
            owner: "Fixture".into(),
            name: name.into(),
            is_static: true,
            parameters: vec![JavaType::Boolean, java.clone()],
            return_type: java,
            throws: vec![],
        };
        let bindings = ParameterBindings {
            parameters: bindings,
            ..Default::default()
        };
        let resolver = Typed(if signature.return_type == JavaType::Boolean {
            JavaType::Int
        } else {
            JavaType::Boolean
        });
        let result = backend_java::emit_method(&ir, &signature, &bindings, &resolver);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{name}: {result:?}")
        };
        sources.push(source);
        let mut instructions = ir.instructions().clone();
        instructions.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            method: MethodRef::new(0),
            args: vec![copy],
            result: None,
        });
        let escaped = MethodIr::<SsaForm>::from_parts(
            Cfg::new(*ir.cfg().block(ir.cfg().entry()).unwrap().payload()),
            instructions,
            ir.values().clone(),
        );
        assert!(
            backend_java::contextual_locals::checked_phi_plan(
                &escaped,
                phi,
                &signature.return_type,
                &signature.return_type,
                &backend_java::type_compatibility::Hierarchy::default(),
                &resolver
            )
            .is_err()
        );
    }
    let root = std::env::temp_dir().join(format!("checked-phi-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Fixture.java"), format!("public class Fixture {{ {} public static void main(String[] args) {{ for(boolean c: new boolean[]{{false,true}}) {{ Effects.log=0; if(integer(c,7)!=(c?7:0)||Effects.log!=(c?1:2)) throw new AssertionError(); Effects.log=0; if(boolean_(c,true)!=c||Effects.log!=(c?1:2)) throw new AssertionError(); Effects.log=0; if(reference(c,\"x\")!=(c?\"x\":null)||Effects.log!=(c?1:2)) throw new AssertionError(); }} }} }} class Effects {{ static int log; static int effect1() {{ log=1; return 0; }} static int effect2() {{ log=2; return 0; }} }}", sources.join("\n").replace(" boolean(", " boolean_("))).unwrap();
    let jdk = std::env::var("JAVA_HOME").unwrap();
    let compiled = std::process::Command::new(format!("{jdk}/bin/javac"))
        .arg(root.join("Fixture.java"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(
        std::process::Command::new(format!("{jdk}/bin/java"))
            .args(["-cp", root.to_str().unwrap(), "Fixture"])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn production_shared_tail_compiles_and_preserves_selected_return() {
    use backend_java::{JavaMethod, JavaType, ParameterBindings, SourceResult};
    let mut f = Fixture::new();
    let a = f.parameter(ManagedType::Boolean);
    let b = f.parameter(ManagedType::Boolean);
    let result = f.parameter(ManagedType::Int);
    let early_result = f.parameter(ManagedType::Int);
    let entry = f.cfg.entry();
    let left = f.block(ManagedInsn::Return { value: None });
    let right = f.block(ManagedInsn::Return { value: None });
    let shared = f.block(ManagedInsn::Return {
        value: Some(result),
    });
    let early = f.block(ManagedInsn::Return {
        value: Some(early_result),
    });
    f.branch(entry, a, left, right);
    f.branch(left, b, shared, early);
    f.link(right, shared);
    let edge = f.cfg.block(right).unwrap().outgoing()[0];
    f.set(
        right,
        ManagedInsn::Branch {
            condition: None,
            then_edge: edge,
            else_edge: None,
        },
    );
    let (ir, bindings) = f.finish();
    let forward = branching::plan_forward(&ir, &bindings).unwrap();
    assert_eq!(forward.regions.blocks.len(), ir.cfg().block_count());
    let generated = backend_java::emit_method(
        &ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "selected".into(),
            is_static: true,
            parameters: vec![
                JavaType::Boolean,
                JavaType::Boolean,
                JavaType::Int,
                JavaType::Int,
            ],
            return_type: JavaType::Int,
            throws: vec![],
        },
        &ParameterBindings {
            parameters: bindings,
            ..Default::default()
        },
        &SourceResolver,
    );
    let SourceResult::Complete { source, .. } = generated else {
        panic!("{generated:?}");
    };
    assert!(source.contains("break forwardBlock_"));
    assert_eq!(source.matches("return p2;").count(), 1);
    assert_eq!(source.matches("return p3;").count(), 1);
    let root = std::env::temp_dir().join(format!(
        "decomphify-production-forward-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Fixture.java"), format!("public class Fixture {{ {source} public static void main(String[] args) {{ for(boolean a: new boolean[]{{false,true}}) for(boolean b: new boolean[]{{false,true}}) if(selected(a,b,73,29)!=(!a || b ? 73 : 29)) throw new AssertionError(); }} }}")).unwrap();
    let jdk = std::env::var("JAVA_HOME").unwrap();
    let compiled = std::process::Command::new(format!("{jdk}/bin/javac"))
        .arg(root.join("Fixture.java"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(
        std::process::Command::new(format!("{jdk}/bin/java"))
            .args(["-cp", root.to_str().unwrap(), "Fixture"])
            .status()
            .unwrap()
            .success()
    );
}
impl Fixture {
    fn new() -> Self {
        let mut insns = InsnArena::with_key();
        let entry = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(entry),
            insns,
            values: ValueArena::with_key(),
            bindings: Vec::new(),
        }
    }
    fn parameter(&mut self, ty: ManagedType) -> ValueId {
        let synthetic = self.insns.insert(ManagedInsn::Return { value: None });
        let v = self.values.insert(Value::new(synthetic, ty));
        self.bindings.push(v);
        v
    }
    fn block(&mut self, insn: ManagedInsn) -> BlockId {
        self.cfg.add_block(self.insns.insert(insn))
    }
    fn set(&mut self, block: BlockId, insn: ManagedInsn) {
        let id = *self.cfg.block(block).unwrap().payload();
        self.insns[id] = insn;
    }
    fn value(&mut self, block: BlockId) -> ValueId {
        self.values.insert(Value::new(
            *self.cfg.block(block).unwrap().payload(),
            ManagedType::Int,
        ))
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> decomp_core::ids::EdgeId {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap()
    }
    fn branch(&mut self, h: BlockId, condition: ValueId, a: BlockId, b: BlockId) {
        // Intentionally insert false before true: polarity is not adjacency.
        let else_edge = self.cfg.add_edge(h, b, EdgeKind::BranchFalse).unwrap();
        let then_edge = self.cfg.add_edge(h, a, EdgeKind::BranchTrue).unwrap();
        self.set(
            h,
            ManagedInsn::Branch {
                condition: Some(condition),
                then_edge,
                else_edge: Some(else_edge),
            },
        );
    }
    fn finish(self) -> (MethodIr<SsaForm>, Vec<ValueId>) {
        (
            MethodIr::<SsaForm>::from_parts(self.cfg, self.insns, self.values),
            self.bindings,
        )
    }
}

// Effect-aware execution model for the lowering contract, independent of Java
// instruction rendering. Invokes record order; copy sources are all snapshotted.
fn execute(
    ir: &MethodIr<SsaForm>,
    steps: &[Step],
    values: &mut HashMap<ValueId, i64>,
    effects: &mut Vec<u32>,
) -> Option<i64> {
    for step in steps {
        match step {
            Step::Switch { dispatch, bodies } => {
                let key = values[&dispatch.key];
                let entry = dispatch
                    .dispatch
                    .iter()
                    .find(|d| d.label.map(i64::from) == Some(key))
                    .unwrap_or_else(|| {
                        dispatch
                            .dispatch
                            .iter()
                            .find(|d| d.label.is_none())
                            .unwrap()
                    });
                let saved: Vec<_> = entry.copies.iter().map(|c| values[&c.source]).collect();
                for (copy, value) in entry.copies.iter().zip(saved) {
                    values.insert(copy.destination, value);
                }
                let target = ir.cfg().edge(entry.edge).unwrap().target();
                let index = dispatch
                    .arms
                    .iter()
                    .position(|a| a.target == target)
                    .unwrap();
                if let Some(v) = execute(ir, &bodies[index].0, values, effects) {
                    return Some(v);
                }
            }
            Step::While {
                evaluation,
                condition,
                body_on_true,
                body_steps,
                exit_steps,
            } => loop {
                assert!(execute(ir, evaluation, values, effects).is_none());
                if (values[condition] != 0) != *body_on_true {
                    assert!(execute(ir, exit_steps, values, effects).is_none());
                    break;
                }
                if let Some(v) = execute(ir, body_steps, values, effects) {
                    return Some(v);
                }
            },
            Step::Copies { copies, .. } => {
                let mut saved = HashMap::new();
                for action in schedule_copies(copies) {
                    match action {
                        CopyAction::Snapshot { slot, source } => {
                            saved.insert(slot, values[&source]);
                        }
                        CopyAction::Assign { destination, slot } => {
                            values.insert(destination, saved[&slot]);
                        }
                    }
                }
            }
            Step::If {
                condition,
                then_steps,
                else_steps,
            } => {
                if let Some(v) = execute(
                    ir,
                    if values[condition] != 0 {
                        then_steps
                    } else {
                        else_steps
                    },
                    values,
                    effects,
                ) {
                    return Some(v);
                }
            }
            Step::Instruction(id) => match ir.instruction(*id).unwrap() {
                ManagedInsn::Const {
                    result,
                    value: Constant::Int(v),
                } => {
                    values.insert(*result, i64::from(*v));
                }
                ManagedInsn::Invoke {
                    result: Some(result),
                    method,
                    ..
                } => {
                    effects.push(method.index());
                    values.insert(*result, i64::from(method.index()));
                }
                ManagedInsn::Return { value: Some(v) } => return Some(values[v]),
                other => panic!("unexpected model instruction {other:?}"),
            },
        }
    }
    None
}

fn diamond_fixture(nested: bool, empty: bool) -> (Fixture, ValueId) {
    let mut f = Fixture::new();
    let c = f.parameter(ManagedType::Boolean);
    let d = f.parameter(ManagedType::Boolean);
    let fallback = f.parameter(ManagedType::Int);
    let h = f.cfg.entry();
    let a = f.block(ManagedInsn::Return { value: None });
    let b = f.block(ManagedInsn::Return { value: None });
    let join = f.block(ManagedInsn::Return { value: None });
    let phi = f.value(join);
    let mut left = a;
    let av;
    if nested {
        let aa = f.block(ManagedInsn::Return { value: None });
        let ab = f.block(ManagedInsn::Return { value: None });
        left = f.block(ManagedInsn::Return { value: None });
        let va = f.value(aa);
        let vb = f.value(ab);
        av = f.value(left);
        f.set(
            aa,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(va),
                method: MethodRef::new(11),
                args: vec![],
            },
        );
        f.set(
            ab,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(vb),
                method: MethodRef::new(12),
                args: vec![],
            },
        );
        f.branch(a, d, aa, ab);
        let ea = f.link(aa, left);
        let eb = f.link(ab, left);
        f.set(
            left,
            ManagedInsn::Phi {
                result: av,
                inputs: vec![(eb, vb), (ea, va)],
            },
        );
    } else {
        av = f.value(a);
        f.set(
            a,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(av),
                method: MethodRef::new(10),
                args: vec![],
            },
        );
    }
    let ea = f.link(left, join);
    let (eb, bv) = if empty {
        // b becomes the join's terminal continuation, so false goes directly
        // to the Phi and must run its copies inside an empty structured arm.
        let eb = f.cfg.add_edge(h, join, EdgeKind::BranchFalse).unwrap();
        let et = f.cfg.add_edge(h, a, EdgeKind::BranchTrue).unwrap();
        f.set(
            h,
            ManagedInsn::Branch {
                condition: Some(c),
                then_edge: et,
                else_edge: Some(eb),
            },
        );
        (eb, fallback)
    } else {
        let bv = f.value(b);
        f.set(
            b,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(bv),
                method: MethodRef::new(20),
                args: vec![],
            },
        );
        f.branch(h, c, a, b);
        (f.link(b, join), bv)
    };
    f.set(
        join,
        ManagedInsn::Phi {
            result: phi,
            inputs: vec![(eb, bv), (ea, av)],
        },
    );
    let tail = if empty {
        b
    } else {
        f.block(ManagedInsn::Return { value: None })
    };
    f.link(join, tail);
    f.set(tail, ManagedInsn::Return { value: Some(phi) });
    (f, phi)
}

fn diamond(nested: bool, empty: bool) -> (MethodIr<SsaForm>, Vec<ValueId>, ValueId) {
    let (f, phi) = diamond_fixture(nested, empty);
    let (ir, bindings) = f.finish();
    (ir, bindings, phi)
}

#[test]
fn diamond_executes_only_selected_effect_and_edge_copy() {
    for nested in [false, true] {
        for empty in [false, true] {
            let (ir, bindings, phi) = diamond(nested, empty);
            let region = structure(ir.cfg()).unwrap().root;
            let p = plan(&ir, &region, &bindings).unwrap();
            assert!(p.locals.contains(&phi));
            assert!(bindings.iter().all(|v| !p.locals.contains(v)));
            assert_eq!(p, plan(&ir, &region, &bindings).unwrap());
            for c in [0, 1] {
                for d in [0, 1] {
                    let mut values =
                        HashMap::from([(bindings[0], c), (bindings[1], d), (bindings[2], 99)]);
                    let mut effects = Vec::new();
                    let expected = if c == 0 {
                        if empty { 99 } else { 20 }
                    } else if nested {
                        if d == 0 { 12 } else { 11 }
                    } else {
                        10
                    };
                    assert_eq!(
                        execute(&ir, &p.steps, &mut values, &mut effects),
                        Some(expected)
                    );
                    assert_eq!(
                        effects,
                        if expected == 99 {
                            vec![]
                        } else {
                            vec![expected as u32]
                        }
                    );
                }
            }
        }
    }
}

struct SourceResolver;
impl backend_java::JavaResolver for SourceResolver {
    fn method(&self, reference: MethodRef) -> Option<backend_java::JavaMethod> {
        Some(backend_java::JavaMethod {
            owner: "Effects".into(),
            name: format!("effect{}", reference.index()),
            is_static: true,
            return_type: backend_java::JavaType::Int,
            parameters: vec![],
            throws: vec![],
        })
    }
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn class(&self, _: managed_ir::refs::ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
}

#[test]
fn integrated_emitter_hoists_declarations_but_keeps_effects_and_single_copies_in_arms() {
    use backend_java::{JavaMethod, JavaType, ParameterBindings, SourceResult, emit_method};
    for nested in [false, true] {
        for empty in [false, true] {
            let (ir, values, _) = diamond(nested, empty);
            let signature = JavaMethod {
                owner: "Fixture".into(),
                name: "choose".into(),
                is_static: true,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Boolean, JavaType::Boolean, JavaType::Int],
                throws: vec![],
            };
            let result = emit_method(
                &ir,
                &signature,
                &ParameterBindings {
                    parameters: values,
                    ..Default::default()
                },
                &SourceResolver,
            );
            let SourceResult::Complete { source, .. } = result else {
                panic!("{result:?}")
            };
            let first_if = source.find("if (p0)").unwrap();
            assert!(source[..first_if].contains("int v0;"), "{source}");
            assert!(
                !source[..first_if].contains("Effects.effect"),
                "effect moved out of arm: {source}"
            );
            for effect in if nested { vec![11, 12] } else { vec![10] } {
                assert_eq!(
                    source.matches(&format!("Effects.effect{effect}()")).count(),
                    1,
                    "{source}"
                );
            }
            assert_eq!(
                source.matches("Effects.effect20()").count(),
                usize::from(!empty)
            );
            assert!(
                !source.contains("int edge"),
                "single-copy edge needs no snapshot: {source}"
            );
            assert!(
                source.contains("v0 = "),
                "Phi destination must still be assigned: {source}"
            );
            assert!(source.rfind("return v").unwrap() > source.rfind("v0 = ").unwrap());
        }
    }
}

struct ObjectResolver;
impl backend_java::JavaResolver for ObjectResolver {
    fn method(&self, reference: MethodRef) -> Option<backend_java::JavaMethod> {
        Some(match reference.index() {
            0 => backend_java::JavaMethod {
                owner: "java.lang.RuntimeException".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: backend_java::JavaType::Void,
                parameters: vec![],
                throws: vec![],
            },
            100 => backend_java::JavaMethod {
                owner: "java.lang.Exception".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: backend_java::JavaType::Void,
                parameters: vec![],
                throws: vec![],
            },
            index => backend_java::JavaMethod {
                owner: "Effects".into(),
                name: format!("effect{index}"),
                is_static: true,
                return_type: backend_java::JavaType::Int,
                parameters: vec![],
                throws: vec![],
            },
        })
    }
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        Some(
            match reference.index() {
                0 => "java.lang.RuntimeException",
                1 => "java.lang.Exception",
                2 => "java.lang.String",
                _ => return None,
            }
            .into(),
        )
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(
        &self,
        actual: &backend_java::JavaType,
        expected: &backend_java::JavaType,
    ) -> bool {
        actual == expected
            || (*actual == backend_java::JavaType::Class("java.lang.RuntimeException".into())
                && (*expected == backend_java::JavaType::Class("java.lang.Throwable".into())
                    || *expected == backend_java::JavaType::Class("java.lang.Exception".into())
                    || *expected == backend_java::JavaType::Class("java.lang.Object".into())))
            || (*actual == backend_java::JavaType::Class("java.lang.Exception".into())
                && (*expected == backend_java::JavaType::Class("java.lang.Throwable".into())
                    || *expected == backend_java::JavaType::Class("java.lang.Object".into())))
            || (*actual == backend_java::JavaType::Class("java.lang.String".into())
                && *expected == backend_java::JavaType::Class("java.lang.Object".into()))
    }
}

fn compile_and_run_fixture(source: &str, main: &str) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "branch-object-init-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("Fixture.java"),
        format!(
            "public class Fixture {{ {source} {main} }} class Effects {{ static int log; static int effect1() {{ log = Effects.log * 10 + 1; return 1; }} static int effect2() {{ log = Effects.log * 10 + 2; return 2; }} }}"
        ),
    )
    .unwrap();
    let jdk =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let compiled = std::process::Command::new(format!("{jdk}/bin/javac"))
        .arg(root.join("Fixture.java"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let ran = std::process::Command::new(format!("{jdk}/bin/java"))
        .args(["-cp", root.to_str().unwrap(), "Fixture"])
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&ran.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn throw_fixture(class: ClassRef, ctor: MethodRef) -> (MethodIr<SsaForm>, Vec<ValueId>) {
    let mut f = Fixture::new();
    let condition = f.parameter(ManagedType::Boolean);
    let fallback = f.parameter(ManagedType::Reference(ClassRef::new(0)));
    let entry = f.cfg.entry();
    let ret = f.block(ManagedInsn::Return {
        value: Some(fallback),
    });
    let alloc = f.block(ManagedInsn::Return { value: None });
    let obj = f.values.insert(Value::new(
        *f.cfg.block(alloc).unwrap().payload(),
        ManagedType::Reference(class),
    ));
    f.set(alloc, ManagedInsn::NewInstance { result: obj, class });
    let init = f.block(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: ctor,
        args: vec![obj],
    });
    let throw = f.block(ManagedInsn::Throw { value: obj });
    f.branch(entry, condition, ret, alloc);
    f.link(alloc, init);
    f.link(init, throw);
    f.finish()
}

#[test]
fn branch_arm_throw_emits_and_runs_unchecked_exception_terminal() {
    use backend_java::{JavaMethod, JavaType, ParameterBindings, SourceResult, emit_method};
    let (ir, bindings) = throw_fixture(ClassRef::new(0), MethodRef::new(0));
    let region = structure(ir.cfg()).unwrap().root;
    let plan = plan(&ir, &region, &bindings).unwrap();
    assert!(
        plan.steps
            .iter()
            .any(|step| matches!(step, Step::If { .. }))
    );
    let result = emit_method(
        &ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "chooseOrThrow".into(),
            is_static: true,
            return_type: JavaType::Class("java.lang.RuntimeException".into()),
            parameters: vec![
                JavaType::Boolean,
                JavaType::Class("java.lang.RuntimeException".into()),
            ],
            throws: vec![],
        },
        &ParameterBindings {
            parameters: bindings,
            ..Default::default()
        },
        &ObjectResolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(
        source.matches("new java.lang.RuntimeException()").count(),
        1,
        "{source}"
    );
    assert!(source.contains("throw v"), "{source}");
    let before_branch = source.split("if (").next().unwrap();
    assert!(
        !before_branch.contains("new java.lang.RuntimeException"),
        "{source}"
    );
    assert!(
        !source.contains("= null"),
        "allocation slots are declarations, not initialized objects: {source}"
    );
    compile_and_run_fixture(
        &source,
        "public static void main(String[] args) { java.lang.RuntimeException ok = new java.lang.RuntimeException(); if (chooseOrThrow(true, ok) != ok) throw new AssertionError(); try { chooseOrThrow(false, ok); throw new AssertionError(); } catch (java.lang.RuntimeException expected) {} }",
    );
}

#[test]
fn branch_arm_throw_checked_exception_requires_declared_throws() {
    use backend_java::{
        JavaMethod, JavaType, ParameterBindings, SourceResult, Unsupported, emit_method,
    };
    let (ir, bindings) = throw_fixture(ClassRef::new(1), MethodRef::new(100));
    let mut method = JavaMethod {
        owner: "Fixture".into(),
        name: "checked".into(),
        is_static: true,
        return_type: JavaType::Class("java.lang.RuntimeException".into()),
        parameters: vec![
            JavaType::Boolean,
            JavaType::Class("java.lang.RuntimeException".into()),
        ],
        throws: vec![],
    };
    let rejected = emit_method(
        &ir,
        &method,
        &ParameterBindings {
            parameters: bindings.clone(),
            ..Default::default()
        },
        &ObjectResolver,
    );
    assert!(
        matches!(rejected, SourceResult::Unsupported(Unsupported::Metadata(ref reason)) if reason.contains("checked exception")),
        "{rejected:?}"
    );
    method.throws.push("java.lang.Exception".into());
    let result = emit_method(
        &ir,
        &method,
        &ParameterBindings {
            parameters: bindings,
            ..Default::default()
        },
        &ObjectResolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(source.contains("throws java.lang.Exception"), "{source}");
    compile_and_run_fixture(
        &source,
        "public static void main(String[] args) throws Exception { java.lang.RuntimeException ok = new java.lang.RuntimeException(); if (checked(true, ok) != ok) throw new AssertionError(); try { checked(false, ok); throw new AssertionError(); } catch (java.lang.Exception expected) {} }",
    );
}

#[test]
fn branch_throw_non_throwable_or_unavailable_type_rejects() {
    use backend_java::{JavaMethod, JavaType, ParameterBindings, SourceResult, emit_method};
    for class in [ClassRef::new(2), ClassRef::new(99)] {
        let (ir, bindings) = throw_fixture(class, MethodRef::new(0));
        let result = emit_method(
            &ir,
            &JavaMethod {
                owner: "Fixture".into(),
                name: "badThrow".into(),
                is_static: true,
                return_type: JavaType::Class("java.lang.RuntimeException".into()),
                parameters: vec![
                    JavaType::Boolean,
                    JavaType::Class("java.lang.RuntimeException".into()),
                ],
                throws: vec![],
            },
            &ParameterBindings {
                parameters: bindings,
                ..Default::default()
            },
            &ObjectResolver,
        );
        assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
    }
}

#[test]
fn branch_shared_tail_allocation_preserves_arm_effect_order() {
    use backend_java::{JavaMethod, JavaType, ParameterBindings, SourceResult, emit_method};
    let mut f = Fixture::new();
    let condition = f.parameter(ManagedType::Boolean);
    let entry = f.cfg.entry();
    let yes = f.block(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(1),
        args: vec![],
    });
    let no = f.block(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(2),
        args: vec![],
    });
    let alloc = f.block(ManagedInsn::Return { value: None });
    let obj = f.values.insert(Value::new(
        *f.cfg.block(alloc).unwrap().payload(),
        ManagedType::Reference(ClassRef::new(0)),
    ));
    f.set(
        alloc,
        ManagedInsn::NewInstance {
            result: obj,
            class: ClassRef::new(0),
        },
    );
    let init = f.block(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![obj],
    });
    let ret = f.block(ManagedInsn::Return { value: Some(obj) });
    f.branch(entry, condition, yes, no);
    f.link(yes, alloc);
    f.link(no, alloc);
    f.link(alloc, init);
    f.link(init, ret);
    let (ir, bindings) = f.finish();
    let result = emit_method(
        &ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "sharedTail".into(),
            is_static: true,
            return_type: JavaType::Class("java.lang.RuntimeException".into()),
            parameters: vec![JavaType::Boolean],
            throws: vec![],
        },
        &ParameterBindings {
            parameters: bindings,
            ..Default::default()
        },
        &ObjectResolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(
        source.matches("new java.lang.RuntimeException()").count(),
        1,
        "{source}"
    );
    compile_and_run_fixture(
        &source,
        "public static void main(String[] args) { Effects.log = 0; if (sharedTail(true) == null || Effects.log != 1) throw new AssertionError(); Effects.log = 0; if (sharedTail(false) == null || Effects.log != 2) throw new AssertionError(); }",
    );
}

#[test]
fn branch_throw_uninitialized_allocation_still_rejects() {
    use backend_java::{JavaMethod, JavaType, ParameterBindings, SourceResult, emit_method};
    let mut f = Fixture::new();
    let condition = f.parameter(ManagedType::Boolean);
    let fallback = f.parameter(ManagedType::Reference(ClassRef::new(0)));
    let entry = f.cfg.entry();
    let ret = f.block(ManagedInsn::Return {
        value: Some(fallback),
    });
    let alloc = f.block(ManagedInsn::Return { value: None });
    let obj = f.values.insert(Value::new(
        *f.cfg.block(alloc).unwrap().payload(),
        ManagedType::Reference(ClassRef::new(0)),
    ));
    f.set(
        alloc,
        ManagedInsn::NewInstance {
            result: obj,
            class: ClassRef::new(0),
        },
    );
    let throw = f.block(ManagedInsn::Throw { value: obj });
    f.branch(entry, condition, ret, alloc);
    f.link(alloc, throw);
    let (ir, bindings) = f.finish();
    let result = emit_method(
        &ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "bad".into(),
            is_static: true,
            return_type: JavaType::Class("java.lang.RuntimeException".into()),
            parameters: vec![
                JavaType::Boolean,
                JavaType::Class("java.lang.RuntimeException".into()),
            ],
            throws: vec![],
        },
        &ParameterBindings {
            parameters: bindings,
            ..Default::default()
        },
        &ObjectResolver,
    );
    assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
}

#[test]
fn unused_descriptor_positions_have_no_fabricated_values() {
    use backend_java::{
        DeclarationBindings, JavaMethod, JavaType, SourceResult, emit_method_declaration,
    };
    let mut f = Fixture::new();
    let entry = f.cfg.entry();
    f.set(entry, ManagedInsn::Return { value: None });
    let (ir, _) = f.finish();
    let signature = JavaMethod {
        owner: "Fixture".into(),
        name: "unused".into(),
        is_static: false,
        return_type: JavaType::Void,
        parameters: vec![JavaType::Long, JavaType::Int],
        throws: vec![],
    };
    let result = emit_method_declaration(
        &ir,
        &signature,
        &DeclarationBindings {
            receiver: None,
            parameters: vec![None, None],
            names: vec![],
        },
        &SourceResolver,
    );
    let SourceResult::Complete {
        source,
        parameter_names,
        ..
    } = result
    else {
        panic!("{result:?}")
    };
    assert_eq!(parameter_names, ["p0", "p1"]);
    assert!(source.contains("void unused(long p0, int p1)"));
    assert!(!source.contains("v0"));
}

#[test]
fn genuine_cfg_initials_are_bindings_not_hoisted_redefinitions() {
    use backend_java::{JavaMethod, JavaType, ParameterBindings, SourceResult};
    let (mut f, _) = diamond_fixture(false, false);
    for value in f.bindings.clone() {
        let id = f.values[value].def();
        f.insns[id] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![value],
            effects: managed_ir::effects::EffectFlags::NONE,
        };
        f.cfg.split_block(f.cfg.entry(), id).unwrap();
    }
    let (ir, parameters) = f.finish();
    let m = JavaMethod {
        owner: "Fixture".into(),
        name: "choose".into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![JavaType::Boolean, JavaType::Boolean, JavaType::Int],
        throws: vec![],
    };
    let result = backend_java::emit_method(
        &ir,
        &m,
        &ParameterBindings {
            parameters,
            ..Default::default()
        },
        &SourceResolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(source.contains("if (p0)"));
    assert_eq!(source.matches("boolean p0").count(), 1);
    assert!(!source.contains("boolean v"));
}

#[test]
fn consecutive_phis_use_cluster_head_edges_and_one_copy_batch() {
    let (mut f, _) = diamond_fixture(false, false);
    let join = f
        .cfg
        .block_ids()
        .find(|b| {
            matches!(
                f.insns[*f.cfg.block(*b).unwrap().payload()],
                ManagedInsn::Phi { .. }
            )
        })
        .unwrap();
    let ManagedInsn::Phi {
        inputs: phi_inputs, ..
    } = f.insns[*f.cfg.block(join).unwrap().payload()].clone()
    else {
        unreachable!()
    };
    let outgoing = f.cfg.block(join).unwrap().outgoing()[0];
    let tail = f.cfg.edge(outgoing).unwrap().target();
    let extra_id = f.insns.insert(ManagedInsn::Return { value: None });
    let extra = f.values.insert(Value::new(extra_id, ManagedType::Int));
    f.insns[extra_id] = ManagedInsn::Phi {
        result: extra,
        inputs: phi_inputs,
    };
    f.cfg.split_block(tail, extra_id).unwrap();
    f.set(tail, ManagedInsn::Return { value: Some(extra) });
    let (ir, bindings) = f.finish();
    let p = plan(&ir, &structure(ir.cfg()).unwrap().root, &bindings).unwrap();
    let Step::If {
        then_steps,
        else_steps,
        ..
    } = &p.steps[0]
    else {
        panic!("expected if")
    };
    for arm in [then_steps, else_steps] {
        let batches: Vec<_> = arm
            .iter()
            .filter_map(|s| {
                if let Step::Copies { copies, .. } = s {
                    Some(copies)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }
    let mut values = HashMap::from([(bindings[0], 1), (bindings[1], 0), (bindings[2], 99)]);
    assert_eq!(
        execute(&ir, &p.steps, &mut values, &mut Vec::new()),
        Some(10)
    );
}

#[test]
fn copy_execution_is_simultaneous_even_for_swap() {
    let (ir, bindings, _) = diamond(false, false);
    let a = bindings[0];
    let b = bindings[1];
    let edge = ir.cfg().edge_ids().next().unwrap();
    let steps = vec![Step::Copies {
        edge,
        copies: vec![
            Copy {
                destination: a,
                source: b,
            },
            Copy {
                destination: b,
                source: a,
            },
        ],
    }];
    let mut values = HashMap::from([(a, 7), (b, 9)]);
    execute(&ir, &steps, &mut values, &mut Vec::new());
    assert_eq!((values[&a], values[&b]), (9, 7));
}

#[test]
fn malformed_region_binding_and_unavailable_phi_fail_closed() {
    let (ir, bindings, _) = diamond(false, false);
    assert_eq!(
        plan(&ir, &Region::Sequence(vec![]), &bindings),
        Err(PlanError::RegionMismatch)
    );
    let region = structure(ir.cfg()).unwrap().root;
    assert_eq!(
        plan(&ir, &region, &[]),
        Err(PlanError::Unavailable(bindings[0]))
    );
    let mut duplicate = bindings.clone();
    duplicate.push(bindings[0]);
    assert!(matches!(
        plan(&ir, &region, &duplicate),
        Err(PlanError::InvalidIr(_))
    ));
    let (mut f, _) = diamond_fixture(false, false);
    // A Phi self-source is not available on the predecessor edge.
    let phi_id = f
        .insns
        .iter()
        .find_map(|(id, i)| matches!(i, ManagedInsn::Phi { .. }).then_some(id))
        .unwrap();
    if let ManagedInsn::Phi { result, inputs } = &mut f.insns[phi_id] {
        inputs[0].1 = *result;
    }
    let (ir, bindings) = f.finish();
    assert!(matches!(
        plan(&ir, &structure(ir.cfg()).unwrap().root, &bindings),
        Err(PlanError::Unavailable(_))
    ));
}

#[test]
fn cycles_and_exception_edges_keep_typed_structure_errors() {
    let mut f = Fixture::new();
    let h = f.cfg.entry();
    let edge = f.link(h, h);
    f.set(
        h,
        ManagedInsn::Branch {
            condition: None,
            then_edge: edge,
            else_edge: None,
        },
    );
    let (ir, bindings) = f.finish();
    assert!(matches!(
        plan(&ir, &Region::Block(h), &bindings),
        Err(PlanError::Structure(StructureError::CyclicControlFlow(_)))
    ));
    let mut f = Fixture::new();
    let h = f.cfg.entry();
    let tail = f.block(ManagedInsn::Return { value: None });
    f.cfg
        .add_edge(h, tail, EdgeKind::Exception("E".into()))
        .unwrap();
    let (ir, bindings) = f.finish();
    assert!(matches!(
        plan(&ir, &Region::Block(h), &bindings),
        Err(PlanError::Structure(StructureError::UnsupportedEdge { .. }))
    ));
}

#[test]
fn join_effect_stays_after_selected_arm_and_before_return() {
    let (mut f, _) = diamond_fixture(true, false);
    let tail = f
        .cfg
        .block_ids()
        .find(|b| {
            matches!(
                f.insns[*f.cfg.block(*b).unwrap().payload()],
                ManagedInsn::Return { value: Some(_) }
            )
        })
        .unwrap();
    let id = f.insns.insert(ManagedInsn::Return { value: None });
    let v = f.values.insert(Value::new(id, ManagedType::Int));
    f.insns[id] = ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(v),
        method: MethodRef::new(30),
        args: vec![],
    };
    f.cfg.split_block(tail, id).unwrap();
    let (ir, bindings) = f.finish();
    let p = plan(&ir, &structure(ir.cfg()).unwrap().root, &bindings).unwrap();
    for c in [0, 1] {
        let mut values = HashMap::from([(bindings[0], c), (bindings[1], 1), (bindings[2], 99)]);
        let mut effects = Vec::new();
        let expected = if c == 1 { 11 } else { 20 };
        assert_eq!(
            execute(&ir, &p.steps, &mut values, &mut effects),
            Some(expected)
        );
        assert_eq!(effects, vec![expected as u32, 30]);
    }
}

#[test]
fn phi_edge_coverage_types_and_branch_polarity_are_checked() {
    for case in 0..4 {
        let (mut f, _) = diamond_fixture(false, false);
        let phi_id = f
            .insns
            .iter()
            .find_map(|(id, i)| matches!(i, ManagedInsn::Phi { .. }).then_some(id))
            .unwrap();
        if case == 3 {
            let id = *f.cfg.block(f.cfg.entry()).unwrap().payload();
            let ManagedInsn::Branch {
                then_edge,
                else_edge: Some(else_edge),
                ..
            } = &mut f.insns[id]
            else {
                unreachable!()
            };
            std::mem::swap(then_edge, else_edge);
        } else {
            let ManagedInsn::Phi { inputs, .. } = &mut f.insns[phi_id] else {
                unreachable!()
            };
            match case {
                0 => {
                    inputs.pop();
                }
                1 => {
                    inputs[0].0 = inputs[1].0;
                }
                2 => {
                    inputs[0].1 = f.bindings[0];
                }
                _ => unreachable!(),
            }
        }
        let (ir, bindings) = f.finish();
        let error = plan(&ir, &structure(ir.cfg()).unwrap().root, &bindings).unwrap_err();
        match case {
            0 | 1 => assert!(matches!(error, PlanError::InvalidPhi(_))),
            2 => assert!(matches!(error, PlanError::TypeMismatch(_))),
            3 => assert!(matches!(error, PlanError::InvalidBranch(_))),
            _ => unreachable!(),
        }
    }
}

#[test]
fn same_target_true_false_edges_still_select_distinct_phi_inputs() {
    let mut f = Fixture::new();
    let condition = f.parameter(ManagedType::Boolean);
    let a = f.parameter(ManagedType::Int);
    let b = f.parameter(ManagedType::Int);
    let join = f.block(ManagedInsn::Return { value: None });
    let v = f.value(join);
    let h = f.cfg.entry();
    f.branch(h, condition, join, join);
    let ManagedInsn::Branch {
        then_edge,
        else_edge: Some(else_edge),
        ..
    } = f.insns[*f.cfg.block(h).unwrap().payload()]
    else {
        unreachable!()
    };
    f.set(
        join,
        ManagedInsn::Phi {
            result: v,
            inputs: vec![(then_edge, a), (else_edge, b)],
        },
    );
    let tail = f.block(ManagedInsn::Return { value: Some(v) });
    f.link(join, tail);
    let (ir, bindings) = f.finish();
    let p = plan(&ir, &structure(ir.cfg()).unwrap().root, &bindings).unwrap();
    for c in [0, 1] {
        let mut values = HashMap::from([(condition, c), (a, 7), (b, 9)]);
        assert_eq!(
            execute(&ir, &p.steps, &mut values, &mut Vec::new()),
            Some(if c == 1 { 7 } else { 9 })
        );
    }
}

#[test]
fn arm_local_cannot_escape_without_phi() {
    let (mut f, _) = diamond_fixture(false, false);
    let arm_value = f
        .insns
        .iter()
        .find_map(|(_, i)| {
            if let ManagedInsn::Invoke {
                result: Some(v),
                method,
                ..
            } = i
            {
                (method.index() == 10).then_some(*v)
            } else {
                None
            }
        })
        .unwrap();
    let tail = f
        .cfg
        .block_ids()
        .find(|b| {
            matches!(
                f.insns[*f.cfg.block(*b).unwrap().payload()],
                ManagedInsn::Return { value: Some(_) }
            )
        })
        .unwrap();
    f.set(
        tail,
        ManagedInsn::Return {
            value: Some(arm_value),
        },
    );
    let (ir, bindings) = f.finish();
    assert_eq!(
        plan(&ir, &structure(ir.cfg()).unwrap().root, &bindings),
        Err(PlanError::Unavailable(arm_value))
    );
}

#[test]
fn terminating_arm_does_not_remove_surviving_arm_definitions() {
    for terminate_true in [false, true] {
        let mut f = Fixture::new();
        let condition = f.parameter(ManagedType::Boolean);
        let fallback = f.parameter(ManagedType::Int);
        let returning = f.block(ManagedInsn::Return {
            value: Some(fallback),
        });
        let surviving = f.block(ManagedInsn::Return { value: None });
        let value = f.value(surviving);
        f.set(
            surviving,
            ManagedInsn::Const {
                result: value,
                value: Constant::Int(42),
            },
        );
        let tail = f.block(ManagedInsn::Return { value: Some(value) });
        f.link(surviving, tail);
        let header = f.cfg.entry();
        let (a, b) = if terminate_true {
            (returning, surviving)
        } else {
            (surviving, returning)
        };
        f.branch(header, condition, a, b);
        let (ir, bindings) = f.finish();
        let p = plan(&ir, &structure(ir.cfg()).unwrap().root, &bindings).unwrap();
        for c in [0, 1] {
            let mut values = HashMap::from([(condition, c), (fallback, 7)]);
            assert_eq!(
                execute(&ir, &p.steps, &mut values, &mut Vec::new()),
                Some(if (c == 1) == terminate_true { 7 } else { 42 })
            );
        }
    }
}

#[test]
fn single_copy_edges_do_not_allocate_temporaries_colliding_with_unused_parameters() {
    use backend_java::{DeclarationBindings, JavaMethod, JavaType, SourceResult};
    let (ir, bindings, _) = diamond(false, false);
    let result = backend_java::emit_method_declaration(
        &ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "choose".into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![
                JavaType::Boolean,
                JavaType::Boolean,
                JavaType::Int,
                JavaType::Int,
            ],
            throws: vec![],
        },
        &DeclarationBindings {
            receiver: None,
            parameters: bindings.into_iter().map(Some).chain([None]).collect(),
            names: vec!["p0".into(), "p1".into(), "p2".into(), "edge3".into()],
        },
        &SourceResolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(source.contains("int edge3)"));
    assert!(!source.contains("int edge3 ="), "{source}");
    assert!(!source.contains("int edge4 ="), "{source}");
}
