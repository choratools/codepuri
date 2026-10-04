//! Detailed subset diagnostics preserve acceptance and identify chain-level failures.
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
fn retained_order_matches_legacy_and_missing_target_is_exact() {
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
    let resolver = Resolver { cycle: false };
    let report =
        plan_constructor_reachable_subset_detailed("Fixture", &inputs, &[], &resolver).unwrap();
    let old = plan_constructor_reachable_subset("Fixture", &inputs, &[], &resolver).unwrap();
    assert_eq!(
        report
            .retained
            .iter()
            .map(|r| (r.reference, &r.plan.source))
            .collect::<Vec<_>>(),
        old.iter()
            .map(|r| (r.reference, &r.plan.source))
            .collect::<Vec<_>>()
    );
    assert!(report.rejected.is_empty());
    let report =
        plan_constructor_reachable_subset_detailed("Fixture", &inputs[..1], &[], &resolver)
            .unwrap();
    assert!(report.retained.is_empty());
    assert_eq!(report.rejected[0].reference, MethodRef::new(1));
    assert_eq!(
        report.rejected[0].attempted_chain,
        vec![MethodRef::new(1), MethodRef::new(0)]
    );
    assert_eq!(
        report.rejected[0].phase,
        ReachableRejectionPhase::MissingInput
    );
    assert_eq!(report.rejected[0].missing_target, Some(MethodRef::new(0)));
    assert_eq!(report.rejected[0].error, None);
}
#[test]
fn cycles_and_group_errors_are_chain_level_in_input_order() {
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
    let report = plan_constructor_reachable_subset_detailed(
        "Fixture",
        &inputs,
        &[],
        &Resolver { cycle: true },
    )
    .unwrap();
    assert_eq!(
        report
            .rejected
            .iter()
            .map(|r| r.reference)
            .collect::<Vec<_>>(),
        vec![MethodRef::new(1), MethodRef::new(0)]
    );
    for r in report.rejected {
        assert_eq!(r.phase, ReachableRejectionPhase::Cycle);
        assert_eq!(r.attempted_chain.first(), Some(&r.reference));
        assert_eq!(r.attempted_chain.last(), r.missing_target.as_ref());
        assert!(r.error.is_none());
    }
    let finals = [BlankFinal {
        reference: FieldRef::new(0),
        writes: [(MethodRef::new(0), 0)].into(),
    }];
    let report = plan_constructor_reachable_subset_detailed(
        "Fixture",
        &inputs,
        &finals,
        &Resolver { cycle: false },
    )
    .unwrap();
    assert!(report.retained.is_empty());
    for r in report.rejected {
        assert_eq!(r.phase, ReachableRejectionPhase::ChainValidation);
        assert!(r.error.is_some());
        assert!(r.missing_target.is_none());
    }
}
