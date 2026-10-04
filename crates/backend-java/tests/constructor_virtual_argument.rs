//! Virtual calls may be constructor argument leaves, never loose prefix effects.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};
struct R {
    checked: bool,
    bad_static: bool,
}
impl JavaResolver for R {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(if c.index() == 0 { "Fixture" } else { "Helper" }.into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(if self.bad_static { 9 } else { 1 })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() < 2 {
            return self.initializer(m).map(|i| i.method);
        }
        Some(JavaMethod {
            owner: "Helper".into(),
            name: if m.index() == 2 { "first" } else { "second" }.into(),
            parameters: vec![],
            return_type: JavaType::Int,
            is_static: self.bad_static,
            throws: if self.checked {
                vec!["java.io.IOException".into()]
            } else {
                vec![]
            },
        })
    }
}
impl ConstructorResolver for R {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: "Fixture".into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 {
                    vec![JavaType::Class("Helper".into())]
                } else {
                    vec![JavaType::Int; 2]
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: if m.index() == 0 {
                Some(MethodRef::new(1))
            } else {
                None
            },
        })
    }
}
fn define(
    insns: &mut InsnArena,
    values: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> ValueId {
    let mut value = None;
    insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ty));
        value = Some(v);
        f(v)
    });
    value.unwrap()
}
fn body(mode: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let this = define(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
        |v| ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        },
    );
    let p = define(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(1)),
        |v| ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        },
    );
    let mut first = None;
    let firstid = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Int));
        first = Some(v);
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: if mode == 4 { None } else { Some(v) },
            method: MethodRef::new(2),
            args: vec![if mode == 2 { this } else { p }],
        }
    });
    let mut second = None;
    let secondid = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Int));
        second = Some(v);
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: Some(v),
            method: MethodRef::new(3),
            args: vec![p],
        }
    });
    let a = first.unwrap();
    let b = second.unwrap();
    if mode == 3 {
        insns.insert(ManagedInsn::Opaque {
            family_tag: 99,
            reads: vec![a],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    let chain = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: if mode == 1 {
            vec![this, b, a]
        } else {
            vec![this, a, b]
        },
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(firstid);
    let mut prev = cfg.entry();
    for id in [secondid, chain, ret] {
        let n = cfg.add_block(id);
        cfg.add_edge(prev, n, EdgeKind::Unconditional).unwrap();
        prev = n;
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![p],
            names: vec![],
        },
    )
}
#[test]
fn virtual_leaf_effects_dispatch_and_null_check_follow_original_order() {
    let (ir, b) = body(0);
    let plan = plan_constructor(
        &ir,
        MethodRef::new(0),
        "java.lang.Object",
        &b,
        &R {
            checked: false,
            bad_static: false,
        },
    )
    .unwrap();
    let source = format!(
        "class Trace {{static String log=\"\";}}class Helper {{int first(){{Trace.log+=\"A\";return 4;}}int second(){{Trace.log+=\"B\";return 5;}}}}class Child extends Helper {{int first(){{Trace.log+=\"D\";return 4;}}}}public class Fixture {{{}Fixture(int a,int b){{Trace.log+=\"T\";if(a!=4||b!=5)throw new AssertionError();}}public static void main(String[]a){{new Fixture(new Helper());if(!Trace.log.equals(\"ABT\"))throw new AssertionError(Trace.log);Trace.log=\"\";new Fixture(new Child());if(!Trace.log.equals(\"DBT\"))throw new AssertionError(Trace.log);Trace.log=\"\";try{{new Fixture((Helper)null);throw new AssertionError();}}catch(NullPointerException expected){{}}if(!Trace.log.isEmpty())throw new AssertionError(Trace.log);}}}}",
        plan.source
    );
    let dir = std::env::temp_dir().join(format!(
        "constructor-virtual-argument-{}",
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
#[test]
fn loose_reordered_escaping_or_unproved_calls_reject() {
    for mode in 1..=4 {
        let (ir, b) = body(mode);
        assert!(
            plan_constructor(
                &ir,
                MethodRef::new(0),
                "java.lang.Object",
                &b,
                &R {
                    checked: false,
                    bad_static: false
                }
            )
            .is_err(),
            "accepted mode{mode}"
        );
    }
    let (ir, b) = body(0);
    for r in [
        R {
            checked: true,
            bad_static: false,
        },
        R {
            checked: false,
            bad_static: true,
        },
    ] {
        assert!(plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &r).is_err());
    }
}
