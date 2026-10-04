#![allow(clippy::unwrap_used)]
use super::*;
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
struct Trusted;
impl backend_java::JavaResolver for Trusted {
    fn method(&self, _: MethodRef) -> Option<backend_java::JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || (*actual == class("org.threeten.bp.temporal.ChronoUnit")
                && *expected == class("org.threeten.bp.temporal.TemporalUnit"))
    }
}
fn class(name: &str) -> JavaType {
    JavaType::Class(name.into())
}
#[test]
fn exact_descriptor_upcast_preserves_source_and_effect_identity() {
    let mut arena = managed_ir::method::InsnArena::default();
    let effect = arena.insert(ManagedInsn::Return { value: None });
    let value = Value::ReferenceExpression {
        source: "org.threeten.bp.temporal.ChronoUnit.DAYS".into(),
        ty: class("org.threeten.bp.temporal.ChronoUnit"),
        effects: vec![effect],
    };
    assert_eq!(
        constant_source(
            &value,
            &class("org.threeten.bp.temporal.TemporalUnit"),
            &Trusted
        )
        .unwrap(),
        "(org.threeten.bp.temporal.TemporalUnit) (org.threeten.bp.temporal.ChronoUnit.DAYS)"
    );
    assert_eq!(expression_effects(&value), &[effect]);
    assert!(constant_source(&value, &class("unresolved.Unit"), &Trusted).is_err());
    assert!(constant_source(&value, &JavaType::Int, &Trusted).is_err());
    let parent = Value::ReferenceExpression {
        source: "unit".into(),
        ty: class("org.threeten.bp.temporal.TemporalUnit"),
        effects: vec![effect],
    };
    assert!(
        constant_source(
            &parent,
            &class("org.threeten.bp.temporal.ChronoUnit"),
            &Trusted
        )
        .is_err()
    );
}
#[test]
fn new_upcast_rule_does_not_admit_array_covariance_or_unresolved_nominals() {
    let array = Value::ReferenceExpression {
        source: "array".into(),
        ty: JavaType::Array(Box::new(class("org.threeten.bp.temporal.ChronoUnit"))),
        effects: vec![],
    };
    assert!(
        constant_source(
            &array,
            &JavaType::Array(Box::new(class("org.threeten.bp.temporal.TemporalUnit"))),
            &Trusted
        )
        .is_err()
    );
    assert!(
        constant_source(
            &array,
            &class("org.threeten.bp.temporal.TemporalUnit"),
            &Trusted
        )
        .is_err()
    );
    assert!(constant_source(&Value::Literal(Constant::Null), &JavaType::Int, &Trusted).is_err());
}
