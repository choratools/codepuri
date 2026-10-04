// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]

use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    value::Value,
};

struct Resolver {
    this: bool,
    cycle: bool,
}
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.initializer(r).map(|x| x.method)
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
        if r.index() > 1 {
            return None;
        }
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 0 || self.this {
                    "Fixture"
                } else {
                    "Base"
                }
                .into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Int, JavaType::Int],
                throws: vec![],
            },
            delegates_to: if r.index() == 0 && self.this {
                Some(MethodRef::new(1))
            } else if r.index() == 1 && self.cycle {
                Some(MethodRef::new(0))
            } else {
                None
            },
        })
    }
}
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
    fn push(&mut self, insn: ManagedInsn) {
        self.order.push(self.insns.insert(insn));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
fn fixture(pattern: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = Builder::default();
    let receiver = b.parameter();
    let p = b.parameter();
    let q = b.parameter();
    let alias = b.value(ManagedType::Reference(ClassRef::new(0)), |v| {
        ManagedInsn::Move {
            result: v,
            value: receiver,
        }
    });
    if pattern == 1 {
        b.push(ManagedInsn::FieldPut {
            object: Some(alias),
            field: FieldRef::new(0),
            value: p,
        });
    }
    if pattern != 2 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(if pattern == 3 { 9 } else { 1 }),
            args: vec![alias, q, p],
            result: None,
        });
    }
    if pattern == 4 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(1),
            args: vec![alias, q, p],
            result: None,
        });
    }
    b.push(ManagedInsn::FieldPut {
        object: Some(alias),
        field: FieldRef::new(0),
        value: p,
    });
    b.push(ManagedInsn::Return { value: None });
    (
        b.finish(),
        ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![p, q],
            names: vec![],
        },
    )
}
#[test]
fn exact_super_and_this_arguments_and_body_order() {
    for this in [false, true] {
        let (ir, bindings) = fixture(0);
        let plan = plan_constructor(
            &ir,
            MethodRef::new(0),
            "Base",
            &bindings,
            &Resolver { this, cycle: false },
        )
        .unwrap();
        let call = if this { "this" } else { "super" };
        assert_eq!(
            plan.source,
            format!(
                "public Fixture(int p0, int p1) {{\n    {call}(p1, p0);\n    this.number = p0;\n    return;\n}}\n"
            )
        );
        assert_eq!(plan.body, ["this.number = p0;", "return;"]);
        assert_eq!(
            plan.chain,
            if this {
                ChainKind::This
            } else {
                ChainKind::Super
            }
        );
    }
}

