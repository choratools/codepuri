//! Original-domain witnesses preserve ABI, Throwable identity and body order.
#![allow(clippy::unwrap_used)]
use backend_java::{
    DeclarationBindings, JavaField, JavaMethod, JavaResolver, JavaType, SourceResult,
    checked_source_bridge::{self, BoundTarget, RuntimeThrowDefinition},
    type_compatibility::{Compatibility, Declaration, Hierarchy},
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

struct Resolver {
    hierarchy: Hierarchy,
}
fn target() -> JavaMethod {
    JavaMethod {
        owner: "RuntimeFixture".into(),
        name: "invoke".into(),
        is_static: true,
        parameters: vec![JavaType::Int],
        return_type: JavaType::Int,
        throws: vec!["java.io.IOException".into(), "java.lang.Throwable".into()],
    }
}
impl JavaResolver for Resolver {
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        if reference.index() == 1 {
            Some(target())
        } else {
            Some(JavaMethod {
                owner: "RuntimeFixture".into(),
                name: if reference.index() == 0 {
                    "before"
                } else {
                    "after"
                }
                .into(),
                is_static: true,
                parameters: vec![JavaType::Int],
                return_type: JavaType::Int,
                throws: vec![],
            })
        }
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, source: &JavaType, target: &JavaType) -> bool {
        self.hierarchy.assignable(source, target) == Compatibility::Assignable
    }
}
fn resolver() -> Resolver {
    let mut hierarchy = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.io.IOException", Some("java.lang.Exception")),
    ] {
        hierarchy
            .insert(Declaration {
                name: name.into(),
                superclass: parent.map(Into::into),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
    }
    Resolver { hierarchy }
}
fn definition() -> RuntimeThrowDefinition {
    RuntimeThrowDefinition {
        original_identity: "classes2.dex#40 LRuntimeFixture;->invoke(I)I".into(),
        code_offset: 120,
        access_flags: 9,
        original_throws: vec![],
        escaping: vec!["java.io.IOException".into(), "java.lang.Throwable".into()],
    }
}
fn fixture() -> (MethodIr<SsaForm>, JavaMethod, DeclarationBindings) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut parameter = None;
    let initial = insns.insert_with_key(|id| {
        let value = values.insert(Value::new(id, ManagedType::Int));
        parameter = Some(value);
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![value],
            effects: EffectFlags::NONE,
        }
    });
    let mut input = parameter.unwrap();
    let mut ids = Vec::new();
    for index in 0..3 {
        let id = insns.insert_with_key(|id| {
            let result = values.insert(Value::new(id, ManagedType::Int));
            let insn = ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                method: MethodRef::new(index),
                args: vec![input],
                result: Some(result),
            };
            input = result;
            insn
        });
        ids.push(id);
    }
    ids.push(insns.insert(ManagedInsn::Return { value: Some(input) }));
    let mut cfg = Cfg::new(ids[0]);
    let mut previous = cfg.entry();
    for id in &ids[1..] {
        let block = cfg.add_block(*id);
        cfg.add_edge(previous, block, EdgeKind::Unconditional)
            .unwrap();
        previous = block;
    }
    let method = JavaMethod {
        owner: "Boundary".into(),
        name: "onCreate".into(),
        is_static: true,
        parameters: vec![JavaType::Int],
        return_type: JavaType::Int,
        throws: vec![],
    };
    let bindings = DeclarationBindings {
        receiver: None,
        parameters: vec![parameter],
        names: vec!["value".into()],
    };
    assert!(insns.get(initial).is_some());
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        method,
        bindings,
    )
}

