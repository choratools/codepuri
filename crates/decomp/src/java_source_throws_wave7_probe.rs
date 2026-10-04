//! Read-only original checked-contract evidence; never changes emitted sources.
#![allow(clippy::unwrap_used)]
use super::*;

#[test]
#[ignore = "bounded frozen-wave6 original source-contract diagnosis; no export"]
fn frozen_wave6_requested_checked_contract_roots() {
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
    let targets = [
        (1, 39995),
        (1, 39939),
        (0, 20281),
        (0, 20283),
        (0, 20284),
        (0, 20288),
        (0, 20294),
        (0, 20295),
        (0, 5339),
    ];
    let roots = targets
        .iter()
        .map(|(pool, index)| method_key(&files[*pool], *index).unwrap())
        .collect();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    let super_inputs = files.iter().collect::<Vec<_>>();
    let super_index =
        crate::java_super_resolution::SuperIndex::collect(&super_inputs, &external).unwrap();
    println!(
        "PLAN methods={} unavailable={}",
        plan.methods.len(),
        plan.unavailable.len()
    );
    for (pool, index) in targets {
        let key = method_key(&files[pool], index).unwrap();
        let (_, method) = facts.definitions[&key];
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[pool]);
        let mut resolver =
            Resolver::with_catalog(&files[pool], &aliases, &metadata, Some(&catalog));
        resolver.inputs = &files;
        resolver.dex_index = pool;
        resolver.external = Some(&external);
        resolver.input_owners = facts.owners.keys().cloned().collect();
        resolver.hierarchy = hierarchy.clone();
        resolver.super_index = Some(&super_index);
        resolver.source_throws = Some(&plan);
        let context = Context {
            resolver: &resolver,
            facts: &facts,
        };
        let declared = context
            .invocation_contract(MethodRef::new(index))
            .unwrap()
            .declared;
        let contract = plan.methods.get(&key);
        println!(
            "ROOT {pool}:{index} {key:?} flags={:#x} declared={declared:?} derived={:?} source={:?} unavailable={:?}",
            method.access_flags,
            contract.map(|c| &c.derived),
            contract.map(|c| &c.source),
            plan.unavailable.get(&key)
        );
        let identity = OriginalMethodIdentity {
            dex: pool,
            index,
            key: key.clone(),
        };
        println!(
            "OVERRIDES {pool}:{index} {:?}",
            context.override_contract(&identity)
        );
        let code = method.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
        let ssa = cached_frontend(&resolver, method, code, &ctx, seeds, None).unwrap();
        let local = required::inspect_method(
            identity.clone(),
            &ssa,
            code,
            method.access_flags,
            declared,
            &context,
        );
        println!(
            "LOCAL {pool}:{index} {:?}",
            local
                .as_ref()
                .map(|m| m.source_targets().collect::<Vec<_>>())
        );
        let caller = resolver.method(MethodRef::new(index)).unwrap();
        for (id, instruction) in ssa.ir().instructions() {
            let ManagedInsn::Invoke {
                method: reference,
                kind,
                ..
            } = instruction
            else {
                continue;
            };
            let target_key = method_key(&files[pool], reference.index()).unwrap();
            let original = context.invoked_contract(&identity, *kind, *reference);
            let target = resolver.method(*reference);
            let required_target = original.as_ref().and_then(|c| c.source_target.as_ref());
            let target_error = required_target.and_then(|key| plan.unavailable.get(key));
            let rejected = target.as_ref().is_some_and(|target| {
                backend_java::checked_exceptions::check(&resolver, &target.throws, &caller.throws)
                    .is_err()
            });
            if rejected || target_error.is_some() || original.is_none() {
                let offset = ssa.offset(id).unwrap();
                let scopes = code
                    .tries
                    .iter()
                    .filter(|t| {
                        t.start_addr <= offset && offset < t.start_addr + u32::from(t.insn_count)
                    })
                    .map(|t| {
                        code.handlers
                            .iter()
                            .find(|h| h.relative_offset == u32::from(t.handler_off))
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                println!(
                    "EDGE {pool}:{index} invoke@{offset} kind={kind:?} target={target_key:?} original={original:?} emitted_throws={:?} caller_throws={:?} target_error={target_error:?} rejected={rejected} protected={scopes:?}",
                    target.as_ref().map(|m| &m.throws),
                    caller.throws
                );
            }
        }
        let mut current = key;
        let mut seen = BTreeSet::new();
        while seen.insert(current.clone()) {
            let Some(error) = plan.unavailable.get(&current) else {
                break;
            };
            println!("FAILURE_CHAIN {pool}:{index} {current:?} {error:?}");
            match error {
                required::ThrowsError::MissingSourceTarget(next) => current = next.clone(),
                _ => break,
            }
        }
    }
}

#[test]
#[ignore = "bounded Function0 contributor and Room captured-object provenance evidence"]
fn frozen_wave6_function0_and_room_handler_evidence() {
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

    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    let input_owners: BTreeSet<_> = facts.owners.keys().cloned().collect();
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
            resolver.input_owners = input_owners.clone();
            resolver.hierarchy = hierarchy.clone();
            resolver
        })
        .collect();
    let key = MethodKey {
        owner: "Lxq/a;".into(),
        name: "invoke".into(),
        parameters: vec![],
        return_type: "Ljava/lang/Object;".into(),
    };
    let mut failures = 0;
    println!(
        "FUNCTION0 candidates={} alias={:?}",
        facts.implementations.get(&key).len(),
        facts.emitted_method_name(&key)
    );
    for candidate in facts.implementations.get(&key) {
        let (pool, method) = facts.definitions[candidate];
        let context = Context {
            resolver: &resolvers[pool],
            facts: &facts,
        };
        let identity = OriginalMethodIdentity {
            dex: pool,
            index: method.method_idx,
            key: candidate.clone(),
        };
        let contract = context.override_contract(&identity);
        if matches!(&contract, OverrideContract::Declarations(ds) if ds.iter().any(|d| d.key == key))
        {
            continue;
        }
        failures += 1;
        if failures <= 5 {
            println!(
                "BAD_IMPL {pool}:{} {candidate:?} flags={:#x} alias={:?} override={contract:?}",
                method.method_idx,
                method.access_flags,
                facts.emitted_method_name(candidate)
            );
            let mut pending = vec![candidate.owner.clone()];
            let mut visited = BTreeSet::new();
            while let Some(owner) = pending.pop() {
                if !visited.insert(owner.clone()) {
                    continue;
                }
                match facts.class(&owner) {
                    Some(class) => {
                        let relevant: Vec<_> = class
                            .methods
                            .iter()
                            .filter(|(k, _)| {
                                k.name == candidate.name && k.parameters == candidate.parameters
                            })
                            .map(|(k, m)| (k, m, facts.emitted_method_name(k)))
                            .collect();
                        println!(
                            "ANCESTOR {owner} parent={:?} interfaces={:?} relevant={relevant:?}",
                            class.parent, class.interfaces
                        );
                        pending.extend(
                            class
                                .parent
                                .clone()
                                .into_iter()
                                .chain(class.interfaces.iter().cloned()),
                        );
                    }
                    None => println!("ANCESTOR {owner} MISSING"),
                }
            }
        }
    }
    println!("FUNCTION0 invalid_contributors={failures}");
    for index in [20294, 20295] {
        let method_key = method_key(&files[0], index).unwrap();
        let (_, method) = facts.definitions[&method_key];
        let resolver = &resolvers[0];
        let code = method.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(resolver, method).unwrap();
        let ssa = cached_frontend(resolver, method, code, &ctx, seeds, None).unwrap();
        let context = Context {
            resolver,
            facts: &facts,
        };
        let required = required::inspect_method(
            OriginalMethodIdentity {
                dex: 0,
                index,
                key: method_key,
            },
            &ssa,
            code,
            method.access_flags,
            vec![],
            &context,
        )
        .unwrap();
        println!(
            "ROOM {index} handlers={:?} source_targets={:?}",
            code.handlers,
            required.source_targets().collect::<Vec<_>>()
        );
        for (id, insn) in ssa.ir().instructions() {
            if let ManagedInsn::Throw { value } = insn {
                let record = ssa.ir().value(*value).unwrap();
                println!(
                    "ROOM_THROW {index} offset={:?} value={value:?} ty={:?} def={:?}",
                    ssa.offset(id),
                    record.ty(),
                    ssa.ir().instruction(record.def()).unwrap()
                );
                if let ManagedInsn::Move { value: src, .. } =
                    ssa.ir().instruction(record.def()).unwrap()
                {
                    let origin = ssa.ir().value(*src).unwrap();
                    println!(
                        "ROOM_THROW_ORIGIN {index} value={src:?} ty={:?} def={:?}",
                        origin.ty(),
                        ssa.ir().instruction(origin.def()).unwrap()
                    );
                }
            }
        }
    }
}
