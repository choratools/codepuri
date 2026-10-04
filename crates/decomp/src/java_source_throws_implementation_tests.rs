#![allow(clippy::unwrap_used)]
use super::*;
use crate::type_compatibility::{Declaration, Hierarchy};

fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("Base", Some("java.lang.Object")),
        ("Child", Some("Base")),
        ("Other", Some("java.lang.Object")),
        ("CycleA", Some("CycleB")),
        ("CycleB", Some("CycleA")),
    ] {
        h.insert(Declaration {
            name: name.into(),
            superclass: parent.map(str::to_owned),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    h
}
#[test]
fn implementation_candidate_distinct_known_alias_is_positive_nonoverride_evidence() {
    assert!(proven_nonimplementation(
        Some("invoke__2"),
        Some("invoke"),
        None,
        None,
        &hierarchy()
    ));
    // Unavailable names provide no name contradiction, even for identical types.
    for names in [(None, Some("invoke")), (Some("invoke"), None), (None, None)] {
        assert!(!proven_nonimplementation(
            names.0,
            names.1,
            Some(&JavaType::Int),
            Some(&JavaType::Int),
            &hierarchy()
        ));
    }
}
#[test]
fn implementation_candidate_primitive_returns_require_identity_without_widening() {
    let h = hierarchy();
    for (actual, expected) in [
        (JavaType::Void, JavaType::Class("java.lang.Object".into())),
        (JavaType::Int, JavaType::Long),
        (JavaType::Byte, JavaType::Int),
        (JavaType::Boolean, JavaType::Int),
        (JavaType::Int, JavaType::Void),
    ] {
        assert!(proven_nonimplementation(
            Some("invoke"),
            Some("invoke"),
            Some(&actual),
            Some(&expected),
            &h
        ));
    }
    assert!(!proven_nonimplementation(
        Some("invoke"),
        Some("invoke"),
        Some(&JavaType::Int),
        Some(&JavaType::Int),
        &h
    ));
    assert!(!proven_nonimplementation(
        Some("invoke"),
        Some("invoke"),
        None,
        Some(&JavaType::Int),
        &h
    ));
}
#[test]
fn implementation_candidate_covariance_retains_unknown_and_cyclic_hierarchies() {
    let h = hierarchy();
    for (actual, expected, excluded) in [
        ("Child", "Base", false),
        ("Base", "Child", true),
        ("Other", "Base", true),
        ("Missing", "Base", false),
        ("CycleA", "Base", false),
    ] {
        assert_eq!(
            proven_nonimplementation(
                Some("invoke"),
                Some("invoke"),
                Some(&JavaType::Class(actual.into())),
                Some(&JavaType::Class(expected.into())),
                &h
            ),
            excluded
        );
    }
    let child_array = JavaType::Array(Box::new(JavaType::Class("Child".into())));
    let base_array = JavaType::Array(Box::new(JavaType::Class("Base".into())));
    assert!(!proven_nonimplementation(
        Some("invoke"),
        Some("invoke"),
        Some(&child_array),
        Some(&base_array),
        &h
    ));
    assert!(proven_nonimplementation(
        Some("invoke"),
        Some("invoke"),
        Some(&base_array),
        Some(&child_array),
        &h
    ));
    assert!(proven_nonimplementation(
        Some("invoke"),
        Some("invoke"),
        Some(&JavaType::Array(Box::new(JavaType::Byte))),
        Some(&JavaType::Array(Box::new(JavaType::Int))),
        &h
    ));
}

#[test]
fn implementation_candidate_unavailable_inherited_alias_keeps_override_unknown() {
    let dex = DexFile::from_bytes(&super::super::external_fixture::minimal_dex()).unwrap();
    let files = vec![dex];
    let key = MethodKey {
        owner: "LChild;".into(),
        name: "invoke".into(),
        parameters: vec![],
        return_type: "V".into(),
    };
    let inherited = MethodKey {
        owner: "LParent;".into(),
        ..key.clone()
    };
    let mut inventory = AliasInventory::default();
    for owner in ["LChild;", "LParent;", "Ljava/lang/Object;"] {
        inventory.add_type(owner).unwrap();
    }
    inventory.add_method(key.clone()).unwrap();
    let aliases = inventory.build(AliasMode::Ascii);
    let catalog = BTreeMap::new();
    let mut facts = Facts::new(&files, &aliases, &catalog, None);
    facts.owners.insert("LParent;".into(), Some((0, 0)));
    let declaration = RawClass {
        parent: Some("Ljava/lang/Object;".into()),
        interfaces: vec![],
        methods: vec![(
            inherited.clone(),
            MethodMetadata {
                access_flags: 1,
                exceptions: Some(vec![]),
            },
        )],
    };
    {
        let mut classes = facts.classes.lock().unwrap();
        classes.insert(
            "LChild;".into(),
            Some(Arc::new(RawClass {
                parent: Some("LParent;".into()),
                interfaces: vec![],
                methods: vec![],
            })),
        );
        classes.insert("LParent;".into(), Some(Arc::new(declaration)));
        classes.insert(
            "Ljava/lang/Object;".into(),
            Some(Arc::new(RawClass {
                parent: None,
                interfaces: vec![],
                methods: vec![],
            })),
        );
    }
    let first = facts.class("LParent;").unwrap();
    let again = facts.class("LParent;").unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(first.methods[0].0, inherited);
    assert_eq!(first.methods[0].1.exceptions, Some(vec![]));
    let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
    let resolver = Resolver::with_catalog(&files[0], &aliases, &metadata, Some(&catalog));
    let context = Context {
        resolver: &resolver,
        facts: &facts,
    };
    assert!(facts.emitted_method_name(&inherited).is_none());
    assert!(!context.nonimplementation(&key, &inherited));
    assert!(matches!(
        context.override_contract(&OriginalMethodIdentity {
            dex: 0,
            index: u32::MAX,
            key
        }),
        OverrideContract::Unknown
    ));
}

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
#[ignore = "original APK bridge/alias admission and source-contract recovery; no export"]
fn implementation_candidate_original_function0_bridges_and_requested_contracts() {
    let (files, aliases, catalog, external, hierarchy) = original_fixture();
    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    let metadata: Vec<_> = files
        .iter()
        .map(dex_decompiler::infer::DexMetadata::new)
        .collect();
    let resolvers: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(pool, dex)| {
            let mut resolver =
                Resolver::with_catalog(dex, &aliases, &metadata[pool], Some(&catalog));
            resolver.inputs = &files;
            resolver.dex_index = pool;
            resolver.external = Some(&external);
            resolver.input_owners = facts.owners.keys().cloned().collect();
            resolver.hierarchy = hierarchy.clone();
            resolver
        })
        .collect();
    let resolver = &resolvers[0];
    let context = Context {
        resolver,
        facts: &facts,
    };
    let target = MethodKey {
        owner: "Lxq/a;".into(),
        name: "invoke".into(),
        parameters: vec![],
        return_type: "Ljava/lang/Object;".into(),
    };
    let mut excluded = 0;
    let mut unknown = vec![];
    for candidate in facts.implementations.get(&target) {
        let (pool, method) = facts.definitions[candidate];
        let candidate_context = Context {
            resolver: &resolvers[pool],
            facts: &facts,
        };
        if candidate_context.nonimplementation(candidate, &target) {
            excluded += 1;
            continue;
        }
        let override_contract = candidate_context.override_contract(&OriginalMethodIdentity {
            dex: pool,
            index: method.method_idx,
            key: candidate.clone(),
        });
        if !matches!(&override_contract,OverrideContract::Declarations(ds) if ds.iter().any(|d|d.key == target))
        {
            unknown.push((
                pool,
                method.method_idx,
                candidate.clone(),
                override_contract,
            ));
        }
    }
    println!(
        "ADMISSION total={} proven_nonoverrides={excluded} retained_unknown={unknown:?}",
        facts.implementations.get(&target).len()
    );
    for index in [5100, 44159] {
        let candidate = method_key(&files[0], index).unwrap();
        assert!(context.nonimplementation(&candidate, &target));
        assert_eq!(
            facts.emitted_method_name(&candidate).as_deref(),
            Some("invoke__2")
        );
        assert!(matches!(
            context.override_contract(&OriginalMethodIdentity {
                dex: 0,
                index,
                key: candidate.clone()
            }),
            OverrideContract::NoInheritedDeclaration
        ));
        let (bridge, (_, method)) = facts
            .definitions
            .iter()
            .find(|(k, _)| {
                k.owner == candidate.owner
                    && k.name == candidate.name
                    && k.parameters == candidate.parameters
                    && k.return_type == target.return_type
            })
            .unwrap();
        assert_eq!(method.access_flags, 0x1041);
        assert!(!context.nonimplementation(bridge, &target));
        assert!(
            matches!(context.override_contract(&OriginalMethodIdentity { dex:0,index:method.method_idx,key:bridge.clone() }),OverrideContract::Declarations(ds) if ds.iter().any(|d|d.key == target))
        );
        let code = method.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(resolver, method).unwrap();
        let ssa = cached_frontend(resolver, method, code, &ctx, seeds, None).unwrap();
        let helper_calls: Vec<_> = ssa
            .ir()
            .instructions()
            .iter()
            .filter_map(|(id, insn)| {
                let ManagedInsn::Invoke { method, .. } = insn else {
                    return None;
                };
                let called = method_key(&files[0], method.index()).unwrap();
                (called == candidate).then_some(ssa.offset(id).unwrap())
            })
            .collect();
        assert_eq!(helper_calls.len(), 1);
        println!(
            "ORIGINAL_ALIAS typed#{index} {candidate:?} bridge#{} {bridge:?} exact_helper_invoke_offsets={helper_calls:?}",
            method.method_idx,
        );
    }
    let roots: BTreeSet<_> = [
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
    .chain([target.clone()])
    .collect();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    for index in [5099, 5100, 44158, 44159] {
        let key = method_key(&files[0], index).unwrap();
        assert_ne!(
            plan.methods.contains_key(&key),
            plan.unavailable.contains_key(&key)
        );
        if let Some(contract) = plan.methods.get(&key) {
            assert!(contract.original.is_empty());
        }
        println!(
            "EXACT_SOURCE {index} {key:?} contract={:?} unavailable={:?}",
            plan.methods.get(&key),
            plan.unavailable.get(&key),
        );
    }
    assert_ne!(
        plan.unavailable.get(&target),
        Some(&required::ThrowsError::Unknown(
            "abstract implementation inherited contract".into()
        ))
    );
    for key in &roots {
        assert_ne!(
            plan.methods.contains_key(key),
            plan.unavailable.contains_key(key)
        );
        println!(
            "RECOVERY {key:?} contract={:?} failure={:?}",
            plan.methods.get(key),
            plan.unavailable.get(key)
        );
        let mut current = key;
        let mut seen = BTreeSet::new();
        while seen.insert(current.clone()) {
            let Some(error) = plan.unavailable.get(current) else {
                break;
            };
            println!("RECOVERY_CHAIN {current:?} {error:?}");
            match error {
                required::ThrowsError::MissingSourceTarget(next) => current = next,
                _ => break,
            }
        }
    }
    assert!(excluded >= 2);
    assert!(matches!(
        plan.unavailable.get(&method_key(&files[0], 5339).unwrap()),
        Some(required::ThrowsError::OverrideConflict(_))
    ));
}