#[test]
fn mixed_throwable_contract_requires_exact_witness_and_preserves_identity_and_order() {
    let resolver = resolver();
    let (ir, original, bindings) = fixture();
    assert!(
        checked_source_bridge::plan_whole_body(&ir, &original, None, "identity", &resolver)
            .is_err()
    );
    let proof =
        BoundTarget::runtime_throwable(MethodRef::new(1), target(), &definition(), &resolver)
            .unwrap();
    let plan = checked_source_bridge::plan_whole_body_bound_targets(
        &ir,
        &original,
        None,
        "identity",
        &resolver,
        &[proof],
    )
    .unwrap()
    .unwrap();
    assert!(plan.original().throws.is_empty());
    assert_eq!(plan.runtime_escaping(), None);
    let SourceResult::Complete { source, .. } =
        checked_source_bridge::emit(&ir, &bindings, &resolver, &plan)
    else {
        panic!("emission failed")
    };
    assert!(source.contains("catch (java.lang.Throwable"));
    assert!(!source.contains("onCreate(int value) throws"));
    let java = format!(
        r#"class Parent {{ public static int onCreate(int value) {{ return value; }} }}
public class Boundary extends Parent {{
{source}
{helper}
public static void main(String[] args) throws Throwable {{
  for (Throwable selected : new Throwable[] {{ null, new java.io.IOException(), new AssertionError(), new Throwable(), new RuntimeException() }}) {{
    RuntimeFixture.failure=selected; RuntimeFixture.trace="";
    Throwable expectedFailure=null; int expected=0;
    try {{ expected=RuntimeFixture.reference(9); }} catch(Throwable failure) {{ expectedFailure=failure; }}
    String expectedTrace=RuntimeFixture.trace; RuntimeFixture.trace="";
    Throwable actualFailure=null; int actual=0;
    try {{ actual=onCreate(9); }} catch(Throwable failure) {{ actualFailure=failure; }}
    if(actual!=expected || actualFailure!=expectedFailure || !RuntimeFixture.trace.equals(expectedTrace)) throw new AssertionError();
  }}
}}
}}
class RuntimeFixture {{
static Throwable failure; static String trace="";
static int before(int value) {{ trace+="B"; return value+1; }}
static int invoke(int value) throws java.io.IOException, Throwable {{ trace+="I"; if(failure!=null)throw failure; return value*2; }}
static int after(int value) {{ trace+="A"; return value+3; }}
static int reference(int value) throws Throwable {{ return after(invoke(before(value))); }}
}}"#,
        helper = plan.helper_source()
    );
    let root = std::env::temp_dir().join(format!("codepuri-runtime-target-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Boundary.java"), java).unwrap();
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Boundary.java"]),
        ("java", vec!["-cp", ".", "Boundary"]),
    ] {
        let result = std::process::Command::new(tool)
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn runtime_witness_rejects_contract_drift_missing_hierarchy_and_nonfinal_virtual_targets() {
    let r = resolver();
    let (ir, original, _) = fixture();
    let wrong =
        BoundTarget::runtime_throwable(MethodRef::new(7), target(), &definition(), &r).unwrap();
    assert!(
        checked_source_bridge::plan_whole_body_bound_targets(
            &ir,
            &original,
            None,
            "identity",
            &r,
            &[wrong]
        )
        .is_err()
    );
    let mut altered = definition();
    altered.escaping.clear();
    assert!(BoundTarget::runtime_throwable(MethodRef::new(1), target(), &altered, &r).is_err());
    altered = definition();
    altered.original_throws.push("java.lang.Exception".into());
    assert!(BoundTarget::runtime_throwable(MethodRef::new(1), target(), &altered, &r).is_err());
    assert!(
        BoundTarget::runtime_throwable(
            MethodRef::new(1),
            target(),
            &definition(),
            &Resolver {
                hierarchy: Hierarchy::default()
            }
        )
        .is_err()
    );
    let mut virtual_target = target();
    virtual_target.is_static = false;
    let mut virtual_definition = definition();
    virtual_definition.access_flags = 1;
    assert!(
        BoundTarget::runtime_throwable(
            MethodRef::new(1),
            virtual_target.clone(),
            &virtual_definition,
            &r
        )
        .is_err()
    );
    virtual_definition.access_flags = 17;
    assert!(
        BoundTarget::runtime_throwable(MethodRef::new(1), virtual_target, &virtual_definition, &r)
            .is_ok()
    );
}
