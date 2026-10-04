//! Selected super declarations remain separate from ordinary symbolic metadata.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

fn target(owner: &str, name: &str, parameter: &str) -> JavaMethod {
    JavaMethod {
        owner: owner.into(),
        name: name.into(),
        is_static: false,
        return_type: JavaType::Int,
        parameters: vec![JavaType::Class(parameter.into())],
        throws: vec![],
    }
}

pub(crate) struct Resolver {
    symbolic: Option<JavaMethod>,
    symbolic_flags: Option<u32>,
    selected: Option<SuperInvocation>,
}
impl Resolver {
    pub(crate) fn fixture() -> Self {
        Self {
            symbolic: Some(target(
                "SuperBindingHarness.Base",
                "choose",
                "java.lang.String",
            )),
            symbolic_flags: Some(1),
            selected: Some(SuperInvocation {
                method: target("SuperBindingHarness.Mid", "choose", "java.lang.Object"),
                access_flags: 1,
            }),
        }
    }
}
impl JavaResolver for Resolver {
    fn super_invocation(&self, caller: &str, reference: MethodRef) -> Option<SuperInvocation> {
        (caller == "SuperBindingHarness.Child" && reference == MethodRef::new(0))
            .then(|| self.selected.clone())
            .flatten()
    }
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        (reference == MethodRef::new(0))
            .then(|| self.symbolic.clone())
            .flatten()
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        self.symbolic_flags
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        match reference.index() {
            0 => Some("SuperBindingHarness.Child".into()),
            1 => Some("java.lang.String".into()),
            _ => None,
        }
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || matches!((actual, expected), (JavaType::Class(a), JavaType::Class(b))
                if a == "java.lang.String" && b == "java.lang.Object"
                    || a == "SuperBindingHarness.Child" && b == "SuperBindingHarness.Base")
    }
}

fn emit(
    kind: InvokeKind,
    wrong_receiver: bool,
    throws: Vec<String>,
    r: &impl JavaResolver,
) -> SourceResult {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let mut parameter = |ty| {
        let mut value = None;
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ty));
            value = Some(v);
            ManagedInsn::Opaque {
                family_tag: 32769,
                reads: vec![],
                writes: vec![v],
                effects: managed_ir::effects::EffectFlags::NONE,
            }
        });
        value.unwrap()
    };
    let receiver = parameter(ManagedType::Reference(ClassRef::new(0)));
    let argument = parameter(ManagedType::Reference(ClassRef::new(1)));
    let other = wrong_receiver.then(|| parameter(ManagedType::Reference(ClassRef::new(0))));
    let mut result = None;
    let invoke = insns.insert_with_key(|id| {
        let value = values.insert(Value::new(id, ManagedType::Int));
        result = Some(value);
        ManagedInsn::Invoke {
            kind,
            result: Some(value),
            method: MethodRef::new(0),
            args: vec![other.unwrap_or(receiver), argument],
        }
    });
    let ret = insns.insert(ManagedInsn::Return { value: result });
    let mut cfg = Cfg::new(invoke);
    let end = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), end, EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let mut signature = target("SuperBindingHarness.Child", "call", "java.lang.String");
    signature.throws = throws;
    let mut parameters = vec![argument];
    if let Some(other) = other {
        parameters.push(other);
        signature
            .parameters
            .push(JavaType::Class("SuperBindingHarness.Child".into()));
    }
    emit_method(
        &ir,
        &signature,
        &ParameterBindings {
            receiver: Some(receiver),
            parameters,
            ..Default::default()
        },
        r,
    )
}

fn source(result: SourceResult) -> String {
    match result {
        SourceResult::Complete { source, .. } => source,
        other => panic!("expected complete super invocation: {other:?}"),
    }
}

#[test]
fn selected_signature_flags_and_throws_ignore_symbolic_metadata() {
    let mut r = Resolver::fixture();
    let symbolic = r.symbolic.as_mut().unwrap();
    symbolic.name = "symbolicOnly".into();
    symbolic.is_static = true;
    symbolic.return_type = JavaType::Void;
    symbolic.throws = vec!["java.io.IOException".into()];
    r.symbolic_flags = Some(8 | 2 | 0x400);
    let text = source(emit(InvokeKind::Super, false, vec![], &r));
    assert!(
        text.contains("super.choose((java.lang.Object) p0)"),
        "{text}"
    );
    assert!(!text.contains("symbolicOnly"));
    r.symbolic = None;
    r.symbolic_flags = None;
    assert!(matches!(
        emit(InvokeKind::Super, false, vec![], &r),
        SourceResult::Complete { .. }
    ));
}

