#![allow(clippy::unwrap_used)]
use super::*;

#[test]
#[ignore = "bounded original Samsung CharacterReader with genuine Android/JDK metadata"]
fn character_reader_contextual_phi_evidence() {
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
    let dex = &files[3];
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let mut resolver = Resolver::with_catalog(dex, &aliases, &metadata, Some(&catalog));
    resolver.inputs = &files;
    resolver.dex_index = 3;
    resolver.external = Some(&external);
    resolver.hierarchy = crate::java_hierarchy::build_hierarchy(
        &files.iter().collect::<Vec<_>>(),
        external.hierarchy(),
    )
    .unwrap()
    .map_names(|n| {
        aliases
            .class_name(&format!("L{};", n.replace('.', "/")))
            .unwrap_or(n)
            .to_owned()
    })
    .unwrap();
    resolver.input_owners = files
        .iter()
        .flat_map(|dex| {
            dex.classes
                .iter()
                .filter_map(|class| descriptor(dex, class.class_idx).map(str::to_owned))
        })
        .collect();
    for declaration in external.hierarchy() {
        let descriptor = format!("L{};", declaration.name.replace('.', "/"));
        if let Some(access_flags) = external.class_access(&descriptor) {
            let name = aliases
                .class_name(&descriptor)
                .unwrap_or(&declaration.name)
                .to_owned();
            resolver.class_catalog.insert(
                name.clone(),
                Some(backend_java::constructors::ConstructorClass {
                    name,
                    superclass: declaration.superclass.map(|s| {
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
        for class in &dex.classes {
            if let Some(name) = descriptor(dex, class.class_idx).and_then(|d| aliases.class_name(d))
            {
                resolver.class_catalog.insert(
                    name.to_owned(),
                    Some(backend_java::constructors::ConstructorClass {
                        name: name.to_owned(),
                        superclass: descriptor(dex, class.superclass_idx)
                            .and_then(|d| aliases.class_name(d))
                            .map(str::to_owned),
                        access_flags: class.access_flags,
                    }),
                );
            }
        }
    }
    let method = dex
        .classes
        .iter()
        .filter_map(|c| c.class_data.as_ref())
        .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
        .find(|m| m.method_idx == 31531)
        .unwrap();
    assert_eq!(method_key(dex, method.method_idx).unwrap().name, "bufferUp");
    let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
    let ssa = cached_frontend(
        &resolver,
        method,
        method.code.as_ref().unwrap(),
        &ctx,
        seeds,
        None,
    )
    .unwrap();
    let ir = ssa.ir();
    let java = resolver.method(MethodRef::new(method.method_idx)).unwrap();
    let plan = backend_java::contextual_locals::declaration_plan(ir, &java.return_type, &resolver)
        .unwrap();
    for (id, insn) in ir.instructions() {
        if let ManagedInsn::Phi { result, .. } = insn {
            println!(
                "PHI {result:?} off={:?} {insn:?} declaration={:?} checked={:?}",
                ssa.offset(id),
                plan.get(result),
                backend_java::contextual_locals::checked_phi_plan(
                    ir,
                    *result,
                    &JavaType::Int,
                    &java.return_type,
                    &resolver.hierarchy,
                    &resolver
                )
            );
        }
    }
    for (id, insn) in ir.instructions() {
        println!("INSN {id:?} off={:?} {insn:?}", ssa.offset(id));
    }
    let loop_phi = ir.instructions().iter().find_map(|(_, insn)| {
        let ManagedInsn::Phi { result, inputs } = insn else { return None; };
        inputs.iter().any(|(_, input)| {
            let definition = ir.value(*input).unwrap().def();
            matches!(ir.instruction(definition).unwrap(),
                ManagedInsn::Arithmetic { op: managed_ir::ops::ArithOp::Add, lhs, .. } if lhs == result)
        }).then_some(*result)
    }).unwrap();
    assert_eq!(plan.get(&loop_phi), Some(&JavaType::Int));
    let class = dex
        .classes
        .iter()
        .find(|c| {
            c.class_data.as_ref().is_some_and(|d| {
                d.direct_methods
                    .iter()
                    .chain(&d.virtual_methods)
                    .any(|m| m.method_idx == method.method_idx)
            })
        })
        .unwrap();
    let candidate = method_source(class, method, &resolver, ClassKind::Class, &mut None)
        .unwrap_or_else(|error| panic!("original bufferUp body must pass production: {error}"));
    assert!(candidate.body);
    assert!(candidate.source.contains("private void bufferUp()"));
    println!(
        "ORIGINAL BODY ACCEPTED index={} bytes={} loop_phi={loop_phi:?}",
        method.method_idx,
        candidate.source.len()
    );
    std::fs::write(
        "/tmp/opencode/character-reader-wave5-original-body.java",
        candidate.source,
    )
    .unwrap();
}
