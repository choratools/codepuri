#![allow(clippy::unwrap_used)]
use super::*;
use crate::type_compatibility::Hierarchy;
type Fixture = (
    Vec<DexFile>,
    JavaAliases,
    BTreeMap<MethodKey, Option<MethodMetadata>>,
    crate::java_external_metadata::Classpath,
    Hierarchy,
);
fn original_fixture() -> Fixture {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
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
    let hierarchy = crate::java_hierarchy::build_hierarchy(
        &files.iter().collect::<Vec<_>>(),
        external.hierarchy(),
    )
    .unwrap()
    .map_names(|n| {
        aliases
            .class_name(&format!("L{};", n.replace('.', "/")))
            .unwrap_or(n)
            .into()
    })
    .unwrap();
    let catalog = method_catalog(&files);
    (files, aliases, catalog, external, hierarchy)
}

#[test]
#[ignore = "exact original shared handler and source contracts with JDK17/Android35; no export"]
fn shared_handler_original_reflectutil_source_plan_and_requested_roots() {
    let (files, aliases, catalog, external, hierarchy) = original_fixture();
    let key = method_key(&files[1], 23703).unwrap();
    assert_eq!(
        key.owner,
        "Lcom/samsung/android/samsungpass/base/common/utils/ReflectUtil;"
    );
    assert_eq!(key.name, "getField");
    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    let (pool, method) = facts.definitions[&key];
    assert_eq!(pool, 1);
    let metadata = dex_decompiler::infer::DexMetadata::new(&files[pool]);
    let mut resolver = Resolver::with_catalog(&files[pool], &aliases, &metadata, Some(&catalog));
    resolver.inputs = &files;
    resolver.dex_index = pool;
    resolver.external = Some(&external);
    resolver.input_owners = facts.owners.keys().cloned().collect();
    resolver.hierarchy = hierarchy.clone();
    let context = Context {
        resolver: &resolver,
        facts: &facts,
    };
    let original = context
        .invocation_contract(MethodRef::new(23703))
        .unwrap()
        .declared;
    assert!(original.is_empty());
    assert_eq!(method.access_flags, 9);
    let code = method.code.as_ref().unwrap();
    assert_eq!(code.tries.len(), 1);
    assert_eq!(code.tries[0].start_addr, 4);
    assert_eq!(code.tries[0].insn_count, 4);
    let handler = &code.handlers[0];
    assert_eq!(handler.handlers.len(), 2);
    assert_eq!(handler.handlers[0].address, 9);
    assert_eq!(handler.handlers[1].address, 9);
    assert!(handler.catch_all_addr.is_none());
    assert_eq!(
        context
            .class(ClassRef::new(handler.handlers[0].type_idx))
            .as_deref(),
        Some("java.lang.NoSuchFieldException")
    );
    assert_eq!(
        context
            .class(ClassRef::new(handler.handlers[1].type_idx))
            .as_deref(),
        Some("java.lang.SecurityException")
    );
    let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
    let ssa = cached_frontend(&resolver, method, code, &ctx, seeds, None).unwrap();
    assert!(
        !ssa.ir()
            .instructions()
            .values()
            .any(|i| matches!(i, ManagedInsn::Throw { .. }))
    );
    let local = required::inspect_method(
        OriginalMethodIdentity {
            dex: pool,
            index: 23703,
            key: key.clone(),
        },
        &ssa,
        code,
        method.access_flags,
        original,
        &context,
    )
    .unwrap();
    println!(
        "LOCAL_REFLECTUTIL {key:?} source_targets={:?}",
        local.source_targets().collect::<Vec<_>>()
    );
    let roots: BTreeSet<_> = [
        (1, 23703),
        (1, 39995),
        (1, 39939),
        (0, 20281),
        (0, 20283),
        (0, 20284),
        (0, 20288),
        (0, 20294),
        (0, 20295),
        (0, 5339),
    ]
    .into_iter()
    .map(|(pool, index)| method_key(&files[pool], index).unwrap())
    .collect();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    for key in &roots {
        assert_ne!(
            plan.methods.contains_key(key),
            plan.unavailable.contains_key(key)
        );
        println!(
            "SHARED_SOURCE {key:?} contract={:?} unavailable={:?}",
            plan.methods.get(key),
            plan.unavailable.get(key)
        );
        let mut current = key;
        let mut seen = BTreeSet::new();
        while seen.insert(current.clone()) {
            let Some(error) = plan.unavailable.get(current) else {
                break;
            };
            println!("SHARED_CHAIN {current:?} {error:?}");
            match error {
                required::ThrowsError::MissingSourceTarget(next) => current = next,
                _ => break,
            }
        }
    }
    assert_ne!(
        plan.unavailable.get(&key),
        Some(&required::ThrowsError::Unknown(
            "opaque throwing effects".into()
        ))
    );
    if let Some(contract) = plan.methods.get(&key) {
        assert!(contract.original.is_empty());
    }
}
