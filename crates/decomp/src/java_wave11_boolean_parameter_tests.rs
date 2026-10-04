//! Bounded original Samsung methods, with genuine original alias and classpath
//! metadata, exercising production contextual and handler-predicate recovery.
#![allow(clippy::unwrap_used)]
use super::*;

fn java_type(ty: &JavaType) -> String {
    match ty {
        JavaType::Void => "void".into(),
        JavaType::Boolean => "boolean".into(),
        JavaType::Byte => "byte".into(),
        JavaType::Short => "short".into(),
        JavaType::Char => "char".into(),
        JavaType::Int => "int".into(),
        JavaType::Long => "long".into(),
        JavaType::Float => "float".into(),
        JavaType::Double => "double".into(),
        JavaType::Class(name) => name.clone(),
        JavaType::Array(element) => format!("{}[]", java_type(element)),
    }
}

#[test]
#[ignore = "original Samsung APK with local genuine Android/JDK17 metadata"]
fn samsung_wave11_original_boolean_parameter_witness() {
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
        (
            6875,
            "Landroidx/appcompat/widget/ButtonBarLayout;",
            "setStacked",
        ),
        (5890, "Landroidx/appcompat/app/y;", "L"),
        (
            7146,
            "Landroidx/appcompat/widget/SeslDropDownItemTextView;",
            "setChecked",
        ),
    ] {
        let key = method_key(dex, method_index).unwrap();
        assert_eq!(key.owner, expected_owner);
        assert_eq!(key.name, expected_name);
        assert_eq!(key.parameters, ["Z"]);
        assert_eq!(key.return_type, "V");
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
        let declarations =
            backend_java::contextual_locals::declaration_plan(ir, &JavaType::Void, &resolver)
                .unwrap();
        let mut dump = format!(
            "key={key:?}\nregisters={} incoming={}\ndeclarations={declarations:?}\n",
            code.registers_size, code.ins_size
        );
        for (id, insn) in ir.instructions() {
            dump.push_str(&format!("{id:?} offset={:?} {insn:?}\n", ssa.offset(id)));
        }
        for (id, value) in ir.values() {
            dump.push_str(&format!(
                "{id:?} {value:?} declaration={:?}\n",
                declarations.get(&id)
            ));
        }
        std::fs::write("/tmp/opencode/wave11-original-boolean-ir.txt", dump).unwrap();
        let mut members = BTreeSet::new();
        for (_, insn) in ir.instructions() {
            match insn {
                ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                    let field = resolver.field(*field).unwrap();
                    members.insert(format!(
                        "F|{}|{}|{}|{}",
                        field.owner,
                        field.name,
                        field.is_static,
                        java_type(&field.ty)
                    ));
                }
                ManagedInsn::Invoke { method, .. } => {
                    let method = resolver.method(*method).unwrap();
                    members.insert(format!(
                        "M|{}|{}|{}|{}|{}",
                        method.owner,
                        method.name,
                        method.is_static,
                        java_type(&method.return_type),
                        method
                            .parameters
                            .iter()
                            .map(java_type)
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
                _ => {}
            }
        }
        let required: BTreeSet<_> = members
            .iter()
            .flat_map(|line| {
                line.split('|')
                    .skip(1)
                    .flat_map(|part| part.split(','))
                    .map(|part| part.trim_end_matches("[]").to_owned())
            })
            .filter(|name| name.contains('.'))
            .collect();
        let mut pending: Vec<_> = required.into_iter().collect();
        let mut seen = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(Some(class)) = resolver.class_catalog.get(&name) {
                members.insert(format!(
                    "C|{}|{}",
                    class.name,
                    class.superclass.as_deref().unwrap_or("java.lang.Object")
                ));
                if let Some(parent) = &class.superclass {
                    pending.push(parent.clone());
                }
            }
        }
        std::fs::write(
            "/tmp/opencode/wave11-original-boolean-boundaries.txt",
            members.into_iter().collect::<Vec<_>>().join("\n"),
        )
        .unwrap();
        let source = method_source(class, method, &resolver, ClassKind::Class, &mut None)
            .unwrap_or_else(|error| {
                panic!("original {expected_owner}->{expected_name} rejected: {error}")
            });
        assert!(source.body);
        assert!(source.source.contains("? 1 : 0"), "{}", source.source);
        let path = format!("/tmp/opencode/wave11-original-{method_index}.java");
        std::fs::write(&path, &source.source).unwrap();
        println!(
            "ORIGINAL ACCEPTED classes.dex:{method_index} owner={expected_owner} name={expected_name} bytes={} path={path}",
            source.source.len()
        );
    }
}
