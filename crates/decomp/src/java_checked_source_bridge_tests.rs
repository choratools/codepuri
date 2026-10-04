#![allow(clippy::unwrap_used)]
use super::*;
#[test]
#[ignore = "bounded actual Samsung Pass and genuine JDK17/Android35 metadata"]
fn actual_fido_supplier_and_messageutil_original_contract_bridge() {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
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
    let roots = [16702, 14395]
        .map(|i| method_key(&files[1], i).unwrap())
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
    let metadata = dex_decompiler::infer::DexMetadata::new(&files[1]);
    let member_index = crate::java_members::MemberIndex::new(&files[1]);
    let mut r = Resolver::with_catalog(&files[1], &aliases, &metadata, Some(&catalog));
    r.inputs = &files;
    r.dex_index = 1;
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
    for i in [16702, 14395] {
        let key = method_key(&files[1], i).unwrap();
        let facts = plan
            .runtime
            .get(&key)
            .unwrap_or_else(|| panic!("no runtime {key:?}: {:?}", plan.unavailable.get(&key)));
        assert!(facts.original.is_empty());
        assert!(!facts.escaping.is_empty());
        let (class, method) = files[1]
            .classes
            .iter()
            .find_map(|c| {
                c.class_data.as_ref().and_then(|data| {
                    data.direct_methods
                        .iter()
                        .chain(&data.virtual_methods)
                        .find(|m| m.method_idx == i)
                        .map(|m| (c, m))
                })
            })
            .unwrap();
        assert!(method.code.as_ref().unwrap().tries.is_empty());
        let candidate = method_source(class, method, &r, ClassKind::Class, &mut None)
            .unwrap_or_else(|e| panic!("{i} {key:?}: {e}"));
        assert!(candidate.body);
        let source =
            if i == 16702 {
                assert!(
                    candidate
                        .source
                        .contains("// Caller-local checked-source recipe:")
                );
                assert!(candidate.source.contains(
                    "// Synthetic checked-source bridge; no original DEX method identity."
                ));
                assert!(
                    !candidate
                        .source
                        .lines()
                        .next()
                        .unwrap()
                        .contains(" throws ")
                );
                candidate.source
            } else {
                // Existing valid projections must remain intact for encoded checked
                // catches in other original callers. Demonstrate local feasibility
                // separately; this is not a published-signature restoration.
                assert!(
                    !candidate
                        .source
                        .contains("// Caller-local checked-source recipe:")
                );
                assert!(
                    candidate
                        .source
                        .lines()
                        .next()
                        .unwrap()
                        .contains(" throws ")
                );
                let mut original = r.method(MethodRef::new(i)).unwrap();
                original.throws.clone_from(&facts.original);
                let code = method.code.as_ref().unwrap();
                let (context, seeds) = initial_context_with_seeds(&r, method).unwrap();
                let ssa = cached_frontend(&r, method, code, &context, seeds, None).unwrap();
                let bindings = declaration_bindings(&ssa, &original, code).unwrap();
                let bridge = backend_java::checked_source_bridge::plan(
                    ssa.ir(),
                    &original,
                    &facts.escaping,
                    "boundedLocalRecipe",
                    &r,
                )
                .unwrap()
                .unwrap();
                let SourceResult::Complete { mut source, .. } =
                    backend_java::checked_source_bridge::emit(ssa.ir(), &bindings, &r, &bridge)
                else {
                    panic!("bounded original 14395 recipe rejected");
                };
                assert!(!source.lines().next().unwrap().contains(" throws "));
                source.push_str(&bridge.helper_source());
                source
            };
        let dir = std::env::temp_dir().join(format!(
            "decomphify-actual-checked-bridge-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{i}.java.fragment")), &source).unwrap();
        println!(
            "EXACT {i} {key:?}; runtime {:?}; fragment {}",
            facts.escaping,
            dir.display()
        );
    }
}

#[test]
#[ignore = "bounded actual Samsung Pass and genuine JDK17/Android35 metadata"]
fn actual_six_checked_roots_source_contract_bridge() {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
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
    let roots = [43824, 43830, 45524, 46352, 46603, 60310]
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
    let dir = std::env::temp_dir().join(format!(
        "decomphify-six-source-contract-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for i in [43824, 43830, 45524, 46352, 46603, 60310] {
        let key = method_key(&files[2], i).unwrap();
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
        let code = method.code.as_ref().unwrap();
        assert!(code.tries.is_empty());
        let (context, seeds) = initial_context_with_seeds(&r, method).unwrap();
        let ssa = cached_frontend(&r, method, code, &context, seeds, None).unwrap();
        let original = r.original_method(MethodRef::new(i)).unwrap();
        let facts = plan.runtime.get(&key);
        let recipe = backend_java::checked_source_bridge::plan_source_contract(
            ssa.ir(),
            &original,
            facts.map(|f| f.escaping.as_slice()),
            "boundedRecipe",
            &r,
        );
        println!(
            "ROOT {i} {key:?} runtime={:?} unavailable={:?} recipe={recipe:?}",
            facts,
            plan.unavailable.get(&key)
        );
        match method_source(class, method, &r, ClassKind::Class, &mut None) {
            Ok(candidate) => {
                println!("EMITTED {i}");
                std::fs::write(dir.join(format!("{i}.java.fragment")), &candidate.source).unwrap();
                if i == 46603 {
                    assert!(
                        candidate
                            .source
                            .contains("Caller-local checked-source recipe")
                    );
                    assert!(candidate.source.contains("runtime escaping None"));
                    assert_eq!(
                        candidate
                            .source
                            .matches("catch (java.lang.Exception")
                            .count(),
                        1
                    );
                    assert_eq!(candidate.source.matches("qs.h$type.m(").count(), 1);
                    assert_eq!(candidate.source.matches("new cq.s0$type").count(), 1);
                    assert!(
                        candidate.source.find("qs.h$type.m(").unwrap()
                            < candidate.source.find("new cq.s0$type").unwrap()
                    );
                }
            }
            Err(error) => {
                println!("REJECTED {i}: {error}");
                panic!("all six source recipes must emit: {i}: {error}");
            }
        }
    }
    println!("ARTIFACT {}", dir.display());
}
