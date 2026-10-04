//! Bounded original Samsung methods, with genuine original alias and classpath
//! metadata, exercising production contextual and handler-predicate recovery.
#![allow(clippy::unwrap_used)]
use super::*;

#[test]
#[ignore = "original Samsung APK with local genuine Android/JDK17 metadata"]
fn samsung_wave7_original_type_and_predicate_witnesses() {
    let files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let external = crate::java_external_metadata::Classpath::from_paths(&[
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
        std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let mut inventory = AliasInventory::default();
    for dex in &files {
        for ty in &dex.type_ids {
            inventory
                .add_type(dex.string(ty.descriptor_idx).unwrap())
                .unwrap();
        }
        for index in 0..dex.method_ids.len() {
            inventory
                .add_method(method_key(dex, index as u32).unwrap())
                .unwrap();
        }
        for index in 0..dex.field_ids.len() {
            inventory
                .add_field(field_key(dex, index as u32).unwrap())
                .unwrap();
        }
    }
    let aliases = inventory.build(AliasMode::Ascii);
    let catalog = method_catalog(&files);
    let original = Arc::new(
        crate::java_hierarchy::build_hierarchy(
            &files.iter().collect::<Vec<_>>(),
            external.hierarchy(),
        )
        .unwrap(),
    );
    let hierarchy = original
        .map_names(|n| {
            aliases
                .class_name(&format!("L{};", n.replace('.', "/")))
                .unwrap_or(n)
                .to_owned()
        })
        .unwrap();
    let dex = &files[0];
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let mut resolver = Resolver::with_catalog(dex, &aliases, &metadata, Some(&catalog));
    resolver.inputs = &files;
    resolver.dex_index = 0;
    resolver.external = Some(&external);
    resolver.original_hierarchy = Some(original);
    resolver.hierarchy = hierarchy;
    resolver.input_owners = files
        .iter()
        .flat_map(|dex| {
            dex.classes
                .iter()
                .filter_map(|c| descriptor(dex, c.class_idx).map(str::to_owned))
        })
        .collect();
    for d in external.hierarchy() {
        let key = format!("L{};", d.name.replace('.', "/"));
        let name = aliases.class_name(&key).unwrap_or(&d.name).to_owned();
        if let Some(access_flags) = external.class_access(&key) {
            resolver.class_catalog.insert(
                name.clone(),
                Some(backend_java::constructors::ConstructorClass {
                    name,
                    superclass: d.superclass.map(|s| {
                        aliases
                            .class_name(&format!("L{};", s.replace('.', "/")))
                            .unwrap_or(&s)
                            .to_owned()
                    }),
                    access_flags,
                }),
            );
        }
    }
    for dex in &files {
        for c in &dex.classes {
            if let Some(name) = descriptor(dex, c.class_idx).and_then(|d| aliases.class_name(d)) {
                resolver.class_catalog.insert(
                    name.to_owned(),
                    Some(backend_java::constructors::ConstructorClass {
                        name: name.to_owned(),
                        superclass: descriptor(dex, c.superclass_idx)
                            .and_then(|d| aliases.class_name(d))
                            .map(str::to_owned),
                        access_flags: c.access_flags,
                    }),
                );
            }
        }
    }
    for (method_index, expected_owner, expected_name) in [
        (38394, "Lda/m0;", "e1"),
        (33268, "Lcom/google/android/material/tabs/TabLayout;", "C0"),
    ] {
        let key = method_key(dex, method_index).unwrap();
        assert_eq!(key.owner, expected_owner);
        assert_eq!(key.name, expected_name);
        let class = dex
            .classes
            .iter()
            .find(|c| {
                c.class_data.as_ref().is_some_and(|d| {
                    d.direct_methods
                        .iter()
                        .chain(&d.virtual_methods)
                        .any(|m| m.method_idx == method_index)
                })
            })
            .unwrap();
        let d = class.class_data.as_ref().unwrap();
        let method = d
            .direct_methods
            .iter()
            .chain(&d.virtual_methods)
            .find(|m| m.method_idx == method_index)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
        let ssa = cached_frontend(&resolver, method, code, &ctx, seeds, None).unwrap();
        let ir = ssa.ir();
        if method_index == 38394 {
            assert_eq!(u16::from_le_bytes([code.insns[52], code.insns[53]]), 0x0331);
            assert_eq!(u16::from_le_bytes([code.insns[56], code.insns[57]]), 0x033a);
            let result = ir
                .instructions()
                .iter()
                .find_map(|(id, insn)| match insn {
                    ManagedInsn::OrderingCompare { result, .. } if ssa.offset(id) == Some(26) => {
                        Some(*result)
                    }
                    _ => None,
                })
                .unwrap();
            assert!(ir.instructions().values().any(|insn|matches!(insn,ManagedInsn::Compare {op:managed_ir::ops::CompareOp::LessThan,lhs,..} if *lhs==result)),"actual if-ltz must consume cmp-long, never the reference at offset12");
        } else {
            let signature = resolver.method(MethodRef::new(method_index)).unwrap();
            let declarations = backend_java::contextual_locals::declaration_plan(
                ir,
                &signature.return_type,
                &resolver,
            )
            .unwrap();
            let receiver = ir
                .instructions()
                .values()
                .find_map(|insn| match insn {
                    ManagedInsn::Invoke { method, args, .. } if method.index() == 3691 => {
                        Some(args[0])
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                declarations.get(&receiver),
                Some(&JavaType::Class("android.view.View".into()))
            );
        }
        let source = method_source(class, method, &resolver, ClassKind::Class, &mut None)
            .unwrap_or_else(|error| {
                panic!("original {expected_owner}->{expected_name} rejected: {error}")
            });
        assert!(source.body);
        let path = format!("/tmp/opencode/wave7-original-{method_index}.java");
        std::fs::write(&path, &source.source).unwrap();
        println!(
            "ORIGINAL ACCEPTED classes.dex:{method_index} owner={expected_owner} name={expected_name} bytes={} path={path}",
            source.source.len()
        );
    }
}

#[test]
fn handler_cmp_predicate_actual_facade_compiles_and_runs_unchanged_method() {
    use dex_format::{
        RawSpan,
        class::ClassData,
        code::{CodeItem, EncodedCatchHandler, TryItem},
        ids::{MethodId, ProtoId, TypeId, TypeList},
        strings::StringData,
    };
    let mut dex = DexFile::from_bytes(&super::fixture::minimal_dex()).unwrap();
    let span = RawSpan { offset: 0, size: 0 };
    for text in ["I", "compare"] {
        dex.strings.push(StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
    }
    dex.type_ids.push(TypeId {
        descriptor_idx: 1,
        span,
    });
    dex.proto_ids.push(ProtoId {
        shorty_idx: 1,
        return_type_idx: 1,
        parameters_off: 1,
        parameters: Some(TypeList {
            types: vec![1],
            span,
        }),
        span,
    });
    dex.method_ids.push(MethodId {
        class_idx: 0,
        proto_idx: 0,
        name_idx: 2,
        span,
    });
    // Division selects the genuine catch-all path. Inside that handler the
    // category-one v3=7 is overwritten by a wide value, then cmp-long writes
    // category-one v3=-1. if-ltz must read this result, not the old v3=7.
    let units = [
        0x1012u16, 0x0093, 0x0600, 0x0012, 0x000f, 0x010d, 0x7312, 0x0316, 10, 0x0516, 20, 0x0331,
        0x0503, 0x033a, 4, 0x1012, 0x000f, 0xf012, 0x000f,
    ];
    dex.classes[0].class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        virtual_methods: vec![],
        span,
        direct_methods: vec![EncodedMethod {
            method_idx: 0,
            access_flags: 9,
            code_off: 1,
            code: Some(CodeItem {
                registers_size: 7,
                ins_size: 1,
                outs_size: 0,
                debug_info_off: 0,
                insns_size: units.len() as u32,
                insns: units.into_iter().flat_map(u16::to_le_bytes).collect(),
                tries: vec![TryItem {
                    start_addr: 1,
                    insn_count: 2,
                    handler_off: 0,
                    span,
                }],
                handlers: vec![EncodedCatchHandler {
                    relative_offset: 0,
                    handlers: vec![],
                    catch_all_addr: Some(5),
                    span,
                }],
                span,
            }),
        }],
    });
    let aliases = inventory(std::slice::from_ref(&dex), &mut None).unwrap();
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let external =
        crate::java_external_metadata::Classpath::from_paths(&[home.join("jmods/java.base.jmod")])
            .unwrap();
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let mut resolver = Resolver::new(&dex, &aliases, &metadata);
    resolver.external = Some(&external);
    resolver.hierarchy =
        crate::java_hierarchy::build_hierarchy(&[&dex], external.hierarchy()).unwrap();
    let class = &dex.classes[0];
    let method = &class.class_data.as_ref().unwrap().direct_methods[0];
    let source = method_source(class, method, &resolver, ClassKind::Class, &mut None)
        .unwrap_or_else(|error| panic!("handler compare facade: {error}"));
    assert!(source.body);
    let dir =
        std::env::temp_dir().join(format!("decomp-handler-cmp-facade-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = format!(
        "public class Fixture {{ {} public static void main(String[] args) {{ if(compare(0)!=-1 || compare(1)!=0 || compare(-7)!=0) throw new AssertionError(); }} }}",
        source.source
    );
    std::fs::write(dir.join("Fixture.java"), text).unwrap();
    let compile = std::process::Command::new(home.join("bin/javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(home.join("bin/java"))
        .args(["-cp", ".", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
