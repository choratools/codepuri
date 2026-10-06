//! A rejected instance family cannot erase independently attested static code.
#![allow(clippy::unwrap_used)]
use super::*;
use dex_format::{
    RawSpan,
    class::{ClassData, EncodedField},
    code::CodeItem,
    encoded_value::{EncodedValue, EncodedValueKind},
    ids::{FieldId, MethodId, ProtoId, TypeId},
    strings::StringData,
};

fn jdk() -> std::path::PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
}
fn code(units: &[u16], registers: u16, incoming: u16, outgoing: u16) -> CodeItem {
    CodeItem {
        registers_size: registers,
        ins_size: incoming,
        outs_size: outgoing,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    }
}
fn fixture(duplicate_store: bool) -> DexFile {
    let mut dex = DexFile::from_bytes(&super::index_tests::fixture::minimal_dex()).unwrap();
    let span = RawSpan { offset: 0, size: 0 };
    dex.strings[0].value = "LStaticPublication;".into();
    for text in [
        "Ljava/lang/Object;",
        "V",
        "I",
        "<clinit>",
        "<init>",
        "helper",
        "PREFIX",
        "VALUE",
    ] {
        dex.strings.push(StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
    }
    for descriptor_idx in [1, 2, 3] {
        dex.type_ids.push(TypeId {
            descriptor_idx,
            span,
        });
    }
    dex.proto_ids.push(ProtoId {
        shorty_idx: 2,
        return_type_idx: 2,
        parameters_off: 0,
        parameters: None,
        span,
    });
    for (class_idx, name_idx) in [(0, 4), (0, 5), (0, 6), (1, 5)] {
        dex.method_ids.push(MethodId {
            class_idx,
            proto_idx: 0,
            name_idx,
            span,
        });
    }
    for name_idx in [7, 8] {
        dex.field_ids.push(FieldId {
            class_idx: 0,
            type_idx: 3,
            name_idx,
            span,
        });
    }
    dex.classes[0].access_flags = 0x11; // public final ordinary class
    dex.classes[0].superclass_idx = 1;
    dex.classes[0].static_values = Some(vec![EncodedValue {
        value: EncodedValueKind::Int(3),
        span,
    }]);
    let static_code = if duplicate_store {
        code(&[0x7012, 0x0067, 1, 0x0067, 1, 0x000e], 1, 0, 0)
    } else {
        code(&[0x7012, 0x0067, 1, 0x000e], 1, 0, 0)
    };
    dex.classes[0].class_data = Some(ClassData {
        static_fields: vec![
            EncodedField {
                field_idx: 0,
                access_flags: 0x19,
            },
            EncodedField {
                field_idx: 1,
                access_flags: 0x19,
            },
        ],
        instance_fields: vec![],
        direct_methods: vec![
            EncodedMethod {
                method_idx: 0,
                access_flags: 8,
                code_off: 1,
                code: Some(static_code),
            },
            EncodedMethod {
                method_idx: 1,
                access_flags: 0x10001,
                code_off: 2,
                code: Some(code(&[0x1070, 3, 0, 0x1070, 2, 0, 0x000e], 1, 1, 1)),
            },
            // Genuine managed synchronization: an extra original inert const
            // separates balanced monitor-enter/exit, so the narrow empty-body
            // scaffold emitter cannot publish this helper. No lock is removed.
            EncodedMethod {
                method_idx: 2,
                access_flags: 0x20002,
                code_off: 3,
                code: Some(code(&[0x011d, 0x1012, 0x011e, 0x000e], 2, 1, 0)),
            },
        ],
        virtual_methods: vec![],
        span,
    });
    dex
}

#[test]
fn full_family_publication_failure_retains_independent_original_clinit_without_granting_family_proof()
 {
    for cache_inventory in [false, true] {
        let dex = fixture(false);
        let files = [dex];
        let aliases = inventory(&files, &mut None).unwrap();
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
        let external = crate::java_external_metadata::Classpath::from_paths(&[
            jdk().join("jmods/java.base.jmod")
        ])
        .unwrap();
        let mut r = Resolver::new(&files[0], &aliases, &metadata);
        r.external = Some(&external);
        r.cache_inventory = cache_inventory;
        let writes = initializers::write_inventory::WriteInventory::scan(&files);
        r.writes = Some(&writes);
        for declaration in external.hierarchy() {
            let original = format!("L{};", declaration.name.replace('.', "/"));
            if let Some(access_flags) = external.class_access(&original) {
                r.class_catalog.insert(
                    declaration.name.clone(),
                    Some(backend_java::constructors::ConstructorClass {
                        name: declaration.name.clone(),
                        superclass: declaration.superclass.clone(),
                        access_flags,
                    }),
                );
            }
            r.hierarchy.insert(declaration).unwrap();
        }
        r.class_catalog.insert(
            "StaticPublication".into(),
            Some(backend_java::constructors::ConstructorClass {
                name: "StaticPublication".into(),
                superclass: Some("java.lang.Object".into()),
                access_flags: 0x11,
            }),
        );
        r.hierarchy
            .insert(crate::type_compatibility::Declaration {
                name: "StaticPublication".into(),
                superclass: Some("java.lang.Object".into()),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
        r.input_owners.insert("LStaticPublication;".into());
        let class = &files[0].classes[0];
        // This is the regression's decisive oracle: semantic planning succeeds,
        // yet publication of its constructor depends on an unavailable helper.
        let (family, fields, prefix) = initialization_source(class, &r, &mut None).unwrap();
        assert_eq!(family.keys().copied().collect::<Vec<_>>(), [0, 1]);
        assert_eq!(family[&1].requirements.methods, BTreeSet::from([2]));
        let helper = &class.class_data.as_ref().unwrap().direct_methods[2];
        assert!(method_source(class, helper, &r, ClassKind::Class, &mut None).is_err());
        let rejected = closure_failures(
            family.iter().map(|(&i, c)| (i, c.body, &c.requirements)),
            &fields.keys().copied().collect(),
        );
        assert!(rejected.contains_key(&1));
        assert!(!rejected.contains_key(&0));
        assert!(prefix.contains("PREFIX = 3"));
        let mut sources = Vec::new();
        let report = crate::dex_java::emit_java(
            &files,
            &JavaOptions {
                classpath: vec![jdk().join("jmods/java.base.jmod")],
                cache_inventory,
                ..Default::default()
            },
            &mut |source: &GeneratedSource| {
                sources.push(source.clone());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            report.complete_static_initializers, 1,
            "{:?}",
            report.reasons
        );
        assert_eq!(report.complete_constructors, 0);
        assert_eq!(report.unavailable_initialization_units, 1);
        assert_eq!(sources.len(), 1);
        let source = &sources[0];
        assert_eq!(source.provenance.descriptor, "LStaticPublication;");
        assert_eq!(source.provenance.status, SourceStatus::Fallback);
        assert!(source.text.contains("original method pool index 0"));
        assert!(source.text.contains("unavailable original method index 1"));
        assert!(
            source
                .text
                .contains("Original field initialization proof: unavailable")
        );
        assert!(source.text.find("PREFIX = 3").unwrap() < source.text.find("VALUE =").unwrap());
        assert!(!source.text.contains("public StaticPublication("));
        let dir = std::env::temp_dir().join(format!(
            "decomphify-static-publication-{}-{cache_inventory}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("StaticPublication.java"), format!("{}\nclass StaticPublicationCheck {{ public static void main(String[] args) {{ if(StaticPublication.PREFIX!=3 || StaticPublication.VALUE!=7) throw new AssertionError(); System.out.print(\"static-publication-ok\"); }} }}", source.text)).unwrap();
        let compiled = std::process::Command::new(jdk().join("bin/javac"))
            .arg("--release")
            .arg("17")
            .arg("StaticPublication.java")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&compiled.stderr),
            source.text
        );
        let run = std::process::Command::new(jdk().join("bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("StaticPublicationCheck")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "static-publication-ok"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn duplicate_original_static_final_store_is_still_rejected() {
    let files = [fixture(true)];
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &files,
        &JavaOptions {
            classpath: vec![jdk().join("jmods/java.base.jmod")],
            ..Default::default()
        },
        &mut |source: &GeneratedSource| {
            sources.push(source.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        report.complete_static_initializers, 0,
        "{:?}",
        report.reasons
    );
    assert!(
        sources
            .iter()
            .all(|s| !s.text.contains("original method pool index 0;"))
    );
}

#[test]
fn unwritten_methods_have_diagnostics_without_source_or_publication_credit() {
    let mut dex = fixture(false);
    // A genuinely invalid original class declaration makes the source gate fail;
    // this is diagnostic input, never permission to publish a malformed header.
    dex.classes[0].access_flags |= 0x8000_0000;
    dex.classes[0].static_values = None;
    let data = dex.classes[0].class_data.as_mut().unwrap();
    data.static_fields.clear();
    data.instance_fields.clear();
    for method in data
        .direct_methods
        .iter_mut()
        .chain(&mut data.virtual_methods)
    {
        method.code = Some(code(&[0xffff], 2, 1, 0));
    }
    let originals: BTreeSet<_> = data
        .direct_methods
        .iter()
        .chain(&data.virtual_methods)
        .map(|m| m.method_idx)
        .collect();
    let files = [dex];
    let mut delivered = 0;
    let report = crate::dex_java::emit_java(
        &files,
        &JavaOptions {
            classpath: vec![jdk().join("jmods/java.base.jmod")],
            ..Default::default()
        },
        &mut |_: &GeneratedSource| {
            delivered += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(delivered, 0);
    assert_eq!(report.classes_emitted, 0);
    assert_eq!(report.complete_methods, 0);
    assert_eq!(report.complete_constructors, 0);
    assert_eq!(report.complete_static_initializers, 0);
    assert_eq!(report.declaration_only_methods, 0);
    assert_eq!(report.unpublished_method_reasons.len(), originals.len());
    for index in originals {
        let key = method_key(&files[0], index).unwrap();
        let identity = format!(
            "{}->{}({}){}",
            key.owner,
            key.name,
            key.parameters.join(""),
            key.return_type
        );
        assert!(!report.unpublished_method_reasons[&identity].is_empty());
    }
}
