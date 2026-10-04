//! Exact-target Throwable whole-body proof; no per-site or unknown-target permission.
#![allow(clippy::unwrap_used)]
use backend_java::{
    DeclarationBindings, JavaField, JavaMethod, JavaResolver, JavaType, SourceResult,
    checked_source_bridge::{self, BoundTarget, FieldThrowDefinition},
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::{Constant, InvokeKind},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct R {
    hierarchy: bool,
}
fn target() -> JavaMethod {
    JavaMethod {
        owner: "cq.m0$type".into(),
        name: "b".into(),
        parameters: vec![JavaType::Class("java.lang.Object".into())],
        return_type: JavaType::Void,
        is_static: true,
        throws: vec!["java.lang.Throwable".into()],
    }
}
impl JavaResolver for R {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        (r.index() == 37112).then(target)
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
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
            || (self.hierarchy
                && a == &JavaType::Class("java.lang.Throwable".into())
                && b == &JavaType::Class("java.lang.Object".into()))
    }
}
fn definition() -> FieldThrowDefinition {
    FieldThrowDefinition {
        original_identity: "classes3.dex#43295 Lcq/m0;->b(Ljava/lang/Object;)V".into(),
        code_offset: 4697940,
        access_flags: 25,
        registers: 2,
        incoming: 1,
        words: vec![4128, 7886, 57, 3, 14, 287, 7886, 4436, 54550, 295],
        handlers: 0,
        original_throws: vec![],
        checked_type_index: 7886,
        field_index: 54550,
        checked_type: "Lcq/l0$b;".into(),
        cast_type: "Lcq/l0$b;".into(),
        field_owner: "Lcq/l0$b;".into(),
        field_type: "Ljava/lang/Throwable;".into(),
    }
}
fn fixture() -> (MethodIr<SsaForm>, JavaMethod) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut parameter = None;
    let first = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Null));
        parameter = Some(result);
        ManagedInsn::Const {
            result,
            value: Constant::Null,
        }
    });
    let call = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        method: MethodRef::new(37112),
        result: None,
        args: vec![parameter.unwrap()],
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(first);
    let c = cfg.add_block(call);
    let r = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), c, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(c, r, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        JavaMethod {
            owner: "Boundary".into(),
            name: "invokeSuspend".into(),
            parameters: vec![],
            return_type: JavaType::Void,
            is_static: false,
            throws: vec![],
        },
    )
}
#[test]
fn exact_raw_body_bound_target_enables_only_outer_boundary() {
    let r = R { hierarchy: true };
    let d = definition();
    let proof = BoundTarget::field_throw(MethodRef::new(37112), target(), &d, &r).unwrap();
    assert_eq!(proof.code_offset(), 4697940);
    let (ir, caller) = fixture();
    assert!(checked_source_bridge::plan_source_contract(&ir, &caller, None, "bridge", &r).is_err());
    assert!(checked_source_bridge::plan_whole_body(&ir, &caller, None, "bridge", &r).is_err());
    let p = checked_source_bridge::plan_whole_body_bound_targets(
        &ir,
        &caller,
        None,
        "bridge",
        &r,
        &[proof],
    )
    .unwrap()
    .unwrap();
    assert!(p.original().throws.is_empty());
    assert!(p.is_whole_body());
    let SourceResult::Complete { source, .. } =
        checked_source_bridge::emit(&ir, &DeclarationBindings::default(), &r, &p)
    else {
        panic!("emission")
    };
    assert!(source.contains("catch (java.lang.Throwable"));
    assert!(!source.contains("throws java.lang.Throwable"));
    assert!(source.contains("cq.m0$type.b"));
}
#[test]
fn target_ref_and_source_contract_are_exact() {
    let r = R { hierarchy: true };
    let d = definition();
    let (ir, caller) = fixture();
    let proof = BoundTarget::field_throw(MethodRef::new(999), target(), &d, &r).unwrap();
    assert!(
        checked_source_bridge::plan_whole_body_bound_targets(
            &ir,
            &caller,
            None,
            "bridge",
            &r,
            &[proof]
        )
        .is_err()
    );
    let mut method = target();
    method.throws.clear();
    assert!(BoundTarget::field_throw(MethodRef::new(37112), method, &d, &r).is_err());
}
#[test]
fn missing_hierarchy_effects_handlers_and_encoded_throws_reject() {
    let r = R { hierarchy: true };
    assert!(
        BoundTarget::field_throw(
            MethodRef::new(37112),
            target(),
            &definition(),
            &R { hierarchy: false }
        )
        .is_err()
    );
    for mode in 0..4 {
        let mut d = definition();
        match mode {
            0 => d.words.insert(9, 0),
            1 => d.handlers = 1,
            2 => d.original_throws.push("java.lang.Throwable".into()),
            _ => d.field_type = "Ljava/lang/Object;".into(),
        };
        assert!(BoundTarget::field_throw(MethodRef::new(37112), target(), &d, &r).is_err());
    }
}
