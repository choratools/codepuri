//! Definite-assignment evidence must inspect branches and failed stores/invokes.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::*;
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};

struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.initializer(r).map(|i| i.method)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: "number".into(),
            ty: JavaType::Int,
            is_static: false,
        })
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ConstructorResolver for Resolver {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 2 { "Base" } else { "Fixture" }.into(),
                name: "<init>".into(),
                parameters: vec![JavaType::Int],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: (r.index() == 1).then_some(MethodRef::new(0)),
        })
    }
}
impl TrustedConstructorResolver for Resolver {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        Some(ConstructorClass {
            name: name.into(),
            superclass: match name {
                "Fixture" => Some("Base".into()),
                "Base" => Some("java.lang.Object".into()),
                "java.lang.Object" => None,
                _ => return None,
            },
            access_flags: 1,
        })
    }
    fn constructor_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
    fn declared_constructors(&self, _: &str) -> Option<Vec<MethodRef>> {
        Some(vec![MethodRef::new(0)])
    }
}
fn parameter(insns: &mut InsnArena, values: &mut ValueArena, ty: ManagedType) -> ValueId {
    let mut value = None;
    insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ty));
        value = Some(v);
        ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        }
    });
    value.unwrap()
}
fn invocation(receiver: ValueId, p: ValueId, method: u32) -> ManagedInsn {
    ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(method),
        args: vec![receiver, p],
        result: None,
    }
}
fn store(receiver: ValueId, p: ValueId) -> ManagedInsn {
    ManagedInsn::FieldPut {
        field: FieldRef::new(0),
        object: Some(receiver),
        value: p,
    }
}
fn empty() -> ManagedInsn {
    ManagedInsn::Opaque {
        family_tag: 0,
        reads: vec![],
        writes: vec![],
        effects: EffectFlags::NONE,
    }
}

#[test]
fn branch_assignment_counts_and_missing_path_are_not_collapsed_to_first_successor() {
    for missing in [false, true] {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let receiver = parameter(
            &mut insns,
            &mut values,
            ManagedType::Reference(ClassRef::new(0)),
        );
        let p = parameter(&mut insns, &mut values, ManagedType::Int);
        let init = insns.insert(invocation(receiver, p, 2));
        let branch = insns.insert(empty());
        let yes = insns.insert(store(receiver, p));
        let no = insns.insert(if missing { empty() } else { store(receiver, p) });
        let ret = insns.insert(ManagedInsn::Return { value: None });
        let mut cfg = Cfg::new(init);
        let b = cfg.add_block(branch);
        let y = cfg.add_block(yes);
        let n = cfg.add_block(no);
        let r = cfg.add_block(ret);
        cfg.add_edge(cfg.entry(), b, EdgeKind::Unconditional)
            .unwrap();
        let then_edge = cfg.add_edge(b, y, EdgeKind::BranchTrue).unwrap();
        let else_edge = cfg.add_edge(b, n, EdgeKind::BranchFalse).unwrap();
        insns[branch] = ManagedInsn::Branch {
            condition: Some(p),
            then_edge,
            else_edge: Some(else_edge),
        };
        cfg.add_edge(y, r, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(n, r, EdgeKind::Unconditional).unwrap();
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let bindings = ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![p],
            names: vec![],
        };
        let input = ConstructorInput {
            reference: MethodRef::new(0),
            ir: &ir,
            bindings: &bindings,
        };
        let evidence =
            constructor_initialization_evidence(&input, "Base", &[FieldRef::new(0)], &Resolver)
                .unwrap();
        assert_eq!(
            evidence.fields[0].normal_return_counts,
            if missing { [0, 1].into() } else { [1].into() }
        );
        assert_eq!(
            evidence.fields[0].after_chain.len(),
            if missing { 1 } else { 2 }
        );
        assert!(
            plan_constructor_family(
                "Fixture",
                &[input],
                &[BlankFinal {
                    reference: FieldRef::new(0),
                    writes: [(MethodRef::new(0), if missing { 1 } else { 2 })].into()
                }],
                &Resolver
            )
            .is_err(),
            "local evidence is not source lowering support"
        );
    }
}

