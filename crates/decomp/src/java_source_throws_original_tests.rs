#![allow(clippy::unwrap_used)]
use super::*;
use managed_ir::types::ManagedType;

type Fixture = (
    Vec<DexFile>,
    JavaAliases,
    BTreeMap<MethodKey, Option<MethodMetadata>>,
    crate::java_external_metadata::Classpath,
    crate::type_compatibility::Hierarchy,
);
fn fixture() -> Fixture {
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
#[ignore = "bounded original Samsung Pass with explicit JDK17/Android35 classpath"]
fn samsung_checked_contract_blocker_diagnostic() {
    let (files, aliases, catalog, external, hierarchy) = fixture();
    let indices = [141, 46, 54, 55, 590];
    let roots = indices
        .map(|index| method_key(&files[0], index).unwrap())
        .into_iter()
        .collect();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    for index in indices {
        let key = method_key(&files[0], index).unwrap();
        println!(
            "ROOT {index} {key:?} source={:?} error={:?}",
            plan.methods.get(&key).map(|v| &v.source),
            plan.unavailable.get(&key)
        );
    }
    for (key, error) in plan.unavailable.iter().take(45) {
        println!("DEPENDENCY {key:?} {error:?}");
    }
    let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
    let mut resolver = Resolver::with_catalog(&files[0], &aliases, &metadata, Some(&catalog));
    resolver.inputs = &files;
    resolver.external = Some(&external);
    resolver.hierarchy = hierarchy;
    for index in [141, 55] {
        let method = files[0]
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
        let ssa = cached_frontend(&resolver, method, code, &ctx, seeds, None).unwrap();
        println!(
            "SSA {index} tries={:?} handlers={:?}",
            code.tries, code.handlers
        );
        for (id, i) in ssa.ir().instructions() {
            if matches!(
                i,
                ManagedInsn::Throw { .. } | ManagedInsn::Opaque { .. } | ManagedInsn::Invoke { .. }
            ) {
                println!("INSN {index} {:?} {i:?}", ssa.offset(id));
            }
        }
    }
}

#[test]
#[ignore = "five original seeded Samsung methods with explicit JDK17/Android35 classpath"]
fn samsung_precise_catch_all_seeded_original_contracts() {
    let (files, aliases, catalog, external, hierarchy) = fixture();
    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    let super_inputs: Vec<_> = files.iter().collect();
    let super_index =
        crate::java_super_resolution::SuperIndex::collect(&super_inputs, &external).unwrap();
    let targets = [
        ("La2/a;", "n"),
        ("La4/a$c;", "k"),
        (
            "Lcom/google/firebase/perf/session/SessionManager;",
            "unregisterForSessionUpdates",
        ),
        ("Landroidx/fragment/app/z0;", "values"),
        ("Landroidx/fragment/app/e$a;", "d"),
    ];
    let mut rejected = Vec::new();
    for (owner, name) in targets {
        let (key, (pool, method)) = facts
            .definitions
            .iter()
            .find(|(key, _)| key.owner == owner && key.name == name)
            .unwrap();
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[*pool]);
        let mut resolver =
            Resolver::with_catalog(&files[*pool], &aliases, &metadata, Some(&catalog));
        resolver.inputs = &files;
        resolver.dex_index = *pool;
        resolver.external = Some(&external);
        resolver.hierarchy = hierarchy.clone();
        resolver.super_index = Some(&super_index);
        let context = Context {
            resolver: &resolver,
            facts: &facts,
        };
        let code = method.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
        let ssa = cached_frontend(&resolver, method, code, &ctx, seeds, None).unwrap();
        let original = context
            .invocation_contract(MethodRef::new(method.method_idx))
            .unwrap()
            .declared;
        let result = required::inspect_method(
            OriginalMethodIdentity {
                dex: *pool,
                index: method.method_idx,
                key: key.clone(),
            },
            &ssa,
            code,
            method.access_flags,
            original.clone(),
            &context,
        );
        println!(
            "SEEDED {pool}:{} {key:?} original={original:?} => {:?}",
            method.method_idx,
            result
                .as_ref()
                .map(|r| r.source_targets().collect::<Vec<_>>())
        );
        match result {
            Ok(requirements) if key.owner == "La2/a;" => {
                assert!(original.is_empty());
                let plan = required::derive_program(&[requirements], &context).unwrap();
                assert!(plan.unavailable.is_empty(), "{:?}", plan.unavailable);
                assert_eq!(
                    plan.methods[key].source,
                    ["java.util.concurrent.ExecutionException"]
                );
                println!(
                    "PRECISE n(Future) rethrow@25 = ExecutionException; InterruptedException excluded by original earlier handler26"
                );
            }
            Ok(requirements) if key.owner == "Landroidx/fragment/app/z0;" => {
                assert_eq!(method.method_idx, 14681);
                assert!(original.is_empty());
                let plan = required::derive_program(&[requirements], &context).unwrap();
                assert!(plan.unavailable.is_empty(), "{:?}", plan.unavailable);
                assert!(plan.methods[key].source.is_empty());
                let clone = ssa
                    .ir()
                    .instructions()
                    .iter()
                    .find_map(|(id, i)| match i {
                        ManagedInsn::Invoke {
                            method,
                            args,
                            kind: InvokeKind::Virtual,
                            ..
                        } if ssa.offset(id) == Some(2) && args.len() == 1 => {
                            Some((*method, args[0]))
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(matches!(
                    ssa.ir().value(clone.1).unwrap().ty(),
                    ManagedType::Array { .. }
                ));
                assert_eq!(
                    resolver.declaration(clone.0.index()).unwrap().exceptions,
                    Some(vec!["Ljava/lang/CloneNotSupportedException;".into()])
                );
                println!(
                    "PRECISE values #14681 clone@2 checked=[]; genuine Object.clone metadata retains CloneNotSupportedException"
                );
            }
            Ok(requirements) if key.owner == "Landroidx/fragment/app/e$a;" => {
                assert_eq!(method.method_idx, 14133);
                assert!(original.is_empty());
                assert!(
                    requirements
                        .source_targets()
                        .any(|key| key.owner == "Landroidx/fragment/app/e$b;" && key.name == "c")
                );
                println!(
                    "LOSS #14133 d(ViewGroup) retains exact e$b.c(Context) source dependency; its enum-clone root #14681 is checked-empty above"
                );
            }
            Ok(_) => {}
            Err(error) => {
                println!("FAILED handlers={:?}", code.handlers);
                rejected.push((key.clone(), error));
            }
        }
    }
    assert!(rejected.is_empty(), "{rejected:?}");
}

#[test]
#[ignore = "bounded wave4 loss diagnosis against original Samsung/JDK17/Android35"]
fn samsung_wave4_checked_contract_losses() {
    let (files, aliases, catalog, external, hierarchy) = fixture();
    let indices = [24570, 25181];
    let roots = indices
        .map(|index| method_key(&files[0], index).unwrap())
        .into_iter()
        .collect();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    for index in indices {
        let key = method_key(&files[0], index).unwrap();
        println!(
            "WAVE4 LOSS #{index} {key:?} source={:?} unavailable={:?}",
            plan.methods.get(&key).map(|v| &v.source),
            plan.unavailable.get(&key)
        );
    }
    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    for (owner, name, expected) in [
        (
            "Lcom/bumptech/glide/load/data/k;",
            "a",
            vec!["java.io.IOException", "x7.w$a"],
        ),
        (
            "Lcom/bumptech/glide/load/data/e;",
            "a",
            vec!["java.io.IOException", "x7.w$a"],
        ),
        (
            "Lcom/google/android/gms/common/internal/c;",
            "c",
            vec!["android.os.RemoteException"],
        ),
        (
            "Lcom/google/android/gms/common/internal/b;",
            "c",
            vec!["android.os.RemoteException"],
        ),
    ] {
        let (key, (pool, method)) = facts
            .definitions
            .iter()
            .find(|(key, _)| key.owner == owner && key.name == name)
            .unwrap();
        let contract = &plan.methods[key];
        assert!(contract.original.is_empty());
        assert_eq!(contract.source, expected);
        assert!(!plan.unavailable.contains_key(key));
        println!(
            "ATTESTED {pool}:{} {key:?} flags={:#x} code={} original={:?} source={:?}",
            method.method_idx,
            method.access_flags,
            method.code.is_some(),
            contract.original,
            contract.source
        );
        if facts.abstracts.contains(key) {
            assert!(method.code.is_none());
            assert_eq!(method.access_flags & (0x400 | 0x100 | 8 | 2), 0x400);
            assert!(
                catalog[key]
                    .as_ref()
                    .unwrap()
                    .exceptions
                    .as_ref()
                    .unwrap()
                    .is_empty()
            );
        }
    }
    for (owner, name) in [("Lx7/w;", "reset"), ("Lhb/a;", "h0")] {
        let (key, (pool, method)) = facts
            .definitions
            .iter()
            .find(|(key, _)| key.owner == owner && key.name == name)
            .unwrap();
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[*pool]);
        let mut resolver =
            Resolver::with_catalog(&files[*pool], &aliases, &metadata, Some(&catalog));
        resolver.inputs = &files;
        resolver.dex_index = *pool;
        resolver.external = Some(&external);
        resolver.hierarchy = hierarchy.clone();
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
        for (id, instruction) in ssa.ir().instructions() {
            match instruction {
                ManagedInsn::Invoke { method, .. } => {
                    let target = method_key(&files[*pool], method.index()).unwrap();
                    if target.owner == "Landroid/os/IBinder;" {
                        println!(
                            "ORIGINAL CAUSE {key:?} invoke@{:?} {target:?} metadata={:?}",
                            ssa.offset(id),
                            resolver.declaration(method.index())
                        );
                    }
                }
                ManagedInsn::Throw { value } => println!(
                    "ORIGINAL CAUSE {key:?} throw@{:?} type={:?}",
                    ssa.offset(id),
                    ssa.ir().value(*value).unwrap().ty()
                ),
                _ => {}
            }
        }
    }
    for (key, contract) in &plan.methods {
        if !contract.source.is_empty() && contract.source != contract.original {
            println!(
                "WAVE4 DERIVED {key:?} original={:?} source={:?}",
                contract.original, contract.source
            );
        }
    }
    for (key, error) in &plan.unavailable {
        if !matches!(error, required::ThrowsError::MissingSourceTarget(_)) {
            println!("WAVE4 CAUSE {key:?} {error:?}");
        }
    }
}
