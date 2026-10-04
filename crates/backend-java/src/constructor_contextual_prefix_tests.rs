//! Exact constructor-prefix operands keep reference identity and proven zero context.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    value::Value,
};

#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(&mut self, ty: ManagedType, f: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut value = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        self.order.push(id);
        value.unwrap()
    }
    fn parameter(&mut self) -> ValueId {
        let v = self.value(ManagedType::Unknown, |v| ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        });
        self.order.pop();
        v
    }
    fn push(&mut self, i: ManagedInsn) {
        self.order.push(self.insns.insert(i));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for id in self.order.iter().skip(1) {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.initializer(r).map(|i| i.method)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "Fixture"
            } else {
                "java.lang.Object"
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ConstructorResolver for Resolver {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 0 { "Fixture" } else { "Base" }.into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: if r.index() == 0 {
                    vec![JavaType::Class("java.lang.Object".into())]
                } else {
                    vec![
                        JavaType::Boolean,
                        JavaType::Class("java.lang.Object".into()),
                    ]
                },
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn fixture(
    op: CompareOp,
    literal: Constant,
    zero_type: ManagedType,
) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = Builder::default();
    let receiver = b.parameter();
    let input = b.parameter();
    let zero = b.value(zero_type, |result| ManagedInsn::Const {
        result,
        value: literal,
    });
    let flag = b.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op,
        result,
        lhs: input,
        rhs: zero,
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![receiver, flag, zero],
    });
    b.push(ManagedInsn::Return { value: None });
    (
        b.finish(),
        ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![input],
            names: vec![],
        },
    )
}
#[test]
fn reference_equality_and_inferred_null_zero_prefix_preserve_descriptor() {
    for ty in [
        ManagedType::Null,
        ManagedType::Zero,
        ManagedType::Reference(ClassRef::new(1)),
    ] {
        let (ir, bindings) = fixture(CompareOp::Equal, Constant::Zero, ty);
        let plan = plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &Resolver).unwrap();
        assert!(plan.first_statement.contains("java.lang.Object"));
        assert!(plan.first_statement.contains("=="));
        assert!(plan.first_statement.contains("null"));
        assert!(plan.prologue.is_empty());
    }
}
#[test]
fn reference_ordering_numeric_literal_and_nonzero_reference_prefix_reject() {
    for (op, literal, ty) in [
        (CompareOp::LessThan, Constant::Zero, ManagedType::Null),
        (CompareOp::Equal, Constant::Int(1), ManagedType::Int),
        (
            CompareOp::Equal,
            Constant::Int(1),
            ManagedType::Reference(ClassRef::new(1)),
        ),
    ] {
        let (ir, bindings) = fixture(op, literal, ty);
        assert!(plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &Resolver).is_err());
    }
}
#[test]
fn unchanged_generated_prefix_compiles_and_preserves_parent_failure_order() {
    use std::process::Command;
    let (ir, bindings) = fixture(CompareOp::Equal, Constant::Zero, ManagedType::Null);
    let plan = plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &Resolver).unwrap();
    let root = std::env::temp_dir().join(format!("decomphify-prefix-null-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = format!(
        "class Base {{ static int calls; final boolean absent; Base(boolean absent,Object zero) {{ calls++; if(zero!=null)throw new AssertionError(); this.absent=absent; if(absent)throw new IllegalArgumentException(\"parent\"); }} }} class Fixture extends Base {{ {} public static void main(String[] args) {{ Object present=new Object(); Fixture good=new Fixture(present); if(good.absent || Base.calls!=1)throw new AssertionError(); try{{new Fixture(null);throw new AssertionError();}}catch(IllegalArgumentException expected){{if(!expected.getMessage().equals(\"parent\")||Base.calls!=2)throw new AssertionError();}} }} }}",
        plan.source
    );
    let path = root.join("Fixture.java");
    std::fs::write(&path, source).unwrap();
    let output = Command::new("javac").arg(&path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(&root)
        .arg("Fixture")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct AbruptResolver;
impl JavaResolver for AbruptResolver {
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || (actual == &JavaType::Class("java.lang.IllegalArgumentException".into())
                && matches!(expected, JavaType::Class(name) if matches!(name.as_str(), "java.lang.RuntimeException" | "java.lang.Exception" | "java.lang.Throwable" | "java.lang.Object")))
    }

    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        if reference.index() == 2 {
            return Some(JavaMethod {
                owner: "Fixture".into(),
                name: "get".into(),
                is_static: true,
                parameters: vec![JavaType::Class("java.lang.Object".into())],
                return_type: JavaType::Class("java.lang.Object".into()),
                throws: vec![],
            });
        }
        self.initializer(reference).map(|x| x.method)
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 2 { 9 } else { 1 })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "Fixture",
                2 => "java.lang.IllegalArgumentException",
                3 => "java.lang.String",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("required".encode_utf16().collect())
    }
}
impl ConstructorResolver for AbruptResolver {
    fn constructor_source_level(&self) -> u16 {
        25
    }
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: match r.index() {
                    0 => "Fixture",
                    1 => "Base",
                    3 => "java.lang.IllegalArgumentException",
                    _ => return None,
                }
                .into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Class(
                    if r.index() == 3 {
                        "java.lang.String"
                    } else {
                        "java.lang.Object"
                    }
                    .into(),
                )],
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn abrupt_fixture() -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = Builder::default();
    let receiver = b.parameter();
    let input = b.parameter();
    let zero = b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    let captured = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![input],
        }
    });
    let guard = b.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op: CompareOp::Equal,
        result,
        lhs: captured,
        rhs: zero,
    });
    let mut cfg = Cfg::new(b.order[0]);
    let mut last = cfg.entry();
    for id in b.order.iter().skip(1) {
        let n = cfg.add_block(*id);
        cfg.add_edge(last, n, EdgeKind::Unconditional).unwrap();
        last = n;
    }
    let branch = b.insns.insert(ManagedInsn::Return { value: None });
    let branch_block = cfg.add_block(branch);
    cfg.add_edge(last, branch_block, EdgeKind::Unconditional)
        .unwrap();
    b.order.clear();
    let failure = b.value(ManagedType::Reference(ClassRef::new(2)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(2),
        }
    });
    let message = b.value(ManagedType::Reference(ClassRef::new(3)), |result| {
        ManagedInsn::Const {
            result,
            value: Constant::String(StringRef::new(0)),
        }
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(3),
        args: vec![failure, message],
    });
    b.push(ManagedInsn::Throw { value: failure });
    let error = cfg.add_block(b.order[0]);
    let mut last_error = error;
    for id in b.order.iter().skip(1) {
        let n = cfg.add_block(*id);
        cfg.add_edge(last_error, n, EdgeKind::Unconditional)
            .unwrap();
        last_error = n;
    }
    let chain = cfg.add_block(b.insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![receiver, captured],
    }));
    let end = cfg.add_block(b.insns.insert(ManagedInsn::Return { value: None }));
    cfg.add_edge(chain, end, EdgeKind::Unconditional).unwrap();
    let then_edge = cfg
        .add_edge(branch_block, error, EdgeKind::BranchTrue)
        .unwrap();
    let else_edge = cfg
        .add_edge(branch_block, chain, EdgeKind::BranchFalse)
        .unwrap();
    b.insns[branch] = ManagedInsn::Branch {
        condition: Some(guard),
        then_edge,
        else_edge: Some(else_edge),
    };
    (
        MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values),
        ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![input],
            names: vec![],
        },
    )
}
#[test]
fn java25_original_abrupt_arm_preserves_getter_and_throw_before_parent() {
    use std::process::Command;
    let (ir, bindings) = abrupt_fixture();
    let plan =
        plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &AbruptResolver).unwrap();
    assert!(!plan.prologue.is_empty());
    let root =
        std::env::temp_dir().join(format!("decomphify-prefix-abrupt-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = format!(
        "class Base {{Base(Object value){{Fixture.trace+=\"P\";}}}} class Fixture extends Base {{static String trace=\"\";static RuntimeException marker=new RuntimeException();public static Object get(Object value){{trace+=\"G\";if(value==marker)throw marker;return value;}} {} public static void main(String[] args){{ new Fixture(new Object());if(!trace.equals(\"GP\"))throw new AssertionError(trace);trace=\"\";try{{new Fixture(null);throw new AssertionError();}}catch(IllegalArgumentException e){{if(!e.getMessage().equals(\"required\")||!trace.equals(\"G\"))throw new AssertionError(trace);}} trace=\"\";try{{new Fixture(marker);throw new AssertionError();}}catch(RuntimeException e){{if(e!=marker||!trace.equals(\"G\"))throw new AssertionError();}} }} }}",
        plan.source
    );
    let path = root.join("Fixture.java");
    std::fs::write(&path, source).unwrap();
    let jdk = "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin";
    let output = Command::new(format!("{jdk}/javac"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(format!("{jdk}/java"))
        .arg("-cp")
        .arg(&root)
        .arg("Fixture")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn effectful_diamond_fixture() -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = Builder::default();
    let receiver = b.parameter();
    let input = b.parameter();
    b.values[input] = Value::new(
        b.values[input].def(),
        ManagedType::Reference(ClassRef::new(1)),
    );
    let zero = b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    let guard = b.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op: CompareOp::Equal,
        result,
        lhs: input,
        rhs: zero,
    });
    let mut cfg = Cfg::new(b.order[0]);
    let guard_block = cfg.add_block(b.order[1]);
    cfg.add_edge(cfg.entry(), guard_block, EdgeKind::Unconditional)
        .unwrap();
    let branch = b.insns.insert(ManagedInsn::Return { value: None });
    let branch_block = cfg.add_block(branch);
    cfg.add_edge(guard_block, branch_block, EdgeKind::Unconditional)
        .unwrap();
    b.order.clear();
    let effect = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![input],
        }
    });
    let yes = cfg.add_block(b.order[0]);
    let copied = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Move {
            result,
            value: input,
        }
    });
    let no = cfg.add_block(b.order[1]);
    let phi = b.value(ManagedType::Reference(ClassRef::new(1)), |_| {
        ManagedInsn::Return { value: None }
    });
    let merge = cfg.add_block(b.order[2]);
    let ye = cfg.add_edge(yes, merge, EdgeKind::Unconditional).unwrap();
    let ne = cfg.add_edge(no, merge, EdgeKind::Unconditional).unwrap();
    b.insns[b.order[2]] = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(ye, effect), (ne, copied)],
    };
    let te = cfg
        .add_edge(branch_block, yes, EdgeKind::BranchTrue)
        .unwrap();
    let fe = cfg
        .add_edge(branch_block, no, EdgeKind::BranchFalse)
        .unwrap();
    b.insns[branch] = ManagedInsn::Branch {
        condition: Some(guard),
        then_edge: te,
        else_edge: Some(fe),
    };
    let chain = cfg.add_block(b.insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![receiver, phi],
    }));
    cfg.add_edge(merge, chain, EdgeKind::Unconditional).unwrap();
    let end = cfg.add_block(b.insns.insert(ManagedInsn::Return { value: None }));
    cfg.add_edge(chain, end, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values),
        ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![input],
            names: vec![],
        },
    )
}
#[test]
fn java25_effectful_phi_branch_evaluates_selected_default_once() {
    use std::process::Command;
    let (ir, bindings) = effectful_diamond_fixture();
    let plan =
        plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &AbruptResolver).unwrap();
    let root = std::env::temp_dir().join(format!(
        "decomphify-prefix-effectful-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = format!(
        "class Base {{final Object value;Base(Object v){{value=v;Fixture.trace+=\"P\";}}}} class Fixture extends Base {{static String trace=\"\";static Object fallback=new Object();static boolean fail;static RuntimeException marker=new RuntimeException();public static Object get(Object ignored){{trace+=\"D\";if(fail)throw marker;return fallback;}} {} public static void main(String[] args){{Object supplied=new Object();Fixture a=new Fixture(supplied);if(a.value!=supplied||!trace.equals(\"P\"))throw new AssertionError(trace);trace=\"\";Fixture b=new Fixture(null);if(b.value!=fallback||!trace.equals(\"DP\"))throw new AssertionError(trace);trace=\"\";fail=true;try{{new Fixture(null);throw new AssertionError();}}catch(RuntimeException e){{if(e!=marker||!trace.equals(\"D\"))throw new AssertionError(trace);}}trace=\"\";new Fixture(supplied);if(!trace.equals(\"P\"))throw new AssertionError(trace);}} }}",
        plan.source
    );
    let path = root.join("Fixture.java");
    std::fs::write(&path, source).unwrap();
    let jdk = "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin";
    let output = Command::new(format!("{jdk}/javac"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(format!("{jdk}/java"))
        .arg("-cp")
        .arg(&root)
        .arg("Fixture")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn java25_prefix_effects_require_genuine_access_metadata() {
    struct MissingAccess;
    impl JavaResolver for MissingAccess {
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            AbruptResolver.method(r)
        }
        fn field(&self, r: FieldRef) -> Option<JavaField> {
            AbruptResolver.field(r)
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            AbruptResolver.class(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            AbruptResolver.string(r)
        }
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            AbruptResolver.assignable(a, b)
        }
    }
    impl ConstructorResolver for MissingAccess {
        fn constructor_source_level(&self) -> u16 {
            25
        }
        fn initializer(&self, r: MethodRef) -> Option<Initializer> {
            AbruptResolver.initializer(r)
        }
    }
    for (ir, bindings) in [abrupt_fixture(), effectful_diamond_fixture()] {
        assert!(
            plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &MissingAccess).is_err()
        );
    }
}
