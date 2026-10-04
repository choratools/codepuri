//! Definite-assignment evidence must inspect branches and failed stores/invokes.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::*;
use backend_java::{DeclarationBindings, JavaField, JavaMethod, JavaResolver, JavaType};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};
use std::collections::HashMap;

struct Resolver {
    cycle: bool,
}
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
                owner: if r.index() == 3 { "Base" } else { "Fixture" }.into(),
                name: "<init>".into(),
                parameters: vec![
                    JavaType::Int;
                    if r.index() == 3 {
                        1
                    } else {
                        r.index() as usize + 1
                    }
                ],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: if self.cycle && r.index() == 0 {
                Some(MethodRef::new(1))
            } else {
                (r.index() == 1).then_some(MethodRef::new(0))
            },
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
        Some(vec![
            MethodRef::new(0),
            MethodRef::new(1),
            MethodRef::new(2),
        ])
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

fn body(target: u32, writes: usize, arity: usize) -> (MethodIr<SsaForm>, DeclarationBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let receiver = parameter(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let p = parameter(&mut insns, &mut values, ManagedType::Int);
    let init = insns.insert(invocation(receiver, p, target));
    let mut cfg = Cfg::new(init);
    let mut tail = cfg.entry();
    for _ in 0..writes {
        let node = cfg.add_block(insns.insert(store(receiver, p)));
        cfg.add_edge(tail, node, EdgeKind::Unconditional).unwrap();
        tail = node;
    }
    let end = cfg.add_block(insns.insert(ManagedInsn::Return { value: None }));
    cfg.add_edge(tail, end, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        DeclarationBindings {
            receiver: Some(receiver),
            parameters: (0..arity).map(|i| (i == 0).then_some(p)).collect(),
            names: vec![],
        },
    )
}
#[test]
fn reachable_chain_retains_exact_identities_and_compiles_without_unrelated_body() {
    let (root, rb) = body(3, 1, 1);
    let (caller, cb) = body(0, 0, 2);
    let inputs = [
        ConstructorInput {
            reference: MethodRef::new(1),
            ir: &caller,
            bindings: &cb,
        },
        ConstructorInput {
            reference: MethodRef::new(0),
            ir: &root,
            bindings: &rb,
        },
    ];
    let finals = [BlankFinal {
        reference: FieldRef::new(0),
        writes: [(MethodRef::new(0), 1)].into(),
    }];
    let resolver = Resolver { cycle: false };
    assert!(plan_constructor_family("Fixture", &inputs, &finals, &resolver).is_err());
    let plans = plan_constructor_reachable_subset("Fixture", &inputs, &finals, &resolver).unwrap();
    assert_eq!(
        plans
            .iter()
            .map(|p| p.reference.index())
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    let source = format!(
        "class Base {{ Base(int p){{ if(p<0)throw new IllegalArgumentException(); }} }} public class Fixture extends Base {{ final int number; {} {} public static void main(String[]x){{ if(new Fixture(9,7).number!=9)throw new AssertionError(); try{{new Fixture(-1,7);throw new AssertionError();}}catch(IllegalArgumentException expected){{}} }} }}",
        plans[0].plan.source, plans[1].plan.source
    );
    let dir = std::env::temp_dir().join(format!("constructor-reachable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
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
fn missing_cycle_duplicate_and_invalid_final_counts_never_publish() {
    let (root, rb) = body(3, 1, 1);
    let (caller, cb) = body(0, 0, 2);
    let one = ConstructorInput {
        reference: MethodRef::new(1),
        ir: &caller,
        bindings: &cb,
    };
    assert!(
        plan_constructor_reachable_subset("Fixture", &[one], &[], &Resolver { cycle: false })
            .unwrap()
            .is_empty()
    );
    let inputs = [
        ConstructorInput {
            reference: MethodRef::new(0),
            ir: &root,
            bindings: &rb,
        },
        ConstructorInput {
            reference: MethodRef::new(1),
            ir: &caller,
            bindings: &cb,
        },
    ];
    assert!(
        plan_constructor_reachable_subset("Fixture", &inputs, &[], &Resolver { cycle: true })
            .unwrap()
            .is_empty()
    );
    let duplicate = [
        ConstructorInput {
            reference: MethodRef::new(0),
            ir: &root,
            bindings: &rb,
        },
        ConstructorInput {
            reference: MethodRef::new(0),
            ir: &root,
            bindings: &rb,
        },
    ];
    assert!(
        plan_constructor_reachable_subset("Fixture", &duplicate, &[], &Resolver { cycle: false })
            .is_err()
    );
    let wrong = [BlankFinal {
        reference: FieldRef::new(0),
        writes: [(MethodRef::new(0), 0)].into(),
    }];
    assert!(
        plan_constructor_reachable_subset("Fixture", &inputs, &wrong, &Resolver { cycle: false })
            .unwrap()
            .is_empty()
    );
    let duplicate_fields = [
        BlankFinal {
            reference: FieldRef::new(0),
            writes: HashMap::new(),
        },
        BlankFinal {
            reference: FieldRef::new(0),
            writes: HashMap::new(),
        },
    ];
    assert!(
        plan_constructor_reachable_subset(
            "Fixture",
            &inputs,
            &duplicate_fields,
            &Resolver { cycle: false }
        )
        .is_err()
    );
    for writes in [0, 2] {
        let (ir, bindings) = body(3, writes, 1);
        let input = ConstructorInput {
            reference: MethodRef::new(0),
            ir: &ir,
            bindings: &bindings,
        };
        let finals = [BlankFinal {
            reference: FieldRef::new(0),
            writes: [(MethodRef::new(0), writes)].into(),
        }];
        assert!(
            plan_constructor_reachable_subset(
                "Fixture",
                &[input],
                &finals,
                &Resolver { cycle: false }
            )
            .unwrap()
            .is_empty()
        );
    }
}
