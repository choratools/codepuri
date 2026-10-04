//! Immutable rendering-cache equivalence and bounded CPU experiment.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{JavaField, JavaType};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    method::{InsnArena, ValueArena},
    refs::{ClassRef, FieldRef, StringRef},
};

struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "CacheFixture".into(),
            name: "target".into(),
            parameters: vec![],
            return_type: JavaType::Void,
            is_static: true,
            throws: vec!["java.io.IOException".into()],
        })
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
            || matches!((a,b),(JavaType::Class(a),JavaType::Class(b)) if a == "java.io.IOException" && b == "java.lang.Exception")
    }
}
fn fixture(count: usize) -> (MethodIr<SsaForm>, Plan) {
    let mut insns = InsnArena::with_key();
    let mut ids: Vec<_> = (0..count)
        .map(|_| {
            insns.insert(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(0),
                args: vec![],
            })
        })
        .collect();
    ids.push(insns.insert(ManagedInsn::Return { value: None }));
    let mut cfg = Cfg::new(ids[0]);
    let mut previous = cfg.entry();
    for id in &ids[1..] {
        let next = cfg.add_block(*id);
        cfg.add_edge(previous, next, EdgeKind::Unconditional)
            .unwrap();
        previous = next;
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, ValueArena::with_key());
    let caller = JavaMethod {
        owner: "CacheFixture".into(),
        name: "caller".into(),
        parameters: vec![],
        return_type: JavaType::Void,
        is_static: true,
        throws: vec![],
    };
    let plan = plan_whole_body(&ir, &caller, None, "bridge", &Resolver)
        .unwrap()
        .unwrap();
    (ir, plan)
}
#[test]
fn cached_internal_signature_is_exact_and_target_drift_still_rejects() {
    let (ir, plan) = fixture(32);
    let legacy = plan.compute_rendering_method();
    assert!(same_method(&legacy, plan.rendering_method()));
    assert!(plan.caller.throws.is_empty());
    assert_eq!(plan.runtime_escaping(), None);
    let bindings = DeclarationBindings::default();
    let cached = emit(&ir, &bindings, &Resolver, &plan);
    let reconstructed = crate::emit_method_declaration(
        &ir,
        &legacy,
        &bindings,
        &Local {
            inner: &Resolver,
            plan: &plan,
        },
    );
    match (cached, reconstructed) {
        (SourceResult::Complete { source: a, .. }, SourceResult::Complete { source: b, .. }) => {
            assert_eq!(a, b);
            assert_eq!(a.matches("CacheFixture.target(").count(), 32);
            assert!(!a.lines().next().unwrap().contains(" throws "));
        }
        pair => panic!("unexpected rendering result: {pair:?}"),
    }
    let mut changed = plan.clone();
    changed
        .resolved
        .values_mut()
        .next()
        .unwrap()
        .target
        .throws
        .clear();
    assert!(matches!(
        emit(&ir, &bindings, &Resolver, &changed),
        SourceResult::Unsupported(_)
    ));
}
#[test]
#[ignore = "bounded CPU experiment, no APK speed claim"]
fn bounded_cached_rendering_cpu_comparison() {
    use std::{hint::black_box, time::Instant};
    let (_, plan) = fixture(128);
    let rendering = plan.compute_rendering_method();
    let iterations = 2000;
    let start = Instant::now();
    for _ in 0..iterations {
        assert!(same_method(
            black_box(&plan.compute_rendering_method()),
            black_box(&rendering)
        ));
    }
    let old = start.elapsed();
    let start = Instant::now();
    for _ in 0..iterations {
        assert!(
            black_box(&plan)
                .body_original(black_box(&rendering))
                .is_some()
        );
    }
    let cached = start.elapsed();
    println!(
        "rendering-cache calls=128 iterations={iterations} old_ns={} cached_ns={} ratio={:.2}",
        old.as_nanos(),
        cached.as_nanos(),
        old.as_secs_f64() / cached.as_secs_f64()
    );
}
