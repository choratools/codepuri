#![allow(clippy::unwrap_used)]
use super::*;
#[test]
#[ignore = "genuine Samsung Pass and explicit JDK17 metadata; four original methods only"]
fn original_aead_requirements_select_same_checked_target_as_backend() {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let cp = crate::java_external_metadata::Classpath::from_paths([
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
    ])
    .unwrap();
    let mut inventory = AliasInventory::default();
    for d in &files {
        for t in &d.type_ids {
            inventory
                .add_type(d.string(t.descriptor_idx).unwrap())
                .unwrap();
        }
        for i in 0..d.method_ids.len() {
            inventory
                .add_method(method_key(d, i as u32).unwrap())
                .unwrap();
        }
        for i in 0..d.field_ids.len() {
            inventory
                .add_field(field_key(d, i as u32).unwrap())
                .unwrap();
        }
    }
    let aliases = inventory.build(AliasMode::Ascii);
    let catalog = method_catalog(&files);
    let d = &files[3];
    let metadata = dex_decompiler::infer::DexMetadata::new(d);
    let field_index = crate::java_members::external_metadata::fields::InputIndex::new(&files);
    let mut r = Resolver::with_catalog(d, &aliases, &metadata, Some(&catalog));
    r.inputs = &files;
    r.external = Some(&cp);
    r.field_index = Some(&field_index);
    r.input_owners = files
        .iter()
        .flat_map(|d| {
            d.classes
                .iter()
                .filter_map(|c| descriptor(d, c.class_idx).map(str::to_owned))
        })
        .collect();
    let original =
        crate::java_hierarchy::build_hierarchy(&files.iter().collect::<Vec<_>>(), cp.hierarchy())
            .unwrap();
    r.hierarchy = original
        .map_names(|n| {
            aliases
                .class_name(&format!("L{};", n.replace('.', "/")))
                .unwrap_or(n)
                .into()
        })
        .unwrap();
    r.original_hierarchy = Some(Arc::new(original));
    for declaration in cp.hierarchy() {
        let original = format!("L{};", declaration.name.replace('.', "/"));
        if let Some(access_flags) = cp.class_access(&original) {
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
        for c in &dex.classes {
            let name = aliases
                .class_name(descriptor(dex, c.class_idx).unwrap())
                .unwrap()
                .to_owned();
            r.class_catalog.insert(
                name.clone(),
                Some(backend_java::constructors::ConstructorClass {
                    name,
                    superclass: descriptor(dex, c.superclass_idx)
                        .and_then(|d| aliases.class_name(d))
                        .map(str::to_owned),
                    access_flags: c.access_flags,
                }),
            );
        }
    }
    for index in [10608, 10609, 10620, 10621] {
        let (class, method) = d
            .classes
            .iter()
            .find_map(|c| {
                c.class_data.as_ref().and_then(|data| {
                    data.direct_methods
                        .iter()
                        .chain(&data.virtual_methods)
                        .find(|m| m.method_idx == index)
                        .map(|m| (c, m))
                })
            })
            .unwrap();
        let code = method.code.as_ref().unwrap();
        assert!(code.tries.is_empty());
        let (ctx, seeds) = initial_context_with_seeds(&r, method).unwrap();
        let ssa = cached_frontend(&r, method, code, &ctx, seeds, None).unwrap();
        let signature = r.method(MethodRef::new(index)).unwrap();
        assert!(signature.throws.is_empty());
        let (reference, receiver) = ssa
            .ir()
            .instructions()
            .values()
            .find_map(|i| {
                if let ManagedInsn::Invoke {
                    kind: InvokeKind::Virtual,
                    method,
                    args,
                    ..
                } = i
                {
                    let target = r.method(*method)?;
                    if target.owner == "java.io.OutputStream" {
                        Some((
                            *method,
                            managed_type_to_java(ssa.ir().value(args[0]).ok()?.ty(), &r)?,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap();
        let symbolic = r.method(reference).unwrap();
        assert_eq!(symbolic.throws, vec!["java.io.IOException"]);
        assert!(
            backend_java::checked_exceptions::check(&r, &symbolic.throws, &signature.throws)
                .is_err()
        );
        let selected = r
            .virtual_receiver_method(reference, &signature.owner, &receiver)
            .unwrap();
        assert_eq!(selected.owner, "java.io.ByteArrayOutputStream");
        assert!(selected.throws.is_empty());
        for unknown in [
            JavaType::Class("java.io.OutputStream".into()),
            JavaType::Class("missing.Unknown".into()),
        ] {
            assert!(
                r.virtual_receiver_method(reference, &signature.owner, &unknown)
                    .is_none()
            );
        }
        requirements(ssa.ir(), &signature, &r, false)
            .unwrap_or_else(|e| panic!("original{index}: {e}"));
        let bindings = declaration_bindings(&ssa, &signature, code).unwrap();
        let result = backend_java::emit_method_declaration(ssa.ir(), &signature, &bindings, &r);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{index}: {result:?}")
        };
        assert!(!source.contains("throws java.io.IOException"));
        assert!(!source.contains("((java.io.OutputStream)"));
        let path = std::env::temp_dir().join(format!("wave33-original-{index}.java.fragment"));
        std::fs::write(&path, &source).unwrap();
        println!(
            "ORIGINAL{index} symbolic={symbolic:?}; selected={selected:?}; fragment={} class={}",
            path.display(),
            class.class_idx
        );
    }
}
