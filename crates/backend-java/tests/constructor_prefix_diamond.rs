//! Exact predecessor-selected lazy constructor arguments.
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
struct R;
impl JavaResolver for R {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(if c.index() == 0 { "Fixture" } else { "Payload" }.into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        self.initializer(m).map(|i| i.method)
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
}
impl ConstructorResolver for R {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if m.index() == 2 { "Payload" } else { "Fixture" }.into(),
                name: "<init>".into(),
                parameters: match m.index() {
                    0 => vec![JavaType::Class("Payload".into()), JavaType::Boolean],
                    1 => vec![JavaType::Class("Payload".into())],
                    _ => vec![],
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: (m.index() == 0).then_some(MethodRef::new(1)),
        })
    }
}
fn definition(
    insns: &mut InsnArena,
    values: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (managed_ir::value::InsnId, ValueId) {
    let mut v = None;
    let id = insns.insert_with_key(|id| {
        let value = values.insert(Value::new(id, ty));
        v = Some(value);
        f(value)
    });
    (id, v.unwrap())
}
fn input(insns: &mut InsnArena, values: &mut ValueArena, ty: ManagedType) -> ValueId {
    definition(insns, values, ty, |v| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![v],
        effects: EffectFlags::NONE,
    })
    .1
}
fn body(mode: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let this = input(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let original = input(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(1)),
    );
    let condition = input(&mut insns, &mut values, ManagedType::Boolean);
    let branch = insns.insert(ManagedInsn::Return { value: None });
    let (allocation, obj) = definition(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(1)),
        |v| ManagedInsn::NewInstance {
            result: v,
            class: ClassRef::new(1),
        },
    );
    let initialize = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(2),
        args: vec![if mode == 4 { this } else { obj }],
    });
    let (phi, result) = definition(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(1)),
        |v| ManagedInsn::Phi {
            result: v,
            inputs: vec![],
        },
    );
    let chain = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this, result],
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(branch);
    let b = cfg.entry();
    let a = cfg.add_block(allocation);
    let i = cfg.add_block(initialize);
    let p = cfg.add_block(phi);
    let c = cfg.add_block(chain);
    let r = cfg.add_block(ret);
    let yes = cfg.add_edge(b, p, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(b, a, EdgeKind::BranchFalse).unwrap();
    cfg.add_edge(a, i, EdgeKind::Unconditional).unwrap();
    let tail = cfg.add_edge(i, p, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(p, c, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(c, r, EdgeKind::Unconditional).unwrap();
    *insns.get_mut(branch).unwrap() = ManagedInsn::Branch {
        condition: Some(if mode == 5 { this } else { condition }),
        then_edge: yes,
        else_edge: Some(no),
    };
    *insns.get_mut(phi).unwrap() = ManagedInsn::Phi {
        result,
        inputs: vec![
            (yes, original),
            (
                if mode == 2 { yes } else { tail },
                if mode == 3 { original } else { obj },
            ),
        ],
    };
    if mode == 1 {
        insns.insert(ManagedInsn::Opaque {
            family_tag: 99,
            reads: vec![obj],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    if mode == 6 {
        cfg.add_edge(i, a, EdgeKind::Unconditional).unwrap();
    }
    if mode == 7 {
        *insns.get_mut(initialize).unwrap() = ManagedInsn::Branch {
            condition: Some(condition),
            then_edge: tail,
            else_edge: Some(tail),
        };
    }
    if mode == 8 {
        *insns.get_mut(ret).unwrap() = ManagedInsn::Return {
            value: Some(result),
        };
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![original, condition],
            names: vec![],
        },
    )
}
#[test]
fn conditional_argument_is_lazy_and_throw_precedes_chain() {
    let (ir, b) = body(0);
    let p = plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &R).unwrap();
    let source = format!(
        "class Trace {{static String log=\"\";static boolean fail;}}class Payload {{static{{Trace.log+=\"C\";}}Payload(){{Trace.log+=\"A\";if(Trace.fail)throw new IllegalArgumentException(\"payload\");}}}}public class Fixture {{Payload value;{}Fixture(Payload p){{Trace.log+=\"T\";value=p;}}public static void main(String[]x){{Fixture f=new Fixture(null,true);if(f.value!=null||!Trace.log.equals(\"T\"))throw new AssertionError(Trace.log);Trace.log=\"\";f=new Fixture(null,false);if(f.value==null||!Trace.log.equals(\"CAT\"))throw new AssertionError(Trace.log);Trace.log=\"\";Trace.fail=true;try{{new Fixture(null,false);throw new AssertionError();}}catch(IllegalArgumentException expected){{if(!expected.getMessage().equals(\"payload\"))throw new AssertionError();}}if(!Trace.log.equals(\"A\"))throw new AssertionError(Trace.log);}}}}",
        p.source
    );
    let dir =
        std::env::temp_dir().join(format!("constructor-prefix-diamond-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    for (tool, args) in [
        ("javac", vec!["Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output =
            std::process::Command::new(format!("/home/jtjisgod/tools/jdk-17.0.13+11/bin/{tool}"))
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
fn unproved_edges_escape_effects_cycles_and_multi_diamonds_reject() {
    for mode in 1..=8 {
        let (ir, b) = body(mode);
        assert!(
            plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &R).is_err(),
            "accepted mutation{mode}"
        );
    }
}
