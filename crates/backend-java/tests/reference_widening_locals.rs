//! Proven reference copies keep the declared Java static type.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "java.lang.Integer",
                1 => "java.lang.Object",
                _ => "missing.Unknown",
            }
            .into(),
        )
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || (actual == &JavaType::Class("java.lang.Integer".into())
                && expected == &JavaType::Class("java.lang.Object".into()))
    }
}

fn copy(
    actual: ManagedType,
    declared: ManagedType,
    parameter: JavaType,
    returned: JavaType,
) -> SourceResult {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut initial = None;
    insns.insert_with_key(|id| {
        let value = values.insert(Value::new(id, actual));
        initial = Some(value);
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![value],
            effects: managed_ir::effects::EffectFlags::NONE,
        }
    });
    let mut result = None;
    let mov = insns.insert_with_key(|id| {
        let value = values.insert(Value::new(id, declared));
        result = Some(value);
        ManagedInsn::Move {
            result: value,
            value: initial.unwrap(),
        }
    });
    let ret = insns.insert(ManagedInsn::Return { value: result });
    let mut cfg = Cfg::new(mov);
    let end = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), end, EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    backend_java::emit_method(
        &ir,
        &JavaMethod {
            owner: "ReferenceCopy".into(),
            name: "copy".into(),
            parameters: vec![parameter],
            return_type: returned,
            is_static: true,
            throws: vec![],
        },
        &ParameterBindings {
            parameters: vec![initial.unwrap()],
            ..Default::default()
        },
        &Resolver,
    )
}

#[test]
fn proven_integer_copy_declares_object_and_preserves_identity() {
    let result = copy(
        ManagedType::Reference(ClassRef::new(0)),
        ManagedType::Reference(ClassRef::new(1)),
        JavaType::Class("java.lang.Integer".into()),
        JavaType::Class("java.lang.Object".into()),
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(source.contains("java.lang.Object v0 = p0;"), "{source}");
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let dir = std::env::temp_dir().join(format!("reference-copy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ReferenceCopy.java"), format!("public class ReferenceCopy {{ {source} public static void main(String[] a) {{ Integer x=1000; if(copy(x)!=x || copy(null)!=null) throw new AssertionError(); }} }}")).unwrap();
    for (tool, args) in [
        ("javac", vec!["--release", "17", "ReferenceCopy.java"]),
        ("java", vec!["-cp", ".", "ReferenceCopy"]),
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
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_hierarchy_and_numeric_reinterpretation_remain_rejected() {
    assert!(matches!(
        copy(
            ManagedType::Reference(ClassRef::new(2)),
            ManagedType::Reference(ClassRef::new(1)),
            JavaType::Class("missing.Unknown".into()),
            JavaType::Class("java.lang.Object".into())
        ),
        SourceResult::Unsupported(_)
    ));
    assert!(matches!(
        copy(
            ManagedType::Int,
            ManagedType::Long,
            JavaType::Int,
            JavaType::Long
        ),
        SourceResult::Unsupported(_)
    ));
}
