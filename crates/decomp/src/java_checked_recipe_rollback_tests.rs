//! Checked-recipe failure retains original caught calls and declaration.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::super::exception_dispatch_facade_tests::{classpath, compile_run, install_jdk, raw_dex};
use super::*;
use dex_format::{
    RawSpan,
    code::TypeAddrPair,
    ids::{MethodId, ProtoId, TypeId, TypeList},
    strings::StringData,
};

#[test]
fn caught_dependency_projection_restores_original_call_and_signature() {
    let mut dex = raw_dex();
    let span = RawSpan { offset: 0, size: 0 };
    for text in [
        "Ljava/lang/String;",
        "Ljava/lang/Class;",
        "Ljava/lang/ClassNotFoundException;",
        "forName",
        "length",
    ] {
        dex.strings.push(StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
    }
    for descriptor_idx in [5, 6, 7] {
        dex.type_ids.push(TypeId {
            descriptor_idx,
            span,
        });
    }
    dex.proto_ids[0].return_type_idx = 1;
    dex.proto_ids[0].parameters.as_mut().unwrap().types = vec![4];
    dex.proto_ids.push(ProtoId {
        shorty_idx: 1,
        return_type_idx: 5,
        parameters_off: 1,
        parameters: Some(TypeList {
            types: vec![4],
            span,
        }),
        span,
    });
    dex.method_ids.push(MethodId {
        class_idx: 5,
        proto_idx: 1,
        name_idx: 8,
        span,
    });
    dex.proto_ids.push(ProtoId {
        shorty_idx: 3,
        return_type_idx: 2,
        parameters_off: 0,
        parameters: None,
        span,
    });
    dex.method_ids.push(MethodId {
        class_idx: 4,
        proto_idx: 2,
        name_idx: 9,
        span,
    });
    let encoded = &mut dex.classes[0].class_data.as_mut().unwrap().direct_methods[0];
    let code = encoded.code.as_mut().unwrap();
    let words = [0x1071u16, 1, 0, 0x106e, 2, 0, 0x000e, 0x000d, 0x000e];
    code.registers_size = 1;
    code.ins_size = 1;
    code.outs_size = 1;
    code.insns_size = words.len() as u32;
    code.insns = words.into_iter().flat_map(u16::to_le_bytes).collect();
    code.tries[0].start_addr = 0;
    code.tries[0].insn_count = 6;
    code.handlers[0].handlers = vec![TypeAddrPair {
        type_idx: 6,
        address: 7,
    }];
    let files = vec![dex];
    let dex = &files[0];
    let aliases = inventory(&files, &mut None).unwrap();
    let external = classpath();
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let mut r = Resolver::new(dex, &aliases, &metadata);
    r.inputs = &files;
    r.external = Some(&external);
    install_jdk(&mut r, &external);
    let mut source_plan = SourceThrowsPlan::default();
    source_plan.unavailable.insert(
        method_key(dex, 0).unwrap(),
        required::ThrowsError::Unknown("fixture unavailable source projection".into()),
    );
    r.source_throws = Some(&source_plan);
    let class = &dex.classes[0];
    let encoded = &class.class_data.as_ref().unwrap().direct_methods[0];
    let ctx = initial_context(&r, encoded).unwrap();
    let ssa =
        dex_decompiler::DexPasses::run_ssa_with(encoded.code.as_ref().unwrap(), &ctx).unwrap();
    let scopes = crate::java_exception_regions::from_dex_dispatch(
        &ssa,
        encoded.code.as_ref().unwrap(),
        &r,
        &r.hierarchy,
    )
    .unwrap();
    let signature = r.method(MethodRef::new(0)).unwrap();
    let bound = declaration_bindings(&ssa, &signature, encoded.code.as_ref().unwrap()).unwrap();
    assert!(
        requirements_with_bridge(ssa.ir(), &signature, &r, false, None).is_err(),
        "the original caller clause alone cannot cover the caught checked callee"
    );
    let caught = scopes.catch_names().collect::<Vec<_>>();
    let projected = dependency_signature_with_catches(ssa.ir(), &signature, &r, &caught);
    assert!(requirements_with_bridge(ssa.ir(), &projected, &r, false, None).is_ok());
    assert!(
        signature.throws.is_empty(),
        "dependency-only proof must not widen the emitted caller clause"
    );
    let ordinary = crate::java_exception_regions::from_dex(
        &ssa,
        encoded.code.as_ref().unwrap(),
        &r,
        &r.hierarchy,
    )
    .unwrap();
    assert!(
        matches!(
            backend_java::exceptions::emit_method_region(
                ssa.ir(),
                &signature,
                &bound,
                &r,
                &ordinary
            ),
            SourceResult::Complete { .. }
        ),
        "original scope-aware source remains independently valid"
    );
    let candidate = method_source(class, encoded, &r, ClassKind::Class, &mut None).unwrap();
    assert!(
        !candidate
            .source
            .contains("throws java.lang.ClassNotFoundException")
    );
    assert!(
        candidate
            .source
            .contains("java.lang.ClassNotFoundException")
    );
    compile_run(
        "checked-recipe-rollback",
        &candidate.source,
        "dispatch(\"java.lang.String\"); dispatch(\"absent.DoesNotExist\"); System.out.print(\"facade-dispatch-ok\");",
    );
}