#[test]
fn failed_store_catch_does_not_inherit_successful_assignment() {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let receiver = parameter(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let p = parameter(&mut insns, &mut values, ManagedType::Int);
    let init = insns.insert(invocation(receiver, p, 2));
    let write = insns.insert(store(receiver, p));
    let normal = insns.insert(ManagedInsn::Return { value: None });
    let catch = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(init);
    let w = cfg.add_block(write);
    let n = cfg.add_block(normal);
    let c = cfg.add_block(catch);
    cfg.add_edge(cfg.entry(), w, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(w, n, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(w, c, EdgeKind::Exception("java.lang.Throwable".into()))
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![p],
        names: vec![],
    };
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &bindings,
    };
    let evidence =
        constructor_initialization_evidence(&input, "Base", &[FieldRef::new(0)], &Resolver)
            .unwrap();
    assert!(evidence.has_exception_paths);
    assert_eq!(evidence.fields[0].normal_return_counts, [0, 1].into());
}

#[test]
fn failed_super_and_pre_super_writes_remain_distinct() {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let receiver = parameter(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let p = parameter(&mut insns, &mut values, ManagedType::Int);
    let pre = insns.insert(store(receiver, p));
    let init = insns.insert(invocation(receiver, p, 2));
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let catch = insns.insert(store(receiver, p));
    let thrown = insns.insert(ManagedInsn::Throw { value: receiver });
    let mut cfg = Cfg::new(pre);
    let i = cfg.add_block(init);
    let r = cfg.add_block(ret);
    let c = cfg.add_block(catch);
    let t = cfg.add_block(thrown);
    cfg.add_edge(cfg.entry(), i, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(i, r, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(i, c, EdgeKind::Exception("java.lang.Throwable".into()))
        .unwrap();
    cfg.add_edge(c, t, EdgeKind::Unconditional).unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![p],
        names: vec![],
    };
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &bindings,
    };
    let evidence =
        constructor_initialization_evidence(&input, "Base", &[FieldRef::new(0)], &Resolver)
            .unwrap();
    assert_eq!(evidence.fields[0].before_chain, [pre, catch].into());
    assert!(evidence.fields[0].after_chain.is_empty());
    assert_eq!(evidence.fields[0].normal_return_counts, [1].into());
    assert!(
        plan_constructor_family(
            "Fixture",
            &[input],
            &[BlankFinal {
                reference: FieldRef::new(0),
                writes: [(MethodRef::new(0), 2)].into()
            }],
            &Resolver
        )
        .is_err()
    );
}

#[test]
fn cycles_saturate_counts_and_double_writes_are_not_one() {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let receiver = parameter(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let p = parameter(&mut insns, &mut values, ManagedType::Int);
    let init = insns.insert(invocation(receiver, p, 2));
    let write = insns.insert(store(receiver, p));
    let branch = insns.insert(empty());
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(init);
    let w = cfg.add_block(write);
    let b = cfg.add_block(branch);
    let r = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), w, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(w, b, EdgeKind::Unconditional).unwrap();
    let then_edge = cfg.add_edge(b, w, EdgeKind::BranchTrue).unwrap();
    let else_edge = cfg.add_edge(b, r, EdgeKind::BranchFalse).unwrap();
    insns[branch] = ManagedInsn::Branch {
        condition: Some(p),
        then_edge,
        else_edge: Some(else_edge),
    };
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![p],
        names: vec![],
    };
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &bindings,
    };
    let evidence =
        constructor_initialization_evidence(&input, "Base", &[FieldRef::new(0)], &Resolver)
            .unwrap();
    assert_eq!(evidence.fields[0].normal_return_counts, [1, 2].into());
}

struct NullResolver;
impl JavaResolver for NullResolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.initializer(r).map(|m| m.method)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 1 {
                "java.lang.String"
            } else {
                "Fixture"
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ConstructorResolver for NullResolver {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 0 { "Fixture" } else { "Base" }.into(),
                name: "<init>".into(),
                is_static: false,
                parameters: if r.index() == 0 {
                    vec![]
                } else {
                    vec![JavaType::Class("java.lang.String".into())]
                },
                return_type: JavaType::Void,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}

#[test]
fn typed_null_pins_exact_parent_constructor_overload() {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let receiver = parameter(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let mut value = None;
    let constant = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(1))));
        value = Some(result);
        ManagedInsn::Const {
            result,
            value: managed_ir::insn::Constant::Null,
        }
    });
    let invoke = insns.insert(invocation(receiver, value.unwrap(), 1));
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(constant);
    let i = cfg.add_block(invoke);
    let r = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), i, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(i, r, EdgeKind::Unconditional).unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let plan = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Base",
        &ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![],
            names: vec![],
        },
        &NullResolver,
    )
    .unwrap();
    // An uncast null is ambiguous between these unrelated overloads; the DEX
    // operand names the String constructor exactly.
    let source = format!(
        "class Base {{ final int selected; Base(String s) {{selected=1;}} Base(StringBuilder s) {{selected=2;}} }} public class Fixture extends Base {{ {} public static void main(String[] a) {{if(new Fixture().selected!=1)throw new AssertionError();}} }}",
        plan.source
    );
    let dir = std::env::temp_dir()
        .join("opencode")
        .join(format!("constructor-null-target-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (binary, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = std::process::Command::new(format!("{home}/bin/{binary}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{binary}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
