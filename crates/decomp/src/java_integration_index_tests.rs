#![allow(clippy::unwrap_used)]
use super::*;
#[path = "java_alias_access_integration_tests.rs"]
mod alias_access_tests;
#[path = "java_contextual_original_tests.rs"]
mod contextual_original_tests;
#[allow(dead_code)]
#[path = "../../dex-format/tests/common/mod.rs"]
pub(super) mod fixture;
#[path = "java_wave7_type_original_tests.rs"]
mod wave7_type_original_tests;
#[path = "java_wave8_contextual_original_tests.rs"]
mod wave8_contextual_original_tests;
#[path = "java_wave9_contextual_loss_original_tests.rs"]
mod wave9_contextual_loss_original_tests;

fn baseline(
    mut graph: BTreeMap<u32, (bool, MemberRequirements)>,
    fields: &BTreeSet<u32>,
) -> BTreeMap<u32, String> {
    let mut failures = BTreeMap::new();
    loop {
        let complete = graph.iter().map(|(&i, _)| i).collect();
        let rejected: Vec<_> = graph
            .iter()
            .filter_map(|(&i, (_, r))| r.check_closure(&complete, fields).err().map(|e| (i, e)))
            .collect();
        if rejected.is_empty() {
            return failures;
        }
        for (i, reason) in rejected {
            graph.remove(&i);
            failures.insert(i, reason);
        }
    }
}
fn compare(graph: BTreeMap<u32, (bool, MemberRequirements)>, fields: BTreeSet<u32>) {
    let actual = closure_failures(graph.iter().map(|(&i, (body, r))| (i, *body, r)), &fields);
    assert_eq!(actual, baseline(graph, &fields));
}
fn req(
    methods: impl IntoIterator<Item = u32>,
    fields: impl IntoIterator<Item = u32>,
) -> MemberRequirements {
    MemberRequirements {
        methods: methods.into_iter().collect(),
        fields: fields.into_iter().collect(),
    }
}

#[test]
fn closure_chain_and_recursive_cycles_match_baseline() {
    for n in [1, 2, 32, 1024] {
        compare(
            (0..n).map(|i| (i, (true, req([i + 1], [])))).collect(),
            BTreeSet::new(),
        );
        compare(
            (0..n)
                .map(|i| (i, (true, req([(i + 1) % n], []))))
                .collect(),
            BTreeSet::new(),
        );
        compare(
            (0..n)
                .map(|i| {
                    (
                        i,
                        (
                            true,
                            req([(i + 1) % n], if i == n - 1 { vec![7] } else { vec![] }),
                        ),
                    )
                })
                .collect(),
            BTreeSet::new(),
        );
    }
}

#[test]
fn closure_exhaustive_three_node_graphs_preserve_reasons() {
    for edges in 0..512u32 {
        for bodies in 0..8u32 {
            for missing_fields in 0..8u32 {
                let graph = (0..3)
                    .map(|i| {
                        (
                            i,
                            (
                                bodies & (1 << i) != 0,
                                req(
                                    (0..3).filter(|j| edges & (1 << (i * 3 + j)) != 0),
                                    if missing_fields & (1 << i) != 0 {
                                        vec![9]
                                    } else {
                                        vec![]
                                    },
                                ),
                            ),
                        )
                    })
                    .collect();
                compare(graph, BTreeSet::new());
            }
        }
    }
}

fn small_dex() -> DexFile {
    use dex_format::{RawSpan, class::ClassData, ids::*, strings::StringData};
    let span = RawSpan { offset: 0, size: 0 };
    let mut dex = DexFile::from_bytes(&fixture::minimal_dex()).unwrap();
    for text in [
        "V",
        "first",
        "second",
        "Ldalvik/annotation/Throws;",
        "value",
        "Ljava/lang/Exception;",
        "field",
        "I",
    ] {
        dex.strings.push(StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
    }
    for descriptor_idx in [1, 4, 6, 8] {
        dex.type_ids.push(TypeId {
            descriptor_idx,
            span,
        });
    }
    dex.proto_ids.push(ProtoId {
        shorty_idx: 1,
        return_type_idx: 1,
        parameters_off: 0,
        parameters: None,
        span,
    });
    let code = dex_format::code::CodeItem {
        registers_size: 0,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: 1,
        insns: vec![0x0e, 0],
        tries: vec![],
        handlers: vec![],
        span,
    };
    let mut methods = Vec::new();
    for name_idx in [2, 3] {
        let method_idx = dex.method_ids.len() as u32;
        dex.method_ids.push(MethodId {
            class_idx: 0,
            proto_idx: 0,
            name_idx,
            span,
        });
        methods.push(EncodedMethod {
            method_idx,
            access_flags: 9,
            code_off: 1,
            code: Some(code.clone()),
        });
    }
    dex.classes[0].class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: methods,
        virtual_methods: vec![],
        span,
    });
    dex
}

