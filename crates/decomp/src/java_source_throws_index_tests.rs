#![allow(clippy::unwrap_used)]
use super::*;
use std::{cell::Cell, io::Write, time::Instant};

type Candidates = BTreeMap<(String, Vec<String>), Vec<MethodKey>>;
fn key(owner: &str, name: &str, parameters: &[&str], result: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: name.into(),
        parameters: parameters.iter().map(|p| (*p).into()).collect(),
        return_type: result.into(),
    }
}
// Exact pre-index target-directed oracle: equality precedes both the visited
// check and the original-input owner lookup, including duplicate/missing owners.
fn old_ancestor(owner: &str, target: &str, parents: &impl Fn(&str) -> Vec<String>) -> bool {
    let mut queue = vec![owner.to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(owner) = queue.pop() {
        if owner == target {
            return true;
        }
        if !seen.insert(owner.clone()) {
            continue;
        }
        queue.extend(parents(&owner));
    }
    false
}
fn old_scan(
    abstracts: &BTreeSet<MethodKey>,
    candidates: &Candidates,
    parents: impl Fn(&str) -> Vec<String>,
) -> BTreeMap<MethodKey, Vec<MethodKey>> {
    let mut result = BTreeMap::new();
    for target in abstracts {
        let mut links = Vec::new();
        for candidate in candidates
            .get(&(target.name.clone(), target.parameters.clone()))
            .into_iter()
            .flatten()
        {
            // The prior production scan cloned its caller identity before the
            // ancestry predicate. Include this cost in the isolated old scan.
            let identity = candidate.clone();
            if old_ancestor(&candidate.owner, &target.owner, &parents) {
                links.push(identity);
            }
        }
        result.insert(target.clone(), links);
    }
    result
}
fn assert_links(
    abstracts: &BTreeSet<MethodKey>,
    candidates: &Candidates,
    parents: impl Fn(&str) -> Vec<String>,
) -> ImplementationIndex {
    let expected = old_scan(abstracts, candidates, &parents);
    let index = ImplementationIndex::build(abstracts, candidates, parents);
    for target in abstracts {
        assert_eq!(index.get(target), expected[target]);
    }
    index
}

#[test]
fn index_preserves_full_returns_parameter_order_and_duplicate_candidate_order() {
    let a = key(
        "LContract;",
        "a",
        &["I", "Ljava/lang/String;"],
        "Ljava/lang/Object;",
    );
    let b = key(
        "LContract;",
        "a",
        &["I", "Ljava/lang/String;"],
        "Ljava/lang/String;",
    );
    let c = key("LContract;", "a", &["Ljava/lang/String;", "I"], "V");
    let first = key(
        "LImpl1;",
        "a",
        &["I", "Ljava/lang/String;"],
        "Ljava/lang/String;",
    );
    let second = key(
        "LImpl2;",
        "a",
        &["I", "Ljava/lang/String;"],
        "Ljava/lang/Object;",
    );
    let candidates = BTreeMap::from([(
        (a.name.clone(), a.parameters.clone()),
        vec![second.clone(), first.clone(), second.clone()],
    )]);
    let abstracts = BTreeSet::from([a.clone(), b.clone(), c.clone()]);
    let index = assert_links(&abstracts, &candidates, |owner| {
        if owner.starts_with("LImpl") {
            vec!["LContract;".into()]
        } else {
            vec![]
        }
    });
    assert_eq!(index.get(&a), [second.clone(), first, second]);
    assert_eq!(index.get(&a), index.get(&b));
    assert!(index.get(&c).is_empty());
    assert_eq!(index.construction.cached_owners, 2);
}
#[test]
fn index_caches_completed_owner_closures_for_cycles_and_unknown_duplicate_boundaries() {
    let graph = BTreeMap::from([
        ("LA;", Some(vec!["LB;", "LMissing;", "LDuplicate;"])),
        ("LB;", Some(vec!["LA;"])),
        ("LDuplicate;", None), // Genuine duplicate-owner lookup stops here.
        ("LExternal;", None),
    ]);
    let abstracts = [
        "LA;",
        "LB;",
        "LMissing;",
        "LDuplicate;",
        "LExternal;",
        "LUnreachable;",
    ]
    .into_iter()
    .map(|o| key(o, "a", &[], "V"))
    .collect();
    let a = key("LA;", "a", &[], "V");
    let b = key("LB;", "a", &[], "Ljava/lang/Object;");
    let candidates = BTreeMap::from([(("a".into(), vec![]), vec![a, b])]);
    let calls = Cell::new(0usize);
    let parents = |owner: &str| {
        calls.set(calls.get() + 1);
        graph
            .get(owner)
            .and_then(Option::as_ref)
            .into_iter()
            .flatten()
            .map(|p| (*p).into())
            .collect()
    };
    let index = assert_links(&abstracts, &candidates, parents);
    assert_eq!(index.construction.cached_owners, 2);
    assert_eq!(index.get(&key("LA;", "a", &[], "V")).len(), 2);
    assert_eq!(index.get(&key("LDuplicate;", "a", &[], "V")).len(), 2);
    assert!(index.get(&key("LUnreachable;", "a", &[], "V")).is_empty());
    assert!(calls.get() > 0);
}
#[test]
fn index_sessions_are_independent_and_one_owner_is_computed_once_across_signatures() {
    let a = key("LContract;", "a", &[], "V");
    let b = key("LContract;", "b", &[], "V");
    let candidates = BTreeMap::from([
        (("a".into(), vec![]), vec![key("LImpl;", "a", &[], "V")]),
        (("b".into(), vec![]), vec![key("LImpl;", "b", &[], "V")]),
    ]);
    let abstracts = BTreeSet::from([a.clone(), b.clone()]);
    let owner_reads = Cell::new(0);
    let first = ImplementationIndex::build(&abstracts, &candidates, |owner| {
        if owner == "LImpl;" {
            owner_reads.set(owner_reads.get() + 1);
            vec!["LContract;".into()]
        } else {
            vec![]
        }
    });
    let second = ImplementationIndex::build(&abstracts, &candidates, |_| vec![]);
    assert_eq!(owner_reads.get(), 1);
    assert_eq!(first.get(&a).len(), 1);
    assert_eq!(first.get(&b).len(), 1);
    assert!(second.get(&a).is_empty());
    assert!(second.get(&b).is_empty());
}
fn key_heap_bytes(key: &MethodKey) -> usize {
    key.owner.capacity()
        + key.name.capacity()
        + key.return_type.capacity()
        + key.parameters.capacity() * std::mem::size_of::<String>()
        + key.parameters.iter().map(String::capacity).sum::<usize>()
}
fn payload_bytes(index: &ImplementationIndex) -> usize {
    index
        .links
        .iter()
        .map(|(key, methods)| {
            std::mem::size_of::<MethodKey>()
                + key_heap_bytes(key)
                + std::mem::size_of::<Vec<MethodKey>>()
                + methods.capacity() * std::mem::size_of::<MethodKey>()
                + methods.iter().map(key_heap_bytes).sum::<usize>()
        })
        .sum()
}
fn write_vectors(path: &std::path::Path, vectors: &BTreeMap<MethodKey, Vec<MethodKey>>) {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let text = |file: &mut std::io::BufWriter<std::fs::File>, text: &str| {
        file.write_all(&(text.len() as u64).to_le_bytes()).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    };
    let method = |file: &mut std::io::BufWriter<std::fs::File>, key: &MethodKey| {
        text(file, &key.owner);
        text(file, &key.name);
        file.write_all(&(key.parameters.len() as u64).to_le_bytes())
            .unwrap();
        for parameter in &key.parameters {
            text(file, parameter);
        }
        text(file, &key.return_type);
    };
    file.write_all(&(vectors.len() as u64).to_le_bytes())
        .unwrap();
    for (target, links) in vectors {
        method(&mut file, target);
        file.write_all(&(links.len() as u64).to_le_bytes()).unwrap();
        for link in links {
            method(&mut file, link);
        }
    }
    file.flush().unwrap();
}
#[test]
#[ignore = "genuine original APK linkage parity and isolated old/index timings; no export"]
fn samsung_original_all_link_vectors_and_isolated_index_costs() {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let mut owners = BTreeMap::new();
    let mut abstracts = BTreeSet::new();
    let mut candidates = Candidates::new();
    for (pool, dex) in files.iter().enumerate() {
        for (index, class) in dex.classes.iter().enumerate() {
            if let Some(owner) = descriptor(dex, class.class_idx) {
                owners
                    .entry(owner.into())
                    .and_modify(|v| *v = None)
                    .or_insert(Some((pool, index)));
            }
            for method in class
                .class_data
                .as_ref()
                .into_iter()
                .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            {
                let key = method_key(dex, method.method_idx).unwrap();
                if method.code.is_none()
                    && method.access_flags & 0x400 != 0
                    && method.access_flags & (0x100 | 8 | 2) == 0
                {
                    abstracts.insert(key);
                } else if method.code.is_some()
                    && method.access_flags & (0x400 | 0x100 | 8 | 2) == 0
                    && !key.name.starts_with('<')
                {
                    candidates
                        .entry((key.name.clone(), key.parameters.clone()))
                        .or_default()
                        .push(key);
                }
            }
        }
    }
    let parents = |owner: &str| input_parents(&files, &owners, owner);
    let old_start = Instant::now();
    let old = old_scan(&abstracts, &candidates, parents);
    let old_seconds = old_start.elapsed().as_secs_f64();
    let build_start = Instant::now();
    let index = ImplementationIndex::build(&abstracts, &candidates, parents);
    let build_seconds = build_start.elapsed().as_secs_f64();
    let lookup_start = Instant::now();
    let found = abstracts
        .iter()
        .map(|key| {
            std::hint::black_box(&index)
                .get(std::hint::black_box(key))
                .len()
        })
        .sum::<usize>();
    let lookup_seconds = lookup_start.elapsed().as_secs_f64();
    let new = abstracts
        .iter()
        .map(|key| (key.clone(), index.get(key).to_vec()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(new, old); // Entire ordered vectors, not just counts or hashes.
    assert_eq!(abstracts.len(), 11_201);
    assert_eq!(candidates.values().map(Vec::len).sum::<usize>(), 105_902);
    assert_eq!(found, 36_518);
    let pairs = abstracts
        .iter()
        .map(|key| {
            candidates
                .get(&(key.name.clone(), key.parameters.clone()))
                .map_or(0, Vec::len)
        })
        .sum::<usize>();
    assert_eq!(pairs, 1_452_305);
    println!(
        "INDEX_PARITY abstracts={} candidates={} old_pairs={pairs} links={found} cached_owners={} cached_ancestor_names={} retained_payload_lower_bound_bytes={} temporary_ancestor_payload_lower_bound_bytes={} old_scan_seconds={old_seconds:.9} index_build_seconds={build_seconds:.9} all_lookup_seconds={lookup_seconds:.9} allocator_btree_overhead_excluded=true",
        abstracts.len(),
        candidates.values().map(Vec::len).sum::<usize>(),
        index.construction.cached_owners,
        index.construction.cached_ancestor_names,
        payload_bytes(&index),
        index.construction.ancestor_payload_bytes
    );
    if let Ok(path) = std::env::var("DECOMPHIFY_THROWS_INDEX_EVIDENCE") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        write_vectors(&path.join("old-link-vectors.bin"), &old);
        write_vectors(&path.join("indexed-link-vectors.bin"), &new);
    }
}
