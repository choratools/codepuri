//! Catch-all and genuine checkcast nominal domains do not grant precise rethrow.
use super::*;
use managed_ir::refs::TypeRef;
struct R;
impl JavaResolver for R {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Context.method(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        Context.field(r)
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Context.class(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Context.string(r)
    }
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        Some(match r.index() {
            0 => JavaType::Class("java.lang.Throwable".into()),
            1 => JavaType::Class("java.lang.String".into()),
            2 => JavaType::Int,
            _ => return None,
        })
    }
}
impl ThrowsHierarchy for R {
    fn exception_kind(&self, n: &str) -> Option<ExceptionKind> {
        Context.exception_kind(n)
    }
}
#[test]
fn catchall_and_exact_resumed_cast_have_nominal_but_not_precise_rethrow_evidence() {
    let mut code = code();
    code.handlers[0].handlers.clear();
    code.handlers[0].catch_all_addr = Some(4);
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &R);
    let captured = thrown(&ssa, 5);
    assert_eq!(
        proof.nominal_thrown_types(ssa.ir(), captured, &R),
        Some(BTreeSet::from(["java.lang.Throwable".into()]))
    );
    let mut cast = None;
    let mut mixed = None;
    let mut wrong = vec![];
    let mut opaque = None;
    let ir = rebuild(&ssa, |insns, values, cfg| {
        let edges = cfg.edge_ids().take(2).collect::<Vec<_>>();
        for target in 0..4 {
            insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ManagedType::Unknown));
                if target == 0 {
                    cast = Some(v)
                } else {
                    wrong.push(v)
                }
                ManagedInsn::Cast {
                    result: v,
                    value: captured,
                    target: TypeRef::new(target),
                }
            });
        }
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Unknown));
            mixed = Some(v);
            ManagedInsn::Phi {
                result: v,
                inputs: vec![(edges[0], captured), (edges[1], cast.unwrap())],
            }
        });
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Unknown));
            opaque = Some(v);
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![v],
                effects: EffectFlags::NONE,
            }
        });
    });
    assert!(proof.origins(&ir, mixed.unwrap()).is_none());
    assert_eq!(
        proof.nominal_thrown_types(&ir, mixed.unwrap(), &R),
        Some(BTreeSet::from(["java.lang.Throwable".into()]))
    );
    for value in wrong.into_iter().chain([opaque.unwrap()]) {
        assert!(proof.nominal_thrown_types(&ir, value, &R).is_none());
    }
}
