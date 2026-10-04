#![allow(clippy::unwrap_used)]
use super::*;

fn declaration(name: &str, parent: Option<&str>) -> Declaration {
    Declaration {
        name: name.into(),
        superclass: parent.map(str::to_owned),
        interfaces: Vec::new(),
        is_interface: false,
    }
}

fn query(h: &Hierarchy, from: &str, to: &str) -> Compatibility {
    h.assignable(&JavaType::Class(from.into()), &JavaType::Class(to.into()))
}

fn entries(h: &Hierarchy) -> usize {
    h.assignments
        .read()
        .unwrap()
        .values()
        .map(BTreeMap::len)
        .sum()
}

#[test]
fn missing_results_refresh_only_after_a_new_attested_declaration() {
    let mut h = Hierarchy::default();
    h.insert(declaration("Child", Some("Base"))).unwrap();
    let missing = Compatibility::Indeterminate(UnsupportedType::MissingHierarchy("Base".into()));
    assert_eq!(query(&h, "Child", "Target"), missing);
    assert_eq!(query(&h, "Child", "Target"), missing);
    assert_eq!(entries(&h), 1);
    h.insert(declaration("Child", Some("Base"))).unwrap();
    assert_eq!(entries(&h), 1);
    assert!(h.insert(declaration("Child", Some("Wrong"))).is_err());
    assert_eq!(entries(&h), 1);
    h.insert(declaration("Base", Some("Target"))).unwrap();
    assert_eq!(entries(&h), 0);
    assert_eq!(query(&h, "Child", "Target"), Compatibility::Assignable);
    assert_eq!(
        query(&h, "Target", "Child"),
        Compatibility::Indeterminate(UnsupportedType::MissingHierarchy("Target".into()))
    );
    assert_eq!(entries(&h), 2);
}

#[test]
fn clones_and_renamed_hierarchies_have_independent_cached_evidence() {
    let mut original = Hierarchy::default();
    original.insert(declaration("Child", Some("Base"))).unwrap();
    let missing = query(&original, "Child", "Target");
    let mut cloned = original.clone();
    assert_eq!(entries(&cloned), 0);
    cloned.insert(declaration("Base", Some("Target"))).unwrap();
    assert_eq!(query(&cloned, "Child", "Target"), Compatibility::Assignable);
    assert_eq!(query(&original, "Child", "Target"), missing);
    let mapped = cloned.map_names(|n| format!("alias.{n}")).unwrap();
    assert_eq!(entries(&mapped), 0);
    assert_eq!(
        query(&mapped, "alias.Child", "alias.Target"),
        Compatibility::Assignable
    );
    assert!(matches!(
        query(&mapped, "Child", "Target"),
        Compatibility::Indeterminate(_)
    ));
}

#[test]
fn cached_roots_match_uncached_walks_with_cycles_and_missing_edges() {
    let mut h = Hierarchy::default();
    for (name, parent) in [
        ("A", Some("B")),
        ("B", Some("A")),
        ("C", Some("Missing")),
        ("D", Some("java.lang.Object")),
    ] {
        h.insert(declaration(name, parent)).unwrap();
    }
    h.insert(Declaration {
        name: "Diamond".into(),
        superclass: Some("C".into()),
        interfaces: vec!["D".into()],
        is_interface: false,
    })
    .unwrap();
    for from in ["A", "B", "C", "D", "Diamond", "Missing"] {
        for to in ["A", "B", "C", "D", "Diamond", "Missing", "java.lang.Object"] {
            let expected = h.ancestors(from, to, &mut BTreeSet::new());
            assert_eq!(query(&h, from, to), expected);
            assert_eq!(query(&h, from, to), expected);
        }
    }
    let array = |name: &str| JavaType::Array(Box::new(JavaType::Class(name.into())));
    assert_eq!(
        h.assignable(&array("Diamond"), &array("D")),
        Compatibility::Assignable
    );
    assert_eq!(
        h.assignable(&JavaType::Int, &JavaType::Long),
        Compatibility::Incompatible
    );
}

#[test]
fn shared_workers_reuse_the_same_completed_type_pair() {
    let mut h = Hierarchy::default();
    h.insert(declaration("Child", Some("Parent"))).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..100 {
                    assert_eq!(query(&h, "Child", "Parent"), Compatibility::Assignable);
                }
            });
        }
    });
    assert_eq!(entries(&h), 1);
}

#[test]
#[ignore = "controlled repeated-query microbenchmark; not whole-APK throughput"]
fn repeated_assignment_benchmark() {
    let mut h = Hierarchy::default();
    for i in 0..64 {
        h.insert(declaration(
            &format!("Class{i}"),
            Some(&format!("Class{}", i + 1)),
        ))
        .unwrap();
    }
    let from = JavaType::Class("Class0".into());
    let to = JavaType::Class("Class64".into());
    let start = std::time::Instant::now();
    for _ in 0..20_000 {
        assert_eq!(
            std::hint::black_box(h.ancestors("Class0", "Class64", &mut BTreeSet::new())),
            Compatibility::Assignable
        );
    }
    let uncached = start.elapsed();
    let start = std::time::Instant::now();
    for _ in 0..20_000 {
        assert_eq!(
            std::hint::black_box(h.assignable(&from, &to)),
            Compatibility::Assignable
        );
    }
    let cached = start.elapsed();
    assert_eq!(entries(&h), 1);
    println!(
        "hierarchy_cache queries=20000 depth=64 uncached_s={:.6} cached_s={:.6} speedup={:.2}",
        uncached.as_secs_f64(),
        cached.as_secs_f64(),
        uncached.as_secs_f64() / cached.as_secs_f64()
    );
}