#[test]
fn selected_checked_exceptions_are_required() {
    let mut r = Resolver::fixture();
    r.selected.as_mut().unwrap().method.throws = vec!["java.io.IOException".into()];
    assert!(matches!(
        emit(InvokeKind::Super, false, vec![], &r),
        SourceResult::Unsupported(Unsupported::Metadata(_))
    ));
    assert!(matches!(
        emit(
            InvokeKind::Super,
            false,
            vec!["java.io.IOException".into()],
            &r
        ),
        SourceResult::Complete { .. }
    ));
}

#[test]
fn selected_invalid_access_and_staticness_fail_closed() {
    for flags in [2, 8, 0x40, 0x80, 0x400] {
        let mut r = Resolver::fixture();
        r.selected.as_mut().unwrap().access_flags = flags;
        assert!(
            matches!(
                emit(InvokeKind::Super, false, vec![], &r),
                SourceResult::Unsupported(_)
            ),
            "flags={flags}"
        );
    }
    let mut r = Resolver::fixture();
    r.selected.as_mut().unwrap().method.is_static = true;
    assert!(matches!(
        emit(InvokeKind::Super, false, vec![], &r),
        SourceResult::Unsupported(_)
    ));
}

#[test]
fn wrong_receiver_and_missing_binding_fail_closed() {
    let mut r = Resolver::fixture();
    assert!(matches!(
        emit(InvokeKind::Super, true, vec![], &r),
        SourceResult::Unsupported(_)
    ));
    r.selected = None;
    assert!(matches!(
        emit(InvokeKind::Super, false, vec![], &r),
        SourceResult::Unsupported(Unsupported::Metadata(_))
    ));
}

#[test]
fn ordinary_virtual_reference_keeps_symbolic_signature_and_throws() {
    let mut r = Resolver::fixture();
    r.symbolic.as_mut().unwrap().name = "symbolicOnly".into();
    let text = source(emit(InvokeKind::Virtual, false, vec![], &r));
    assert!(text.contains("symbolicOnly(p0)"), "{text}");
    assert!(!text.contains("super."));
    r.symbolic.as_mut().unwrap().throws = vec!["java.io.IOException".into()];
    assert!(matches!(
        emit(InvokeKind::Virtual, false, vec![], &r),
        SourceResult::Unsupported(Unsupported::Metadata(_))
    ));
}

#[test]
fn legacy_default_requires_both_signature_and_flags() {
    struct Legacy(Resolver);
    impl JavaResolver for Legacy {
        fn java_super_selects_dex_target(&self, _: &str, _: MethodRef) -> bool {
            true
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            self.0.method(r)
        }
        fn method_access(&self, r: MethodRef) -> Option<u32> {
            self.0.method_access(r)
        }
        fn field(&self, r: FieldRef) -> Option<JavaField> {
            self.0.field(r)
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            self.0.class(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            self.0.string(r)
        }
    }
    let mut r = Legacy(Resolver::fixture());
    assert!(
        r.super_invocation("SuperBindingHarness.Child", MethodRef::new(0))
            .is_some()
    );
    r.0.symbolic_flags = None;
    assert!(
        r.super_invocation("SuperBindingHarness.Child", MethodRef::new(0))
            .is_none()
    );
    r.0.symbolic_flags = Some(1);
    r.0.symbolic = None;
    assert!(
        r.super_invocation("SuperBindingHarness.Child", MethodRef::new(0))
            .is_none()
    );
}

#[test]
fn compiled_super_uses_parent_implementation_and_selected_overload() {
    let mut r = Resolver::fixture();
    r.symbolic.as_mut().unwrap().parameters = vec![JavaType::Class("java.lang.Object".into())];
    let super_source = source(emit(InvokeKind::Super, false, vec![], &r));
    let virtual_source =
        source(emit(InvokeKind::Virtual, false, vec![], &r)).replace(" call(", " virtualCall(");
    let harness = format!(
        r#"public class SuperBindingHarness {{
        static class Base {{ public int choose(Object x) {{ return 1; }} }}
        static class Mid extends Base {{
            public int choose(Object x) {{ return 3; }}
            public int choose(String x) {{ return 5; }}
        }}
        static class Child extends Mid {{
            public int choose(Object x) {{ return 9; }}
            public int choose(String x) {{ return 11; }}
            {super_source}
            {virtual_source}
        }}
        public static void main(String[] args) {{
            Child c = new Child();
            System.out.print(c.call("value") + ":" + c.virtualCall("value"));
        }}
    }}"#
    );
    let dir = std::env::temp_dir().join(format!("decomphify-super-binding-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SuperBindingHarness.java"), harness).unwrap();
    let jdk =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let compile = std::process::Command::new(format!("{jdk}/bin/javac"))
        .args(["--release", "17", "SuperBindingHarness.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(format!("{jdk}/bin/java"))
        .args(["-cp", ".", "SuperBindingHarness"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3:9");
    std::fs::remove_dir_all(dir).unwrap();
}
