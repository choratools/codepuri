//! Comparison uses preserve DEX zero interpretation and reference identity.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::{CompareOp, Constant},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "java.lang.String"
            } else {
                "java.lang.Integer"
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn emit(name: &str, ty: ManagedType, source: JavaType, op: CompareOp) -> SourceResult {
    emit_constant(name, ty, source, op, Constant::Zero)
}
fn emit_constant(
    name: &str,
    ty: ManagedType,
    source: JavaType,
    op: CompareOp,
    constant: Constant,
) -> SourceResult {
    let mut instructions = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut param = None;
    let _initial = instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ty));
        param = Some(v);
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: managed_ir::effects::EffectFlags::NONE,
        }
    });
    let mut zero = None;
    let z = instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Int));
        zero = Some(v);
        ManagedInsn::Const {
            result: v,
            value: constant,
        }
    });
    let mut result = None;
    let c = instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Boolean));
        result = Some(v);
        ManagedInsn::Compare {
            op,
            result: v,
            lhs: param.unwrap(),
            rhs: zero.unwrap(),
        }
    });
    let ret = instructions.insert(ManagedInsn::Return { value: result });
    let mut cfg = Cfg::new(z);
    let mut prev = cfg.entry();
    for id in [c, ret] {
        let next = cfg.add_block(id);
        cfg.add_edge(prev, next, EdgeKind::Unconditional).unwrap();
        prev = next;
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, values);
    backend_java::emit_method(
        &ir,
        &JavaMethod {
            owner: "CompareContext".into(),
            name: name.into(),
            parameters: vec![source],
            return_type: JavaType::Boolean,
            is_static: true,
            throws: vec![],
        },
        &ParameterBindings {
            receiver: None,
            parameters: vec![param.unwrap()],
            names: vec![],
        },
        &Resolver,
    )
}
#[test]
fn reference_and_boolean_zero_comparisons_compile_and_run() {
    let mut methods = String::new();
    for (name, ty, java) in [
        (
            "reference",
            ManagedType::Reference(ClassRef::new(0)),
            JavaType::Class("java.lang.String".into()),
        ),
        ("bool", ManagedType::Boolean, JavaType::Boolean),
    ] {
        let SourceResult::Complete { source, .. } = emit(name, ty, java, CompareOp::NotEqual)
        else {
            panic!("comparison rejected")
        };
        methods.push_str(&source);
    }
    let SourceResult::Complete { source, .. } = emit_constant(
        "boolOne",
        ManagedType::Boolean,
        JavaType::Boolean,
        CompareOp::NotEqual,
        Constant::Int(1),
    ) else {
        panic!("boolean/integer comparison rejected")
    };
    methods.push_str(&source);
    let SourceResult::Complete { source, .. } = emit(
        "narrow",
        ManagedType::Byte,
        JavaType::Byte,
        CompareOp::LessThan,
    ) else {
        panic!("narrow comparison rejected")
    };
    methods.push_str(&source);
    let dir = std::env::temp_dir().join(format!("compare-context-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("CompareContext.java");
    std::fs::write(&file,format!("public class CompareContext {{{methods} public static void main(String[] a){{if(reference(null)||!reference(\"x\")||bool(false)||!bool(true)||boolOne(true)||!boolOne(false)||!narrow((byte)-1)||narrow((byte)0))throw new AssertionError();}} }}")).unwrap();
    let c = std::process::Command::new("javac")
        .arg(&file)
        .output()
        .unwrap();
    assert!(c.status.success(), "{}", String::from_utf8_lossy(&c.stderr));
    let r = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("CompareContext")
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}
#[test]
fn reference_ordering_still_rejects() {
    assert!(matches!(
        emit(
            "bad",
            ManagedType::Reference(ClassRef::new(0)),
            JavaType::Class("java.lang.String".into()),
            CompareOp::LessThan
        ),
        SourceResult::Unsupported(_)
    ));
}
