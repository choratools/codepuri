//! Pre-chain argument allocation order and escape evidence.
#![allow(clippy::unwrap_used)]
use backend_java::{JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, MethodRef},
    types::ManagedType,
    value::{Value, ValueId},
};
struct Resolver {
    mapping: bool,
}
impl JavaResolver for Resolver {
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        Some("literal".encode_utf16().collect())
    }
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "Fixture",
                1 => "Arg",
                2 => "java.lang.String",
                _ => return None,
            }
            .into(),
        )
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        self.initializer(m).map(|i| i.method)
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
}
impl ConstructorResolver for Resolver {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: match m.index() {
                    0..=2 => "Fixture",
                    3 | 6 | 7 | 8 => "Base",
                    4 => "Arg",
                    _ => return None,
                }
                .into(),
                name: "<init>".into(),
                parameters: if m.index() == 8 {
                    vec![JavaType::Class("java.lang.String".into()); 2]
                } else if m.index() == 7 {
                    vec![
                        JavaType::Class("java.lang.String".into()),
                        JavaType::Class("Arg".into()),
                    ]
                } else if m.index() == 6 {
                    vec![JavaType::Class("Arg".into()); 2]
                } else if matches!(m.index(), 1 | 3) {
                    vec![JavaType::Class("Arg".into())]
                } else {
                    vec![]
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: (m.index() == 0 && self.mapping).then_some(MethodRef::new(1)),
        })
    }
}
fn parameter(insns: &mut InsnArena, values: &mut ValueArena, class: u32) -> ValueId {
    let mut value = None;
    insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(class))));
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
fn body(escape: bool) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let receiver = parameter(&mut insns, &mut values, 0);
    let mut object = None;
    let allocation = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(1))));
        object = Some(v);
        ManagedInsn::NewInstance {
            result: v,
            class: ClassRef::new(1),
        }
    });
    let object = object.unwrap();
    let init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(4),
        args: vec![object],
        result: None,
    });
    let chain = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(1),
        args: vec![receiver, object],
        result: None,
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    if escape {
        insns.insert(ManagedInsn::Opaque {
            family_tag: 99,
            reads: vec![object],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    let mut cfg = Cfg::new(allocation);
    let mut previous = cfg.entry();
    for instruction in [init, chain, ret] {
        let block = cfg.add_block(instruction);
        cfg.add_edge(previous, block, EdgeKind::Unconditional)
            .unwrap();
        previous = block;
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![],
            names: vec![],
        },
    )
}
#[test]
fn allocated_argument_preserves_initialization_throw_and_single_evaluation() {
    let (ir, bindings) = body(false);
    let plan = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Base",
        &bindings,
        &Resolver { mapping: true },
    )
    .unwrap();
    assert_eq!(plan.first_statement, "this(new Arg());");
    let source = format!(
        "class Trace {{static String log=\"\";static boolean fail;}} class Arg {{static {{Trace.log+=\"C\";}} public Arg(){{Trace.log+=\"A\";if(Trace.fail)throw new IllegalArgumentException();}}}} class Base {{Base(Arg arg){{Trace.log+=\"B\";}}}} public class Fixture extends Base {{ {} public Fixture(Arg arg){{super(arg);}} public static void main(String[]a){{new Fixture();if(!Trace.log.equals(\"CAB\"))throw new AssertionError(Trace.log);Trace.log=\"\";Trace.fail=true;try{{new Fixture();throw new AssertionError();}}catch(IllegalArgumentException expected){{}}if(!Trace.log.equals(\"A\"))throw new AssertionError(Trace.log);}}}}",
        plan.source
    );
    let dir = std::env::temp_dir().join(format!("constructor-argument-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
fn unreachable_extra_read_prevents_argument_duplication_or_escape() {
    let (ir, bindings) = body(true);
    assert!(
        plan_constructor(
            &ir,
            MethodRef::new(0),
            "Base",
            &bindings,
            &Resolver { mapping: true }
        )
        .is_err()
    );
}

#[test]
fn two_allocation_arguments_require_original_left_to_right_order_without_duplication() {
    for mode in 0..3 {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let receiver = parameter(&mut insns, &mut values, 0);
        let mut sequence = Vec::new();
        let mut objects = Vec::new();
        for _ in 0..2 {
            let mut object = None;
            let allocation = insns.insert_with_key(|id| {
                let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(1))));
                object = Some(value);
                ManagedInsn::NewInstance {
                    result: value,
                    class: ClassRef::new(1),
                }
            });
            let value = object.unwrap();
            objects.push(value);
            sequence.push(allocation);
            sequence.push(insns.insert(ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                method: MethodRef::new(4),
                args: vec![value],
                result: None,
            }));
        }
        let arguments = match mode {
            0 => vec![receiver, objects[0], objects[1]],
            1 => vec![receiver, objects[1], objects[0]],
            _ => vec![receiver, objects[0], objects[0]],
        };
        sequence.push(insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(6),
            args: arguments,
            result: None,
        }));
        sequence.push(insns.insert(ManagedInsn::Return { value: None }));
        let mut cfg = Cfg::new(sequence[0]);
        let mut last = cfg.entry();
        for instruction in &sequence[1..] {
            let next = cfg.add_block(*instruction);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let bindings = ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![],
            names: vec![],
        };
        let plan = plan_constructor(
            &ir,
            MethodRef::new(2),
            "Base",
            &bindings,
            &Resolver { mapping: true },
        );
        assert_eq!(plan.is_ok(), mode == 0, "mode {mode}: {plan:?}");
    }
}

