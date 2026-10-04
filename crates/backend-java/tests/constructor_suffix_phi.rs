//! Constructor suffix joins keep final stores after predecessor-selected Phi copies.
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
        Some(
            match c.index() {
                0 => "Fixture",
                1 => "Payload",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: "value".into(),
            ty: JavaType::Class("Payload".into()),
            is_static: false,
        })
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
                owner: if m.index() == 2 { "Base" } else { "Fixture" }.into(),
                name: "<init>".into(),
                parameters: if m.index() == 2 {
                    vec![JavaType::Class("java.lang.Object".into())]
                } else {
                    vec![
                        JavaType::Class("java.lang.Object".into()),
                        JavaType::Class("Payload".into()),
                    ]
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
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
    // Class 2 is resolved separately as java.lang.Object.
    let context = input(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(2)),
    );
    let original = input(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(1)),
    );
    let chain = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(2),
        args: vec![this, context],
    });
    let (zero, z) = definition(&mut insns, &mut values, ManagedType::Null, |v| {
        ManagedInsn::Const {
            result: v,
            value: managed_ir::insn::Constant::Zero,
        }
    });
    let (compare, condition) = definition(&mut insns, &mut values, ManagedType::Boolean, |v| {
        ManagedInsn::Compare {
            op: managed_ir::ops::CompareOp::NotEqual,
            result: v,
            lhs: context,
            rhs: z,
        }
    });
    let branch = insns.insert(ManagedInsn::Return { value: None });
    let (null, n) = definition(&mut insns, &mut values, ManagedType::Null, |v| {
        ManagedInsn::Const {
            result: v,
            value: managed_ir::insn::Constant::Zero,
        }
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
    let store = insns.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(0),
        value: result,
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(chain);
    let c = cfg.entry();
    let z = cfg.add_block(zero);
    let q = cfg.add_block(compare);
    let b = cfg.add_block(branch);
    let nblock = cfg.add_block(null);
    let p = cfg.add_block(phi);
    let f = cfg.add_block(store);
    let r = cfg.add_block(ret);
    cfg.add_edge(c, z, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(z, q, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(q, b, EdgeKind::Unconditional).unwrap();
    let yes = cfg.add_edge(b, p, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(b, nblock, EdgeKind::BranchFalse).unwrap();
    let tail = cfg.add_edge(nblock, p, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(p, f, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(f, r, EdgeKind::Unconditional).unwrap();
    *insns.get_mut(branch).unwrap() = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge: yes,
        else_edge: Some(no),
    };
    *insns.get_mut(phi).unwrap() = ManagedInsn::Phi {
        result,
        inputs: vec![
            (yes, if mode == 3 { result } else { original }),
            (if mode == 1 { yes } else { tail }, n),
        ],
    };
    if mode == 2 {
        cfg.add_edge(nblock, b, EdgeKind::Unconditional).unwrap();
    }
    if mode == 4 {
        *insns.get_mut(null).unwrap() = ManagedInsn::Opaque {
            family_tag: 99,
            reads: vec![],
            writes: vec![n],
            effects: EffectFlags::NONE,
        };
    }
    if mode == 5 {
        cfg.add_edge(b, p, EdgeKind::Exception("java.lang.Exception".into()))
            .unwrap();
    }
    if mode == 6 {
        *insns.get_mut(null).unwrap() = ManagedInsn::FieldPut {
            object: Some(this),
            field: FieldRef::new(0),
            value: original,
        };
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![context, original],
            names: vec![],
        },
    )
}
#[test]
fn final_phi_store_uses_selected_edge_once_after_parent() {
    let (ir, b) = body(0);
    let plan = plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).unwrap();
    let source = format!(
        r#"class Payload {{}}class Trace {{static String log="";}}class Base {{Base(Object o){{Trace.log+="B";if(o instanceof String)throw new IllegalArgumentException("base");}}}}public class Fixture extends Base {{final Payload value;{}public static void main(String[]x){{Payload p=new Payload();if(new Fixture(null,p).value!=null)throw new AssertionError();if(new Fixture(new Object(),p).value!=p)throw new AssertionError();if(new Fixture(new Object(),null).value!=null)throw new AssertionError();try{{new Fixture("fail",p);throw new AssertionError();}}catch(IllegalArgumentException expected){{}}if(!Trace.log.equals("BBBB"))throw new AssertionError(Trace.log);}}}}"#,
        plan.source
    );
    let dir = std::env::temp_dir().join(format!("constructor-suffix-phi-{}", std::process::id()));
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
fn wrong_edges_cycles_unavailable_sources_and_exception_joins_reject() {
    for mode in 1..=6 {
        let (ir, b) = body(mode);
        assert!(
            plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).is_err(),
            "accepted mutation{mode}"
        );
    }
}
impl TrustedConstructorResolver for R {
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
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        Some(match owner {
            "Fixture" => vec![MethodRef::new(0)],
            "Base" => vec![MethodRef::new(2)],
            _ => vec![],
        })
    }
}
#[test]
fn final_family_proof_remains_once_per_normal_path() {
    let (ir, b) = body(0);
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &b,
    };
    let finals = [BlankFinal {
        reference: FieldRef::new(0),
        writes: [(MethodRef::new(0), 1)].into(),
    }];
    let evidence =
        constructor_initialization_evidence(&input, "Base", &[FieldRef::new(0)], &R).unwrap();
    assert_eq!(evidence.fields[0].normal_return_counts, [1].into());
    assert_eq!(
        plan_constructor_family("Fixture", &[input], &finals, &R)
            .unwrap()
            .len(),
        1
    );
    let (ir, b) = body(6);
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &b,
    };
    let evidence =
        constructor_initialization_evidence(&input, "Base", &[FieldRef::new(0)], &R).unwrap();
    assert_eq!(evidence.fields[0].normal_return_counts, [1, 2].into());
    assert!(plan_constructor_family("Fixture", &[input], &finals, &R).is_err());
}
