//! Multi-predecessor constructor suffixes retain selected original Phi batches.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
struct R;
impl JavaResolver for R {
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        // The unchanged JVM fixture declares package-private final first/second.
        Some(0x0010)
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        use backend_java::type_compatibility::{Declaration, Hierarchy};
        let mut hierarchy = Hierarchy::default();
        for (name, parent) in [
            ("java.lang.Object", None),
            ("java.lang.Throwable", Some("java.lang.Object")),
            ("java.lang.Exception", Some("java.lang.Throwable")),
            ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ] {
            hierarchy
                .insert(Declaration {
                    name: name.into(),
                    superclass: parent.map(str::to_owned),
                    interfaces: vec![],
                    is_interface: false,
                })
                .unwrap();
        }
        matches!(
            hierarchy.assignable(actual, expected),
            backend_java::type_compatibility::Compatibility::Assignable
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "Fixture",
                1 => "Base",
                2 => "java.lang.RuntimeException",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn field(&self, f: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: if f.index() == 0 { "first" } else { "second" }.into(),
            ty: JavaType::Int,
            is_static: false,
        })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() == 2 {
            Some(JavaMethod {
                owner: "Trace".into(),
                name: "mark".into(),
                parameters: vec![JavaType::Int],
                return_type: JavaType::Void,
                is_static: true,
                throws: vec![],
            })
        } else {
            self.initializer(m).map(|i| i.method)
        }
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
}
impl ConstructorResolver for R {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: match m.index() {
                    0 => "Fixture",
                    1 => "Base",
                    _ => return None,
                }
                .into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 {
                    vec![
                        JavaType::Boolean,
                        JavaType::Boolean,
                        JavaType::Int,
                        JavaType::Int,
                        JavaType::Class("java.lang.RuntimeException".into()),
                    ]
                } else {
                    vec![]
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
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
            "Base" => vec![MethodRef::new(1)],
            _ => vec![],
        })
    }
}
fn def(
    i: &mut InsnArena,
    v: &mut ValueArena,
    t: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut result = None;
    let id = i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, t));
        result = Some(x);
        f(x)
    });
    (id, result.unwrap())
}
fn input(i: &mut InsnArena, v: &mut ValueArena, t: ManagedType) -> ValueId {
    def(i, v, t, |x| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![x],
        effects: EffectFlags::NONE,
    })
    .1
}
fn body(mode: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let this = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(0)));
    let a = input(&mut i, &mut v, ManagedType::Boolean);
    let b = input(&mut i, &mut v, ManagedType::Boolean);
    let left = input(&mut i, &mut v, ManagedType::Int);
    let right = input(&mut i, &mut v, ManagedType::Int);
    let error = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(2)));
    let chain = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this],
    });
    let outer = i.insert(ManagedInsn::Return { value: None });
    let inner = i.insert(ManagedInsn::Return { value: None });
    let mut arm_ids = vec![];
    let mut arm_values = vec![];
    for token in 1..=3 {
        let (literal, value) = def(&mut i, &mut v, ManagedType::Int, |x| ManagedInsn::Const {
            result: x,
            value: Constant::Int(token),
        });
        let mark = i.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![value],
        });
        arm_ids.push((literal, mark));
        arm_values.push(value);
    }
    let receiver_phi = if mode == 15 {
        Some(def(
            &mut i,
            &mut v,
            ManagedType::Reference(ClassRef::new(0)),
            |result| ManagedInsn::Phi {
                result,
                inputs: vec![],
            },
        ))
    } else {
        None
    };
    let (p, x) = def(&mut i, &mut v, ManagedType::Int, |x| ManagedInsn::Phi {
        result: x,
        inputs: vec![],
    });
    let (p2, y) = def(&mut i, &mut v, ManagedType::Int, |x| ManagedInsn::Phi {
        result: x,
        inputs: vec![],
    });
    let (four, four_value) = def(&mut i, &mut v, ManagedType::Int, |x| ManagedInsn::Const {
        result: x,
        value: Constant::Int(4),
    });
    let common = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(2),
        args: vec![four_value],
    });
    let store = i.insert(ManagedInsn::FieldPut {
        object: Some(receiver_phi.map_or(this, |(_, value)| value)),
        field: FieldRef::new(0),
        value: x,
    });
    let store2 = i.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(1),
        value: y,
    });
    let ret = i.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(chain);
    let root = cfg.entry();
    let ob = cfg.add_block(outer);
    let ib = cfg.add_block(inner);
    let mut arms = vec![];
    for (literal, mark) in &arm_ids {
        arms.push((cfg.add_block(*literal), cfg.add_block(*mark)));
    }
    let ph = cfg.add_block(receiver_phi.map_or(p, |(id, _)| id));
    let int_phi = if receiver_phi.is_some() {
        let next = cfg.add_block(p);
        cfg.add_edge(ph, next, EdgeKind::Unconditional).unwrap();
        next
    } else {
        ph
    };
    let ph2 = cfg.add_block(p2);
    let fb = cfg.add_block(four);
    let cb = cfg.add_block(common);
    let sb = cfg.add_block(store);
    let sb2 = cfg.add_block(store2);
    let rb = cfg.add_block(ret);
    cfg.add_edge(root, ob, EdgeKind::Unconditional).unwrap();
    let ot = cfg.add_edge(ob, ib, EdgeKind::BranchTrue).unwrap();
    let of = cfg.add_edge(ob, arms[2].0, EdgeKind::BranchFalse).unwrap();
    let it = cfg.add_edge(ib, arms[0].0, EdgeKind::BranchTrue).unwrap();
    let ie = cfg.add_edge(ib, arms[1].0, EdgeKind::BranchFalse).unwrap();
    let mut incoming = vec![];
    for (index, (literal, mark)) in arms.iter().enumerate() {
        cfg.add_edge(*literal, *mark, EdgeKind::Unconditional)
            .unwrap();
        if index != 2 || !matches!(mode, 10 | 11) {
            incoming.push(cfg.add_edge(*mark, ph, EdgeKind::Unconditional).unwrap());
        }
    }
    cfg.add_edge(int_phi, ph2, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(ph2, fb, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(fb, cb, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(cb, sb, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(sb, sb2, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(sb2, rb, EdgeKind::Unconditional).unwrap();
    *i.get_mut(outer).unwrap() = ManagedInsn::Branch {
        condition: Some(a),
        then_edge: ot,
        else_edge: Some(of),
    };
    *i.get_mut(inner).unwrap() = ManagedInsn::Branch {
        condition: Some(b),
        then_edge: it,
        else_edge: Some(ie),
    };
    let mut inputs = vec![
        (incoming[0], left),
        (incoming[1], right),
        (incoming.get(2).copied().unwrap_or(incoming[0]), left),
    ];
    let mut inputs2 = vec![
        (incoming[0], right),
        (incoming[1], left),
        (incoming.get(2).copied().unwrap_or(incoming[0]), left),
    ];
    match mode {
        1 => {
            inputs.pop();
        }
        2 => inputs[1].0 = incoming[0],
        3 => inputs[1].1 = arm_values[0],
        4 => inputs2[0].1 = x,
        5 => {
            cfg.add_edge(
                arms[0].1,
                rb,
                EdgeKind::Exception("java.lang.Exception".into()),
            )
            .unwrap();
        }
        6 => {
            cfg.add_edge(arms[2].1, ob, EdgeKind::Unconditional)
                .unwrap();
        }
        7 => {
            *i.get_mut(arm_ids[0].1).unwrap() = ManagedInsn::FieldPut {
                object: Some(this),
                field: FieldRef::new(0),
                value: left,
            };
        }
        8 => {
            *i.get_mut(store).unwrap() = ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![left],
            };
        }
        9 => {
            *i.get_mut(arm_ids[0].1).unwrap() = ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                method: MethodRef::new(1),
                args: vec![this],
            };
        }
        10 => {
            *i.get_mut(arm_ids[2].1).unwrap() = ManagedInsn::Return { value: None };

            inputs.pop();
            inputs2.pop();
        }
        11 => {
            *i.get_mut(arm_ids[2].1).unwrap() = ManagedInsn::Throw { value: error };

            inputs.pop();
            inputs2.pop();
        }
        12 => {
            *v.get_mut(x).unwrap() = Value::new(p2, ManagedType::Int);
        }
        13 => {
            inputs2[0].0 = cfg.block(arms[0].0).unwrap().outgoing()[0];
        }
        _ => {}
    }
    if let Some((id, result)) = receiver_phi {
        *i.get_mut(id).unwrap() = ManagedInsn::Phi {
            result,
            inputs: incoming.iter().map(|edge| (*edge, this)).collect(),
        };
    }
    *i.get_mut(p).unwrap() = ManagedInsn::Phi { result: x, inputs };
    *i.get_mut(p2).unwrap() = ManagedInsn::Phi {
        result: y,
        inputs: inputs2,
    };
    (
        MethodIr::<SsaForm>::from_parts(cfg, i, v),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![a, b, left, right, error],
            names: vec![],
        },
    )
}
fn final_plan(
    ir: &MethodIr<SsaForm>,
    b: &ParameterBindings,
) -> Result<Vec<ConstructorPlan>, backend_java::Unsupported> {
    plan_constructor_family(
        "Fixture",
        &[ConstructorInput {
            reference: MethodRef::new(0),
            ir,
            bindings: b,
        }],
        &[
            BlankFinal {
                reference: FieldRef::new(0),
                writes: [(MethodRef::new(0), 1)].into(),
            },
            BlankFinal {
                reference: FieldRef::new(1),
                writes: [(MethodRef::new(0), 1)].into(),
            },
        ],
        &R,
    )
}
#[test]
fn selected_three_way_parallel_phi_copies_and_complete_family() {
    let (ir, b) = body(0);
    let plans = final_plan(&ir, &b).unwrap();
    assert_eq!(plans.len(), 1);
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &b,
    };
    let evidence = constructor_initialization_evidence(
        &input,
        "Base",
        &[FieldRef::new(0), FieldRef::new(1)],
        &R,
    )
    .unwrap();
    assert!(
        evidence
            .fields
            .iter()
            .all(|f| f.normal_return_counts == [1].into())
    );
    // The shared tail has one source placement; selected edge copies still
    // feed exactly one assignment on every original normal-return path.
    assert_eq!(plans[0].source.matches("this.first =").count(), 1);
}
#[test]
fn malformed_edges_unavailable_and_current_batch_sources_catches_cycles_and_final_paths_deny() {
    for mode in (1..=13).filter(|mode| *mode != 11) {
        let (ir, b) = body(mode);
        assert!(final_plan(&ir, &b).is_err(), "accepted mutation{mode}");
    }
    let (ir, b) = body(0);
    assert!(plan_constructor_family("Fixture", &[], &[], &R).is_err());
    let _ = plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).unwrap();
}
#[test]
fn three_paths_and_throw_preserve_selected_values_effects_and_exception_identity_on_jvm() {
    let mut snippets = Vec::new();
    for mode in [0, 11, 15] {
        let (ir, b) = body(mode);
        snippets.push(final_plan(&ir, &b).unwrap().remove(0).source);
    }
    for (mode, constructor) in [0, 11, 15].into_iter().zip(snippets) {
        let source = format!(
            r#"class Trace{{static String log="";static void mark(int n){{log+=n;if(n==4&&Boolean.getBoolean("commonFail"))throw new IllegalStateException("common");}}}}class Base{{Base(){{Trace.log+="B";if(Boolean.getBoolean("baseFail"))throw new IllegalArgumentException("base");}}}}public class Fixture extends Base{{final int first,second;{constructor}public static void main(String[]args){{boolean a=Boolean.parseBoolean(args[0]),b=Boolean.parseBoolean(args[1]);RuntimeException e=new RuntimeException("same");try{{Fixture f=new Fixture(a,b,7,9,e);System.out.println(Trace.log+":"+f.first+":"+f.second);}}catch(RuntimeException x){{System.out.println(Trace.log+":"+(x==e?"same":x.getMessage()));}}}}}}"#
        );
        let dir = std::env::temp_dir().join(format!(
            "constructor-selected-receiver-alias-{}-{mode}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fixture.java"), source).unwrap();
        let java = std::path::Path::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin");
        let r = std::process::Command::new(java.join("javac"))
            .arg("Fixture.java")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        for (a, b, expected) in [
            ("true", "true", "B14:7:9"),
            ("true", "false", "B24:9:7"),
            (
                "false",
                "true",
                if mode != 11 { "B34:7:7" } else { "B:same" },
            ),
        ] {
            let r = std::process::Command::new(java.join("java"))
                .args(["-Xverify:all", "-cp", ".", "Fixture", a, b])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
            assert_eq!(String::from_utf8(r.stdout).unwrap().trim(), expected);
        }
        for (property, expected) in [("baseFail", "B:base"), ("commonFail", "B14:common")] {
            let r = std::process::Command::new(java.join("java"))
                .args([
                    &format!("-D{property}=true"),
                    "-Xverify:all",
                    "-cp",
                    ".",
                    "Fixture",
                    "true",
                    "true",
                ])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(r.status.success());
            assert_eq!(String::from_utf8(r.stdout).unwrap().trim(), expected);
        }
    }
}