#[test]
fn constructed_argument_does_not_invent_missing_this_delegation_metadata() {
    let (ir, bindings) = body(false);
    let error = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Base",
        &bindings,
        &Resolver { mapping: false },
    )
    .unwrap_err();
    assert!(
        matches!(error, backend_java::Unsupported::Metadata(ref reason)
        if reason == "this delegation mapping mismatch")
    );
}

#[test]
fn literal_effect_trace_accepts_matching_argument_order_and_rejects_original_reversal() {
    for literal_first in [true, false] {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let receiver = parameter(&mut insns, &mut values, 0);
        let mut literal = None;
        let constant = insns.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(2))));
            literal = Some(value);
            ManagedInsn::Const {
                result: value,
                value: managed_ir::insn::Constant::String(managed_ir::refs::StringRef::new(0)),
            }
        });
        let mut object = None;
        let allocation = insns.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(1))));
            object = Some(value);
            ManagedInsn::NewInstance {
                result: value,
                class: ClassRef::new(1),
            }
        });
        let object = object.unwrap();
        let constant_value = literal.unwrap();
        let init = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(4),
            args: vec![object],
            result: None,
        });
        let chain = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(7),
            args: vec![receiver, constant_value, object],
            result: None,
        });
        let ret = insns.insert(ManagedInsn::Return { value: None });
        let order = if literal_first {
            vec![constant, allocation, init, chain, ret]
        } else {
            vec![allocation, init, constant, chain, ret]
        };
        let mut cfg = Cfg::new(order[0]);
        let mut previous = cfg.entry();
        for id in &order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(previous, next, EdgeKind::Unconditional)
                .unwrap();
            previous = next;
        }
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let bindings = ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![],
            names: vec![],
        };
        let plan = plan_constructor(
            &ir,
            MethodRef::new(2),
            "Base",
            &bindings,
            &Resolver { mapping: true },
        );
        assert_eq!(plan.is_ok(), literal_first, "{plan:?}");
        if let Ok(plan) = plan {
            assert!(
                plan.first_statement.find("literal").unwrap()
                    < plan.first_statement.find("new Arg").unwrap()
            );
        }
    }
}

#[test]
fn original_single_native_literal_can_reuse_its_canonical_identity() {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let receiver = parameter(&mut insns, &mut values, 0);
    let mut literal = None;
    let constant = insns.insert_with_key(|id| {
        let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(2))));
        literal = Some(value);
        ManagedInsn::Const {
            result: value,
            value: managed_ir::insn::Constant::String(managed_ir::refs::StringRef::new(0)),
        }
    });
    let literal = literal.unwrap();
    let chain = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(8),
        args: vec![receiver, literal, literal],
        result: None,
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(constant);
    let c = cfg.add_block(chain);
    let r = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), c, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(c, r, EdgeKind::Unconditional).unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![],
        names: vec![],
    };
    let plan = plan_constructor(
        &ir,
        MethodRef::new(2),
        "Base",
        &bindings,
        &Resolver { mapping: true },
    )
    .unwrap();
    assert!(!plan.first_statement.contains("new java.lang.String"));
    assert_eq!(plan.first_statement.matches("literal").count(), 2);
    let source = format!(
        "class Base {{Base(String a,String b){{if(a!=b||a!=\"literal\")throw new AssertionError();}}}} public class Fixture extends Base {{{}public static void main(String[]a){{new Fixture();}}}}",
        plan.source
    );
    let dir = std::env::temp_dir().join(format!(
        "constructor-canonical-string-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    for (tool, args) in [
        ("javac", vec!["Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let o =
            std::process::Command::new(format!("/home/jtjisgod/tools/jdk-17.0.13+11/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}
