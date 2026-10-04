#![allow(clippy::unwrap_used)]
use super::*;
#[test]
#[ignore = "genuine Samsung APK and explicit JDK17/Android35 metadata"]
fn actual_sole_field_throw_callback_bound_source_recipe() {
    let files = dex_format::apk::parse_apk_path(std::path::Path::new(
        "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    ))
    .unwrap();
    let external = crate::java_external_metadata::Classpath::from_paths(&[
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
        std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/jdk.unsupported.jmod"),
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.naming.jmod"),
    ])
    .unwrap();
    let mut inventory = AliasInventory::default();
    for dex in &files {
        for ty in &dex.type_ids {
            inventory
                .add_type(dex.string(ty.descriptor_idx).unwrap())
                .unwrap();
        }
        for i in 0..dex.method_ids.len() {
            inventory
                .add_method(method_key(dex, i as u32).unwrap())
                .unwrap();
        }
        for i in 0..dex.field_ids.len() {
            inventory
                .add_field(field_key(dex, i as u32).unwrap())
                .unwrap();
        }
    }
    let aliases = inventory.build(AliasMode::Ascii);
    let original_hierarchy = Arc::new(
        crate::java_hierarchy::build_hierarchy(
            &files.iter().collect::<Vec<_>>(),
            external.hierarchy(),
        )
        .unwrap(),
    );
    let hierarchy = Arc::new(
        original_hierarchy
            .map_names(|n| {
                aliases
                    .class_name(&format!("L{};", n.replace('.', "/")))
                    .unwrap_or(n)
                    .into()
            })
            .unwrap(),
    );
    let catalog = method_catalog(&files);
    let roots = [51318, 51320]
        .map(|i| method_key(&files[2], i).unwrap())
        .into_iter()
        .collect();
    let plan = source_throws::collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    let field_index = crate::java_members::external_metadata::fields::InputIndex::new(&files);
    let metadata = dex_decompiler::infer::DexMetadata::new(&files[2]);
    let member_index = crate::java_members::MemberIndex::new(&files[2]);
    let mut r = Resolver::with_catalog(&files[2], &aliases, &metadata, Some(&catalog));
    r.inputs = &files;
    r.dex_index = 2;
    r.external = Some(&external);
    r.hierarchy = (*hierarchy).clone();
    r.original_hierarchy = Some(original_hierarchy);
    r.field_index = Some(&field_index);
    r.member_index = Some(&member_index);
    r.source_throws = Some(&plan);
    for dex in &files {
        for c in &dex.classes {
            r.input_owners
                .insert(descriptor(dex, c.class_idx).unwrap().into());
        }
    }
    for declaration in external.hierarchy() {
        let original = format!("L{};", declaration.name.replace('.', "/"));
        if let Some(access_flags) = external.class_access(&original) {
            let name = aliases
                .class_name(&original)
                .unwrap_or(&declaration.name)
                .to_owned();
            let superclass = declaration.superclass.map(|s| {
                aliases
                    .class_name(&format!("L{};", s.replace('.', "/")))
                    .unwrap_or(&s)
                    .to_owned()
            });
            r.class_catalog.insert(
                name.clone(),
                Some(backend_java::constructors::ConstructorClass {
                    name,
                    superclass,
                    access_flags,
                }),
            );
        }
    }
    for dex in &files {
        for class in &dex.classes {
            let name = aliases
                .class_name(descriptor(dex, class.class_idx).unwrap())
                .unwrap()
                .to_owned();
            let superclass = descriptor(dex, class.superclass_idx)
                .and_then(|d| aliases.class_name(d))
                .map(str::to_owned);
            r.class_catalog.insert(
                name.clone(),
                Some(backend_java::constructors::ConstructorClass {
                    name,
                    superclass,
                    access_flags: class.access_flags,
                }),
            );
        }
    }

    let witnesses = crate::java_checked_bound_targets::FieldThrowCatalog::new(&files, &catalog);
    r.field_throw_catalog = Some(&witnesses);
    eprintln!(
        "actual structural candidates {}",
        witnesses.candidate_count()
    );
    let bound = witnesses
        .bind(&files[2], MethodRef::new(43295), &r)
        .unwrap();
    assert_eq!(bound.code_offset(), 4697940);
    for i in [51318, 51320] {
        let (class, method) = files[2]
            .classes
            .iter()
            .find_map(|c| {
                c.class_data.as_ref().and_then(|d| {
                    d.direct_methods
                        .iter()
                        .chain(&d.virtual_methods)
                        .find(|m| m.method_idx == i)
                        .map(|m| (c, m))
                })
            })
            .unwrap();
        let candidate = method_source(class, method, &r, ClassKind::Class, &mut None)
            .unwrap_or_else(|e| panic!("original {i}: {e}"));
        assert!(candidate.body);
        assert!(candidate.source.contains("catch (java.lang.Throwable"));
        assert!(candidate.source.contains("m0$type.b"));
        std::fs::write(
            format!("/tmp/opencode/wave49-throwable-prototype/original-{i}.java"),
            &candidate.source,
        )
        .unwrap();
        eprintln!("original {i} COMPLETE {}", candidate.source);
    }
}