#[test]
fn production_crossdex_publication_uses_defining_pool_and_owner_fields() {
    use dex_format::{
        RawSpan,
        class::EncodedField,
        ids::{FieldId, TypeId},
        strings::StringData,
    };
    let span = RawSpan { offset: 0, size: 0 };
    let mut caller = small_dex();
    caller.strings[0].value = "LCaller;".into();
    caller.strings.push(StringData {
        value: "LTarget;".into(),
        utf16_size: 8,
        span,
    });
    caller.type_ids.push(TypeId {
        descriptor_idx: 9,
        span,
    });
    caller.method_ids[1].class_idx = 5;
    caller.field_ids.push(FieldId {
        class_idx: 5,
        type_idx: 4,
        name_idx: 7,
        span,
    });
    caller.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .truncate(1);
    let code = caller.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    code.registers_size = 1;
    code.insns = [0x0060u16, 0, 0x0071, 1, 0, 0x000e]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 6;
    let mut target = small_dex();
    target.strings[0].value = "LTarget;".into();
    target.strings[2].value = "second".into();
    target.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .truncate(1);
    target.field_ids.push(FieldId {
        class_idx: 0,
        type_idx: 4,
        name_idx: 7,
        span,
    });
    target.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .static_fields
        .push(EncodedField {
            field_idx: 0,
            access_flags: 9,
        });
    let files = [caller, target];
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &files,
        &JavaOptions::default(),
        &mut |s: &GeneratedSource| {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 2, "{:?}", report.reasons);
    assert_eq!(sources.len(), 2);
    let caller = sources
        .iter()
        .find(|s| s.provenance.descriptor == "LCaller;")
        .unwrap();
    let target = sources
        .iter()
        .find(|s| s.provenance.descriptor == "LTarget;")
        .unwrap();
    assert!(caller.text.contains("Target.field"), "{}", caller.text);
    assert!(caller.text.contains("Target.second()"), "{}", caller.text);
    assert!(
        !caller.text.contains("Global method source closure"),
        "{}",
        caller.text
    );
    assert!(
        target.text.contains("public static int field;"),
        "{}",
        target.text
    );
    assert!(target.text.contains("original method pool index 0"));
    let root = std::env::temp_dir().join(format!(
        "production-crossdex-publication-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let paths: Vec<_> = sources
        .iter()
        .map(|s| {
            let p = root.join(&s.relative_path);
            std::fs::write(&p, &s.text).unwrap();
            p
        })
        .collect();
    let compile = std::process::Command::new("javac")
        .arg("-proc:none")
        .args(&paths)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let mut limited = Vec::new();
    crate::dex_java::emit_java(
        &files,
        &JavaOptions {
            limit: Some(1),
            workers: 2,
            ..Default::default()
        },
        &mut |s: &GeneratedSource| {
            limited.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(limited.len(), 1);
    assert!(limited[0].text.contains("Global method source closure"));
    assert!(limited[0].text.contains("SourceSubsetAbsent"));
    let mut missing = files;
    missing[1].classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[0]
        .code = None;
    missing[1].classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[0]
        .code_off = 0;
    let mut partial = Vec::new();
    crate::dex_java::emit_java(
        &missing,
        &JavaOptions::default(),
        &mut |s: &GeneratedSource| {
            partial.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(partial.len(), 2, "field-only owner must remain published");
    let caller = partial
        .iter()
        .find(|s| s.provenance.descriptor == "LCaller;")
        .unwrap();
    assert!(caller.text.contains("Target.second()"));
    assert!(caller.text.contains("Global method source closure"));
    assert!(
        !partial
            .iter()
            .find(|s| s.provenance.descriptor == "LTarget;")
            .unwrap()
            .text
            .contains("void second(")
    );
}

#[test]
fn production_inherited_android_field_uses_actual_owner_and_exact_jar() {
    use dex_format::{
        RawSpan,
        ids::{FieldId, TypeId},
        strings::StringData,
    };
    let mut dex = small_dex();
    let span = RawSpan { offset: 0, size: 0 };
    let name = "Landroid/widget/LinearLayout;";
    dex.strings.push(StringData {
        value: name.into(),
        utf16_size: name.len() as u32,
        span,
    });
    dex.type_ids.push(TypeId {
        descriptor_idx: 9,
        span,
    });
    dex.strings[7].value = "VISIBLE".into();
    dex.field_ids.push(FieldId {
        class_idx: 5,
        type_idx: 4,
        name_idx: 7,
        span,
    });
    let code = dex.classes[0].class_data.as_mut().unwrap().direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    code.registers_size = 1;
    code.insns = [0x0060u16, 0, 0x000e]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 3;
    let android =
        std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar");
    let jdk = std::env::var("JAVA_HOME").unwrap();
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &[dex],
        &JavaOptions {
            classpath: vec![
                std::path::PathBuf::from(&jdk).join("jmods/java.base.jmod"),
                android.clone(),
            ],
            ..Default::default()
        },
        &mut |s: &GeneratedSource| {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 2, "{:?}", report.reasons);
    assert!(
        sources[0].text.contains("android.view.View.VISIBLE"),
        "{}",
        sources[0].text
    );
    assert!(
        !sources[0]
            .text
            .contains("android.widget.LinearLayout.VISIBLE")
    );
    let root = std::env::temp_dir().join(format!(
        "production-inherited-android-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(&sources[0].relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &sources[0].text).unwrap();
    let compiled = std::process::Command::new(format!("{jdk}/bin/javac"))
        .arg("-classpath")
        .arg(android)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
}

#[test]
fn protected_receiver_proof_uses_original_access_and_exact_metadata() {
    use dex_format::{RawSpan, ids::TypeId, strings::StringData};
    let mut dex = small_dex();
    dex.strings[0].value = "Lq/Sub;".into();
    let span = RawSpan { offset: 0, size: 0 };
    dex.strings.push(StringData {
        value: "Lp/Base;".into(),
        utf16_size: 8,
        span,
    });
    dex.type_ids.push(TypeId {
        descriptor_idx: 9,
        span,
    });
    dex.method_ids[0].class_idx = 5;
    dex.classes[0].superclass_idx = 5;
    dex.field_ids.push(dex_format::ids::FieldId {
        class_idx: 5,
        type_idx: 4,
        name_idx: 7,
        span,
    });
    let mut inv = AliasInventory::default();
    for ty in &dex.type_ids {
        inv.add_type(dex.string(ty.descriptor_idx).unwrap())
            .unwrap();
    }
    for i in 0..dex.method_ids.len() {
        inv.add_method(method_key(&dex, i as u32).unwrap()).unwrap();
    }
    let mut overload = method_key(&dex, 0).unwrap();
    overload.owner = "Lq/Sub;".into();
    overload.parameters.push("I".into());
    inv.add_method(overload.clone()).unwrap();
    let mut override_key = method_key(&dex, 0).unwrap();
    override_key.owner = "Lq/Sub;".into();
    inv.add_method(override_key.clone()).unwrap();
    inv.add_field(field_key(&dex, 0).unwrap()).unwrap();
    let aliases = inv.build(AliasMode::Ascii);
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let mut r = Resolver::new(&dex, &aliases, &metadata);
    r.methods.insert(0, (4, Some(vec![])));
    r.catalog = method_catalog(std::slice::from_ref(&dex));
    r.catalog.insert(
        method_key(&dex, 0).unwrap(),
        Some(MethodMetadata {
            access_flags: 4,
            exceptions: Some(vec![]),
        }),
    );
    r.input_owners.insert("Lq/Sub;".into());
    r.class_catalog.insert(
        "q.Sub".into(),
        Some(backend_java::constructors::ConstructorClass {
            name: "q.Sub".into(),
            superclass: Some("p.Base".into()),
            access_flags: 1,
        }),
    );
    for declaration in [
        crate::type_compatibility::Declaration {
            name: "q.Sub".into(),
            superclass: Some("p.Base".into()),
            interfaces: vec![],
            is_interface: false,
        },
        crate::type_compatibility::Declaration {
            name: "p.Base".into(),
            superclass: None,
            interfaces: vec![],
            is_interface: false,
        },
    ] {
        r.hierarchy.insert(declaration).unwrap();
    }
    let receiver = JavaType::Class("q.Sub".into());
    assert!(r.retain_method_receiver(MethodRef::new(0), "q.Sub", &receiver));
    assert!(!r.retain_method_receiver(
        MethodRef::new(0),
        "q.Sub",
        &JavaType::Class("p.Base".into())
    ));
    assert!(!r.retain_method_receiver(MethodRef::new(0), "q.Other", &receiver));
    r.catalog.insert(
        overload.clone(),
        Some(MethodMetadata {
            access_flags: 1,
            exceptions: Some(vec![]),
        }),
    );
    assert!(!r.retain_method_receiver(MethodRef::new(0), "q.Sub", &receiver));
    r.catalog.remove(&overload);
    for flags in [2, 8] {
        r.catalog.insert(
            override_key.clone(),
            Some(MethodMetadata {
                access_flags: flags,
                exceptions: Some(vec![]),
            }),
        );
        assert!(!r.retain_method_receiver(MethodRef::new(0), "q.Sub", &receiver));
    }
    r.catalog.insert(
        override_key.clone(),
        Some(MethodMetadata {
            access_flags: 1,
            exceptions: Some(vec![]),
        }),
    );
    assert!(r.retain_method_receiver(MethodRef::new(0), "q.Sub", &receiver));
    r.catalog.remove(&override_key);
    r.fields.insert(0, (4, false));
    r.field_names.insert("q.Sub".into(), BTreeSet::new());
    assert!(r.retain_field_receiver(FieldRef::new(0), "q.Sub", &receiver));
    assert!(!r.retain_field_receiver(FieldRef::new(0), "q.Sub", &JavaType::Class("p.Base".into())));
    r.field_names
        .get_mut("q.Sub")
        .unwrap()
        .insert("field".into());
    assert!(!r.retain_field_receiver(FieldRef::new(0), "q.Sub", &receiver));
    r.original_names.insert("q.Sub".into(), "q.Original".into());
    assert!(!r.retain_method_receiver(MethodRef::new(0), "q.Sub", &receiver));
}
#[test]
fn facade_retains_exact_same_owner_private_static_call() {
    let mut dex = small_dex();
    let methods = &mut dex.classes[0].class_data.as_mut().unwrap().direct_methods;
    methods[1].access_flags = 2 | 8;
    let code = methods[0].code.as_mut().unwrap();
    code.insns = [0x0071u16, 1, 0, 0x000e]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 4;
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &[dex],
        &JavaOptions::default(),
        &mut |s: &GeneratedSource| {
            sources.push(s.text.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 2);
    assert_eq!(report.unsupported_methods, 0);
    assert!(sources[0].contains("private static void second()"));
    assert!(sources[0].contains(".second()"));
}
#[test]
fn facade_native_reference_requires_declaration_not_native_body() {
    let mut dex = small_dex();
    dex.strings[3].value = "class".into();
    let methods = &mut dex.classes[0].class_data.as_mut().unwrap().direct_methods;
    methods[1].access_flags = 1 | 8 | 0x100;
    methods[1].code = None;
    methods[1].code_off = 0;
    let code = methods[0].code.as_mut().unwrap();
    code.insns = [0x0071u16, 1, 0, 0x000e]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 4;
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &[dex],
        &JavaOptions::default(),
        &mut |s: &GeneratedSource| {
            sources.push(s.text.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 1);
    assert_eq!(report.declaration_only_methods, 1);
    assert_eq!(report.unsupported_methods, 0);
    assert!(sources[0].contains("native void "));
    assert!(sources[0].contains("name: \"class\""));
    assert!(
        !sources[0].contains("Global method source closure"),
        "{}",
        sources[0]
    );
}

#[test]
fn analyst_field_inventory_does_not_publish_a_winner_for_three_duplicate_declarations() {
    use dex_format::{RawSpan, class::EncodedField, ids::FieldId};
    let mut dex = small_dex();
    dex.field_ids.push(FieldId {
        class_idx: 0,
        type_idx: 4,
        name_idx: 7,
        span: RawSpan { offset: 0, size: 0 },
    });
    dex.classes[0].class_data.as_mut().unwrap().static_fields = vec![
        EncodedField {
            field_idx: 0,
            access_flags: 9
        };
        3
    ];
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &[dex],
        &JavaOptions::default(),
        &mut |s: &GeneratedSource| {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 2);
    assert_eq!(report.fields_emitted, 0);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].provenance.status, SourceStatus::Fallback);
    assert!(!sources[0].text.contains("int field;"));
    assert!(
        sources[0]
            .text
            .contains("Global field inventory limitations")
    );
    assert!(
        sources[0]
            .text
            .contains("Original field initialization proof: unavailable.")
    );
}

#[test]
fn production_protected_field_receiver_uses_global_source_lookup_across_two_parents() {
    use dex_format::{
        RawSpan,
        class::EncodedField,
        ids::{FieldId, TypeId},
        strings::StringData,
    };
    let span = RawSpan { offset: 0, size: 0 };
    let mut caller = small_dex();
    caller.strings[0].value = "Lq/Caller;".into();
    for owner in ["Lq/Middle;", "Lp/Target;"] {
        let index = caller.strings.len() as u32;
        caller.strings.push(StringData {
            value: owner.into(),
            utf16_size: owner.len() as u32,
            span,
        });
        caller.type_ids.push(TypeId {
            descriptor_idx: index,
            span,
        });
    }
    caller.classes[0].superclass_idx = 5;
    caller.field_ids.push(FieldId {
        class_idx: 6,
        type_idx: 4,
        name_idx: 7,
        span,
    });
    let methods = &mut caller.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods;
    methods.truncate(1);
    methods[0].access_flags = 1;
    let code = methods[0].code.as_mut().unwrap();
    code.registers_size = 2;
    code.ins_size = 1;
    code.insns = [0x1052u16, 0, 0x000e]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 3;
    let mut middle = small_dex();
    middle.strings[0].value = "Lq/Middle;".into();
    middle.strings[2].value = "middleMethod".into();
    middle.strings.push(StringData {
        value: "Lp/Target;".into(),
        utf16_size: 10,
        span,
    });
    middle.type_ids.push(TypeId {
        descriptor_idx: 9,
        span,
    });
    middle.classes[0].superclass_idx = 5;
    middle.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .truncate(1);
    let mut target = small_dex();
    target.strings[0].value = "Lp/Target;".into();
    target.field_ids.push(FieldId {
        class_idx: 0,
        type_idx: 4,
        name_idx: 7,
        span,
    });
    let data = target.classes[0].class_data.as_mut().unwrap();
    data.direct_methods.clear();
    data.instance_fields.push(EncodedField {
        field_idx: 0,
        access_flags: 4,
    });
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &[caller, middle, target],
        &JavaOptions::default(),
        &mut |s: &GeneratedSource| {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 2, "{:?}", report.reasons);
    assert_eq!(sources.len(), 3);
    let caller = sources
        .iter()
        .find(|s| s.provenance.descriptor == "Lq/Caller;")
        .unwrap();
    assert!(caller.text.contains("this.field"), "{}", caller.text);
    assert!(!caller.text.contains("((p.Target)"), "{}", caller.text);
    let root = std::env::temp_dir().join(format!(
        "production-protected-global-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let paths: Vec<_> = sources
        .iter()
        .map(|s| {
            let path = root.join(&s.relative_path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &s.text).unwrap();
            path
        })
        .collect();
    let compiled = std::process::Command::new("javac")
        .arg("-proc:none")
        .args(&paths)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
}
#[test]
fn facade_preserves_same_owner_direct_private_dispatch() {
    let mut dex = small_dex();
    let methods = &mut dex.classes[0].class_data.as_mut().unwrap().direct_methods;
    methods[0].access_flags = 1;
    methods[1].access_flags = 2;
    for method in methods.iter_mut() {
        let code = method.code.as_mut().unwrap();
        code.registers_size = 1;
        code.ins_size = 1;
    }
    let code = methods[0].code.as_mut().unwrap();
    code.insns = [0x1070u16, 1, 0, 0x000e]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 4;
    code.outs_size = 1;
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &[dex],
        &JavaOptions::default(),
        &mut |s: &GeneratedSource| {
            sources.push(s.text.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 2);
    assert_eq!(report.unsupported_methods, 0);
    assert!(sources[0].contains("this.second()"));
    assert!(sources[0].contains("private void second()"));
}

#[test]
fn numeric_instruction_targets_are_exact_and_do_not_alias_dex_pool() {
    let dex = small_dex();
    let aliases = inventory(std::slice::from_ref(&dex), &mut None).unwrap();
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let resolver = Resolver::new(&dex, &aliases, &metadata);
    for (opcode, expected) in [
        (0x80, JavaType::Int),
        (0x81, JavaType::Long),
        (0x82, JavaType::Float),
        (0x83, JavaType::Double),
        (0x84, JavaType::Int),
        (0x85, JavaType::Float),
        (0x86, JavaType::Double),
        (0x87, JavaType::Int),
        (0x88, JavaType::Long),
        (0x89, JavaType::Double),
        (0x8a, JavaType::Int),
        (0x8b, JavaType::Long),
        (0x8c, JavaType::Float),
        (0x8d, JavaType::Byte),
        (0x8e, JavaType::Char),
        (0x8f, JavaType::Short),
    ] {
        assert_eq!(
            resolver.instruction_type(TypeRef::new(0x8000_0000 | opcode)),
            Some(expected)
        );
    }
    assert_eq!(resolver.instruction_type(TypeRef::new(0x8000_0090)), None);
    assert_eq!(resolver.instruction_type(TypeRef::new(u32::MAX)), None);
}

#[test]
fn original_samsung_lost_identity_is_restored() {
    let files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let aliases = inventory(&files, &mut None).unwrap();
    let dex = &files[0];
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == Some("Ld1/g;"))
        .unwrap();
    let method = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 37256)
        .unwrap();
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let mut resolver = Resolver::new(dex, &aliases, &metadata);
    resolver.catalog = method_catalog(&files);
    let signature = resolver.method(MethodRef::new(method.method_idx)).unwrap();
    let context = initial_context(&resolver, method).unwrap();
    let ssa =
        dex_decompiler::DexPasses::run_ssa_with(method.code.as_ref().unwrap(), &context).unwrap();
    let bindings = declaration_bindings(&ssa, &signature, method.code.as_ref().unwrap()).unwrap();
    let result = backend_java::emit_method_declaration(ssa.ir(), &signature, &bindings, &resolver);
    let backend_java::SourceResult::Complete { source, .. } = result else {
        panic!("original Ld1/g;->a(I)J must remain emitted: {result:?}");
    };
    assert!(source.contains("(long)"));
}

#[test]
fn indexed_annotations_match_raw_duplicates_and_malformed_throws() {
    use dex_format::{RawSpan, annotations::*, encoded_value::*};
    let span = RawSpan { offset: 0, size: 0 };
    let valid = AnnotationItem {
        visibility: Visibility::System,
        span,
        annotation: EncodedAnnotation {
            type_idx: 2,
            span,
            elements: vec![AnnotationElement {
                name_idx: 5,
                value: EncodedValue {
                    value: EncodedValueKind::Array(vec![EncodedValue {
                        value: EncodedValueKind::Type(3),
                        span,
                    }]),
                    span,
                },
            }],
        },
    };
    let mut malformed = valid.clone();
    malformed.visibility = Visibility::Runtime;
    let mut invalid_type = valid.clone();
    invalid_type.annotation.type_idx = 999;
    for items in [
        vec![],
        vec![valid.clone()],
        vec![valid.clone(), valid.clone()],
        vec![malformed],
        vec![invalid_type],
    ] {
        let mut dex = small_dex();
        dex.classes[0].annotations = Some(AnnotationsDirectory {
            class_annotations: None,
            field_annotations: vec![],
            parameter_annotations: vec![],
            span,
            method_annotations: items
                .into_iter()
                .map(|item| MemberAnnotations {
                    member_idx: 0,
                    annotations: AnnotationSet {
                        entries: vec![item],
                        span,
                    },
                })
                .collect(),
        });
        let aliases = inventory(std::slice::from_ref(&dex), &mut None).unwrap();
        let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
        let resolver = Resolver::new(&dex, &aliases, &metadata);
        let raw = MemberResolver { dex: &dex };
        for i in 0..2 {
            assert_eq!(
                resolver.methods[&i].1,
                raw.declared_throws_for_class(&dex.classes[0], i)
                    .and_then(|names| names
                        .into_iter()
                        .map(|n| aliases
                            .class_name(&format!("L{};", n.replace('.', "/")))
                            .map(str::to_owned))
                        .collect())
            );
        }
    }
}

#[test]
fn real_facade_small_metadata_preserves_sources_and_counters() {
    let dex = small_dex();
    let run = |workers| {
        let mut sources = Vec::new();
        let report = crate::dex_java::emit_java(
            std::slice::from_ref(&dex),
            &JavaOptions {
                workers,
                ..Default::default()
            },
            &mut |s: &GeneratedSource| {
                sources.push((s.relative_path.clone(), s.text.clone()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(report.classes_emitted, 1);
        assert_eq!(report.complete_methods, 2, "{report:?} {sources:?}");
        assert_eq!(report.unsupported_methods, 0);
        assert!(sources[0].1.contains("void first()"));
        assert!(sources[0].1.contains("void second()"));
        (format!("{report:?}"), sources)
    };
    assert_eq!(run(1), run(4));
}

#[test]
fn static_positions_match_first_position_and_initializer_boundary() {
    use dex_format::{RawSpan, class::EncodedField, encoded_value::*, ids::FieldId};
    let span = RawSpan { offset: 0, size: 0 };
    let mut dex = small_dex();
    for i in 0..64 {
        dex.field_ids.push(FieldId {
            class_idx: 0,
            type_idx: 4,
            name_idx: 7,
            span,
        });
        dex.classes[0]
            .class_data
            .as_mut()
            .unwrap()
            .static_fields
            .push(EncodedField {
                field_idx: i,
                access_flags: 9,
            });
    }
    dex.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .static_fields
        .push(EncodedField {
            field_idx: 0,
            access_flags: 9,
        });
    dex.classes[0].static_values = Some(vec![
        EncodedValue {
            value: EncodedValueKind::Int(0),
            span
        };
        16
    ]);
    let aliases = inventory(std::slice::from_ref(&dex), &mut None).unwrap();
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let resolver = Resolver::new(&dex, &aliases, &metadata);
    for i in 0..64 {
        let position = dex.classes[0]
            .class_data
            .as_ref()
            .unwrap()
            .static_fields
            .iter()
            .position(|f| f.field_idx == i)
            .unwrap();
        assert_eq!(resolver.static_positions[&(0, i)], position);
        let result = resolver.field_declaration(&dex.classes[0], i);
        let declaration = result.unwrap();
        assert!(declaration.contains("static int"));
        assert!(
            !declaration.contains('='),
            "partial dependencies must not invent initializers"
        );
    }
}

#[test]
fn interface_blank_fields_are_unavailable_even_with_implicit_flags() {
    use dex_format::{RawSpan, class::EncodedField, ids::FieldId};
    for flags in [0, 9, 0x19, 0x1c, 0x59] {
        let mut dex = small_dex();
        dex.classes[0].access_flags = 0x601;
        dex.field_ids.push(FieldId {
            class_idx: 0,
            type_idx: 4,
            name_idx: 7,
            span: RawSpan { offset: 0, size: 0 },
        });
        dex.classes[0]
            .class_data
            .as_mut()
            .unwrap()
            .static_fields
            .push(EncodedField {
                field_idx: 0,
                access_flags: flags,
            });
        let aliases = inventory(std::slice::from_ref(&dex), &mut None).unwrap();
        let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
        let resolver = Resolver::new(&dex, &aliases, &metadata);
        assert_eq!(
            resolver.field_declaration(&dex.classes[0], 0).unwrap_err(),
            "interface field initializer unavailable"
        );
        let raw = MemberResolver { dex: &dex };
        assert!(raw.field_declaration_without_initialization(0).is_err());
        // A body requiring this field cannot survive the same closure gate
        // used by publication; its original identity is rejected, not edited.
        let requirements = MemberRequirements {
            fields: [0].into(),
            methods: BTreeSet::new(),
        };
        assert!(
            requirements
                .check_closure(&BTreeSet::new(), &BTreeSet::new())
                .is_err()
        );
    }
}

#[test]
fn interface_field_usage_is_rejected_and_exact_subset_compiles_java17() {
    use dex_format::{RawSpan, class::EncodedField, ids::FieldId};
    let mut dex = small_dex();
    dex.classes[0].access_flags = 0x601;
    dex.field_ids.push(FieldId {
        class_idx: 0,
        type_idx: 4,
        name_idx: 7,
        span: RawSpan { offset: 0, size: 0 },
    });
    let data = dex.classes[0].class_data.as_mut().unwrap();
    data.static_fields.push(EncodedField {
        field_idx: 0,
        access_flags: 0x19,
    });
    let code = data.direct_methods[0].code.as_mut().unwrap();
    code.registers_size = 1;
    code.insns = [0x0060u16, 0, 0x000e]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 3;
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &[dex],
        &JavaOptions::default(),
        &mut |s: &GeneratedSource| {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 1);
    assert_eq!(report.unsupported_methods, 1);
    assert_eq!(report.fields_emitted, 0);
    assert!(
        sources[0]
            .text
            .contains("unavailable original method index 0")
    );
    assert!(sources[0].text.contains("original method pool index 1"));
    assert!(
        sources[0]
            .text
            .contains("interface field initializer unavailable")
    );
    let root = std::env::temp_dir().join(format!("decomp-interface-field-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(&sources[0].relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &sources[0].text).unwrap();
    let compile = |p: &std::path::Path| {
        std::process::Command::new("javac")
            .args(["--release", "17", "-d"])
            .arg(&root)
            .arg(p)
            .output()
            .unwrap()
    };
    let result = compile(&path);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let negative = root.join("Missing.java");
    std::fs::write(
        &negative,
        "interface Missing { public static final Object field; }",
    )
    .unwrap();
    let result = compile(&negative);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("= expected"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_facade_checked_throws_boundary_and_same_class_call() {
    use dex_format::{RawSpan, annotations::*, encoded_value::*};
    let span = RawSpan { offset: 0, size: 0 };
    let item = AnnotationItem {
        visibility: Visibility::System,
        span,
        annotation: EncodedAnnotation {
            type_idx: 2,
            span,
            elements: vec![AnnotationElement {
                name_idx: 5,
                value: EncodedValue {
                    value: EncodedValueKind::Array(vec![EncodedValue {
                        value: EncodedValueKind::Type(3),
                        span,
                    }]),
                    span,
                },
            }],
        },
    };
    for caller_declares in [false, true] {
        let mut dex = small_dex();
        let code = dex.classes[0].class_data.as_mut().unwrap().direct_methods[0]
            .code
            .as_mut()
            .unwrap();
        code.insns = [0x0071u16, 1, 0, 0x000e]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        code.insns_size = 4;
        dex.classes[0].annotations = Some(AnnotationsDirectory {
            class_annotations: None,
            field_annotations: vec![],
            parameter_annotations: vec![],
            span,
            method_annotations: (if caller_declares { vec![0, 1] } else { vec![1] })
                .into_iter()
                .map(|member_idx| MemberAnnotations {
                    member_idx,
                    annotations: AnnotationSet {
                        entries: vec![item.clone()],
                        span,
                    },
                })
                .collect(),
        });
        // Genuine fixture DEX field and sget-object/throw body let runtime
        // verify the exact object, rather than replacing emitted Java bodies.
        dex.field_ids.push(dex_format::ids::FieldId {
            class_idx: 0,
            type_idx: 3,
            name_idx: 7,
            span,
        });
        let data = dex.classes[0].class_data.as_mut().unwrap();
        data.static_fields.push(dex_format::class::EncodedField {
            field_idx: 0,
            access_flags: 9,
        });
        let second = data.direct_methods[1].code.as_mut().unwrap();
        second.registers_size = 1;
        second.insns = [0x0062u16, 0, 0x0027]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        second.insns_size = 3;
        let mut sources = Vec::new();
        let report = crate::dex_java::emit_java(
            &[dex],
            &JavaOptions {
                classpath: vec![std::path::PathBuf::from(
                    "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
                )],
                ..Default::default()
            },
            &mut |s: &GeneratedSource| {
                sources.push((s.relative_path.clone(), s.text.clone()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(report.complete_methods, 2, "{report:?} {sources:?}");
        assert_eq!(report.unsupported_methods, 0);
        assert!(
            sources[0]
                .1
                .contains("void second() throws java.lang.Exception")
        );
        if caller_declares {
            assert!(
                sources[0]
                    .1
                    .contains("void first() throws java.lang.Exception")
            );
        } else {
            assert!(sources[0].1.contains("void first() {"));
            assert!(!sources[0].1.contains("void first() throws"));
            assert!(sources[0].1.contains("Synthetic checked-source bridge"));
        }
        assert_eq!(
            sources[0].1.matches("original method pool index").count(),
            2
        );
        let root = std::env::temp_dir().join(format!(
            "decomphify-original-source-clause-{}-{caller_declares}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_path = root.join(&sources[0].0);
        std::fs::write(&source_path, &sources[0].1).unwrap();
        let class = source_path.file_stem().unwrap().to_str().unwrap();
        std::fs::write(root.join("Run.java"),format!("public class Run {{ public static void main(String[] args)throws Throwable {{ java.lang.Exception value=new java.lang.Exception(\"identity\");value.addSuppressed(new Exception());StackTraceElement[] stack=value.getStackTrace(); {class}.field=value;try{{{class}.first();throw new AssertionError();}}catch(Throwable actual){{if(actual!=value||!java.util.Arrays.equals(stack,value.getStackTrace())||actual.getSuppressed().length!=1)throw new AssertionError();}}if({class}.class.getDeclaredMethod(\"second\").getExceptionTypes()[0]!=Exception.class)throw new AssertionError();if({class}.class.getDeclaredMethod(\"first\").getExceptionTypes().length!={})throw new AssertionError();}}}}",usize::from(caller_declares))).unwrap();
        for jdk in [
            "/home/jtjisgod/tools/jdk-17.0.13+11",
            "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
        ] {
            let compilation = std::process::Command::new(format!("{jdk}/bin/javac"))
                .arg("-d")
                .arg(&root)
                .arg(&source_path)
                .arg(root.join("Run.java"))
                .output()
                .unwrap();
            assert!(
                compilation.status.success(),
                "{}",
                String::from_utf8_lossy(&compilation.stderr)
            );
            let run = std::process::Command::new(format!("{jdk}/bin/java"))
                .arg("-cp")
                .arg(&root)
                .arg("Run")
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
        }
    }
}

#[path = "java_wave11_boolean_parameter_tests.rs"]
mod wave11_boolean_parameter_tests;
