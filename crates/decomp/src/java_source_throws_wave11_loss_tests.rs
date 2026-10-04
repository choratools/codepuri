use super::*;

#[test]
#[ignore = "bounded genuine wave11 loss source-contract diagnosis; no export"]
fn wave11_original_loss_source_contract_diagnosis() {
    let (files, aliases, catalog, external, hierarchy) = original_fixture();
    let input_refs = files.iter().collect::<Vec<_>>();
    let super_index =
        crate::java_super_resolution::SuperIndex::collect(&input_refs, &external).unwrap();
    let targets = [
        (0, 5505),
        (0, 5507),
        (0, 5509),
        (0, 5572),
        (0, 5517),
        (0, 9017),
        (0, 8177),
        (2, 45058),
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
    let metadata = files
        .iter()
        .map(dex_decompiler::infer::DexMetadata::new)
        .collect::<Vec<_>>();
    let resolvers = files
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
            resolver.super_index = Some(&super_index);
            resolver
        })
        .collect::<Vec<_>>();
    for (pool, index) in targets {
        let key = method_key(&files[pool], index).unwrap();
        let mut pending = vec![key];
        let mut seen = BTreeSet::new();
        while let Some(key) = pending.pop() {
            if !seen.insert(key.clone()) {
                continue;
            }
            let Some((pool, method)) = facts.definitions.get(&key) else {
                continue;
            };
            let context = Context {
                resolver: &resolvers[*pool],
                facts: &facts,
            };
            let identity = OriginalMethodIdentity {
                dex: *pool,
                index: method.method_idx,
                key: key.clone(),
            };
            println!(
                "WAVE11 NODE root={index} pool={pool} index={} {key:?} flags={:#x} original={:?} source={:?} failure={:?} overrides={:?}",
                method.method_idx,
                method.access_flags,
                context.invocation_contract(MethodRef::new(method.method_idx)),
                plan.methods.get(&key),
                plan.unavailable.get(&key),
                context.override_contract(&identity)
            );
            if let Some(required::ThrowsError::MissingSourceTarget(next)) =
                plan.unavailable.get(&key)
            {
                pending.push(next.clone());
            }
            if facts.concrete_sources.contains(&key) || facts.abstracts.contains(&key) {
                for candidate in facts.implementations.get(&key) {
                    if candidate == &key {
                        continue;
                    }
                    let (candidate_pool, candidate_method) = facts.definitions[candidate];
                    let candidate_context = Context {
                        resolver: &resolvers[candidate_pool],
                        facts: &facts,
                    };
                    if candidate_context.nonimplementation(candidate, &key) {
                        continue;
                    }
                    let candidate_identity = OriginalMethodIdentity {
                        dex: candidate_pool,
                        index: candidate_method.method_idx,
                        key: candidate.clone(),
                    };
                    let declaration = candidate_context.override_contract(&candidate_identity);
                    println!(
                        "WAVE11 CANDIDATE root={index} parent={} pool={candidate_pool} index={} {candidate:?} original={:?} source={:?} failure={:?} overrides={declaration:?}",
                        method.method_idx,
                        candidate_method.method_idx,
                        candidate_context
                            .invocation_contract(MethodRef::new(candidate_method.method_idx)),
                        plan.methods.get(candidate),
                        plan.unavailable.get(candidate)
                    );
                    if matches!(declaration, OverrideContract::Unknown) {
                        pending.push(candidate.clone());
                    }
                }
            }
            if let Some(code) = &method.code {
                let (ctx, seeds) = initial_context_with_seeds(&resolvers[*pool], method).unwrap();
                let ssa =
                    cached_frontend(&resolvers[*pool], method, code, &ctx, seeds, None).unwrap();
                for (id, insn) in ssa.ir().instructions() {
                    if let ManagedInsn::Invoke {
                        method: reference,
                        kind,
                        ..
                    } = insn
                    {
                        let callee = method_key(&files[*pool], reference.index()).unwrap();
                        println!(
                            "WAVE11 EDGE root={index} caller={} offset={:?} ref={} kind={kind:?} {callee:?} original={:?} source={:?} failure={:?}",
                            method.method_idx,
                            ssa.offset(id),
                            reference.index(),
                            context.invoked_contract(&identity, *kind, *reference),
                            plan.methods.get(&callee).map(|v| &v.source),
                            plan.unavailable.get(&callee)
                        );
                    }
                }
            }
        }
    }
}