#[test]
fn unused_constructor_descriptor_position_preserves_arity_and_executes() {
    struct Unused(Resolver);
    impl JavaResolver for Unused {
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            self.initializer(r).map(|m| m.method)
        }
        fn field(&self, r: FieldRef) -> Option<JavaField> {
            self.0.field(r)
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            self.0.class(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            self.0.string(r)
        }
    }
    impl ConstructorResolver for Unused {
        fn initializer(&self, r: MethodRef) -> Option<Initializer> {
            let mut m = self.0.initializer(r)?;
            if r.index() == 0 {
                m.method.parameters.push(JavaType::Long);
            }
            Some(m)
        }
    }
    let (ir, bindings) = fixture(0);
    let declaration = DeclarationBindings {
        receiver: bindings.receiver,
        parameters: vec![
            Some(bindings.parameters[0]),
            Some(bindings.parameters[1]),
            None,
        ],
        names: vec![],
    };
    let resolver = Unused(Resolver {
        this: false,
        cycle: false,
    });
    let plan =
        plan_constructor_declaration(&ir, MethodRef::new(0), "Base", &declaration, &resolver)
            .unwrap();
    assert!(plan.source.contains("Fixture(int p0, int p1, long p2)"));
    assert!(plan.source.contains("super(p1, p0);"));
    let root = std::env::temp_dir().join(format!(
        "unused-original-constructor-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("Fixture.java");
    std::fs::write(&path, format!("class Base {{ int seen; Base(int x, int y) {{ seen = x * 10 + y; }} }}\npublic class Fixture extends Base {{ int number; {} public static void main(String[] args) {{ Fixture f = new Fixture(3, 7, 99L); if (f.number != 3 || f.seen != 73) throw new AssertionError(); }} }}", plan.source)).unwrap();
    let compile = std::process::Command::new("javac")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(
        std::process::Command::new("java")
            .args(["-cp", root.to_str().unwrap(), "Fixture"])
            .status()
            .unwrap()
            .success()
    );
    let mut wrong_arity = declaration;
    wrong_arity.parameters.pop();
    assert!(
        plan_constructor_declaration(&ir, MethodRef::new(0), "Base", &wrong_arity, &resolver)
            .is_err()
    );
}
#[test]
fn rejects_preinit_effect_missing_mapping_missing_chain_and_double_init() {
    for pattern in 1..=4 {
        let (ir, bindings) = fixture(pattern);
        assert!(
            plan_constructor(
                &ir,
                MethodRef::new(0),
                "Base",
                &bindings,
                &Resolver {
                    this: false,
                    cycle: false
                }
            )
            .is_err(),
            "pattern {pattern}"
        );
    }
}
#[test]
fn rejects_delegation_cycle() {
    let (ir, bindings) = fixture(0);
    assert!(matches!(
        plan_constructor(
            &ir,
            MethodRef::new(0),
            "Base",
            &bindings,
            &Resolver {
                this: true,
                cycle: true
            }
        ),
        Err(Unsupported::ControlFlow(_))
    ));
}
#[test]
fn rejects_cfg_cycle() {
    let mut b = Builder::default();
    let receiver = b.parameter();
    let p = b.parameter();
    let q = b.parameter();
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(1),
        args: vec![receiver, p, q],
        result: None,
    });
    let mut cfg = Cfg::new(b.order[0]);
    cfg.add_edge(cfg.entry(), cfg.entry(), EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![p, q],
        names: vec![],
    };
    assert!(matches!(
        plan_constructor(
            &ir,
            MethodRef::new(0),
            "Base",
            &bindings,
            &Resolver {
                this: false,
                cycle: false
            }
        ),
        Err(Unsupported::ControlFlow(_))
    ));
}

#[test]
fn primitive_constant_is_in_chain_without_a_leading_local() {
    let mut b = Builder::default();
    let receiver = b.parameter();
    let p = b.parameter();
    let q = b.parameter();
    let c = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(17),
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(1),
        args: vec![receiver, c, p],
        result: None,
    });
    b.push(ManagedInsn::Return { value: None });
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![p, q],
        names: vec![],
    };
    let plan = plan_constructor(
        &b.finish(),
        MethodRef::new(0),
        "Base",
        &bindings,
        &Resolver {
            this: false,
            cycle: false,
        },
    )
    .unwrap();
    assert_eq!(plan.first_statement, "super(17, p0);");
    assert_eq!(
        plan.source,
        "public Fixture(int p0, int p1) {\n    super(17, p0);\n    return;\n}\n"
    );
}

#[test]
fn rejects_uninitialized_this_as_argument_and_non_special_dispatch() {
    for kind in [InvokeKind::Special, InvokeKind::Virtual] {
        let mut b = Builder::default();
        let receiver = b.parameter();
        let p = b.parameter();
        let q = b.parameter();
        b.push(ManagedInsn::Invoke {
            kind,
            method: MethodRef::new(1),
            args: vec![receiver, receiver, p],
            result: None,
        });
        b.push(ManagedInsn::Return { value: None });
        let bindings = ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![p, q],
            names: vec![],
        };
        assert!(
            plan_constructor(
                &b.finish(),
                MethodRef::new(0),
                "Base",
                &bindings,
                &Resolver {
                    this: false,
                    cycle: false
                }
            )
            .is_err()
        );
    }
}
