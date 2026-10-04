//! Genuine resolver hierarchy evidence for heterogeneous reference joins.
#![allow(clippy::unwrap_used)]
use backend_java::{JavaResolver, JavaType, contextual_locals};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Resolver(bool);
impl JavaResolver for Resolver {
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || self.0
                && matches!((actual, expected),
            (JavaType::Class(a), JavaType::Class(b))
                if matches!(a.as_str(), "java.lang.IllegalArgumentException" | "java.lang.IllegalStateException")
                    && matches!(b.as_str(), "java.lang.RuntimeException" | "java.lang.Throwable"))
            || self.0
                && matches!((actual, expected),
                (JavaType::Class(a), JavaType::Class(b))
                    if a == "java.lang.RuntimeException" && b == "java.lang.Throwable")
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        match reference.index() {
            0 => Some("java.lang.IllegalArgumentException".into()),
            1 => Some("java.lang.IllegalStateException".into()),
            _ => None,
        }
    }
    fn method(&self, _: MethodRef) -> Option<backend_java::JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
#[test]
fn throw_join_requires_attested_common_unchecked_ancestor() {
    let mut instructions = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut add = |ty, build: &dyn Fn(_) -> ManagedInsn| {
        let mut result = None;
        instructions.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ty));
            result = Some(value);
            build(value)
        });
        result.unwrap()
    };
    let left = add(ManagedType::Reference(ClassRef::new(0)), &|result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let right = add(ManagedType::Reference(ClassRef::new(1)), &|result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(1),
        }
    });
    let phi = add(ManagedType::Unknown, &|result| ManagedInsn::Phi {
        result,
        inputs: vec![(Default::default(), left), (Default::default(), right)],
    });
    let terminal = instructions.insert(ManagedInsn::Throw { value: phi });
    let ir =
        MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(terminal), instructions, values);
    let proved =
        contextual_locals::declaration_plan(&ir, &JavaType::Void, &Resolver(true)).unwrap();
    assert_eq!(
        proved.get(&phi),
        Some(&JavaType::Class("java.lang.RuntimeException".into()))
    );
    let missing =
        contextual_locals::declaration_plan(&ir, &JavaType::Void, &Resolver(false)).unwrap();
    assert!(!missing.contains_key(&phi));
}
