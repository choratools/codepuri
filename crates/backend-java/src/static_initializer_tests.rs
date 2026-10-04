// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]

use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture".into(),
            name: "tick".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(field())
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn field() -> JavaField {
    JavaField {
        owner: "Fixture".into(),
        name: "counter".into(),
        ty: JavaType::Int,
        is_static: true,
    }
}
fn signature() -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: "<clinit>".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![],
        throws: vec![],
    }
}
fn context() -> StaticInitializerContext {
    StaticInitializerContext {
        owner: "Fixture".into(),
        writable_static_fields: vec![field()],
    }
}
fn fixture(return_value: bool) -> MethodIr<SsaForm> {
    let mut instructions = InsnArena::default();
    let mut values = ValueArena::default();
    let mut value = None;
    let first = instructions.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Int));
        value = Some(result);
        ManagedInsn::Const {
            result,
            value: Constant::Int(7),
        }
    });
    let value = value.unwrap();
    let mut cfg = Cfg::new(first);
    let mut last = cfg.entry();
    for instruction in [
        ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(0),
            value,
        },
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(0),
            args: vec![],
        },
        ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(0),
            value,
        },
        ManagedInsn::Return {
            value: return_value.then_some(value),
        },
    ] {
        let next = cfg.add_block(instructions.insert(instruction));
        cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
        last = next;
    }
    MethodIr::<SsaForm>::from_parts(cfg, instructions, values)
}
#[test]
fn field_assignments_and_effects_keep_instruction_order() {
    let source = emit_static_initializer(
        &fixture(false),
        &signature(),
        &ParameterBindings::default(),
        &Resolver,
        &context(),
    )
    .unwrap();
    assert_eq!(
        source,
        "static {\n    int v0 = 7;\n    Fixture.counter = v0;\n    Fixture.tick();\n    Fixture.counter = v0;\n}\n"
    );
}
#[test]
fn rejects_value_return() {
    assert!(matches!(
        emit_static_initializer(
            &fixture(true),
            &signature(),
            &ParameterBindings::default(),
            &Resolver,
            &context()
        ),
        Err(Unsupported::Type(_))
    ));
}

#[test]
fn terminal_throw_returns_diagnostic_instead_of_successor_panic() {
    struct ThrowResolver;
    impl JavaResolver for ThrowResolver {
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "Fixture".into(),
                name: "failure".into(),
                is_static: true,
                return_type: JavaType::Class("java.lang.RuntimeException".into()),
                parameters: vec![],
                throws: vec![],
            })
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn class(&self, _: ClassRef) -> Option<String> {
            Some("java.lang.RuntimeException".into())
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
        fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
            actual == expected
                || (*actual == JavaType::Class("java.lang.RuntimeException".into())
                    && *expected == JavaType::Class("java.lang.Throwable".into()))
        }
    }
    let mut instructions = InsnArena::default();
    let mut values = ValueArena::default();
    let mut result = None;
    let invoke = instructions.insert_with_key(|id| {
        let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        result = Some(value);
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(value),
            method: MethodRef::new(0),
            args: vec![],
        }
    });
    let thrown = instructions.insert(ManagedInsn::Throw {
        value: result.unwrap(),
    });
    let mut cfg = Cfg::new(invoke);
    let terminal = cfg.add_block(thrown);
    cfg.add_edge(cfg.entry(), terminal, EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, values);
    assert!(matches!(
        emit_method(
            &ir,
            &JavaMethod {
                name: "body".into(),
                ..signature()
            },
            &ParameterBindings::default(),
            &ThrowResolver
        ),
        SourceResult::Complete { .. }
    ));
    assert!(matches!(emit_static_initializer(&ir, &signature(),
        &ParameterBindings::default(), &ThrowResolver, &context()),
        Err(Unsupported::ControlFlow(reason)) if reason == "static initializer has no normal return"));
}
#[test]
fn requires_field_member_context() {
    let mut context = context();
    context.writable_static_fields.clear();
    assert!(matches!(
        emit_static_initializer(
            &fixture(false),
            &signature(),
            &ParameterBindings::default(),
            &Resolver,
            &context
        ),
        Err(Unsupported::Metadata(_))
    ));
}
#[test]
fn rejects_non_initializer_and_wrong_enclosing_class() {
    let mut method = signature();
    method.name = "ordinary".into();
    assert!(
        emit_static_initializer(
            &fixture(false),
            &method,
            &ParameterBindings::default(),
            &Resolver,
            &context()
        )
        .is_err()
    );
    let mut context = context();
    context.owner = "Other".into();
    assert!(
        emit_static_initializer(
            &fixture(false),
            &signature(),
            &ParameterBindings::default(),
            &Resolver,
            &context
        )
        .is_err()
    );
}
