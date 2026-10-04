//! Shared pool equivalence, isolation, and bounded construction measurements.
#![allow(clippy::unwrap_used)]

use dex_decompiler::infer::{DexMetadata, DexTypes, TypeContext};
use dex_format::DexFile;
use managed_ir::types::ManagedType;

fn fixture() -> DexFile {
    DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap()
}

fn exercise(ctx: &dyn TypeContext, dex: &DexFile) -> Vec<(&'static str, u64)> {
    for i in 0..dex.type_ids.len() as u32 + 2 {
        assert_eq!(ctx.type_at(i), ctx.type_at(i));
    }
    for i in 0..dex.field_ids.len() as u32 + 2 {
        ctx.field_type(i);
    }
    for i in 0..dex.method_ids.len() as u32 + 2 {
        ctx.method_return(i);
        let mut params = vec![ManagedType::Unknown];
        ctx.method_params(i, &mut params);
    }
    ctx.note_unresolved("method", u32::MAX);
    ctx.take_unresolved()
}

#[test]
fn cached_matches_fresh_types_and_diagnostics() {
    let mut dex = fixture();
    // Include malformed descriptors, invalid pool references and prototypes.
    dex.type_ids[0].descriptor_idx = u32::MAX;
    dex.proto_ids[0].return_type_idx = u32::MAX;
    dex.method_ids[0].proto_idx = u16::MAX;
    let metadata = DexMetadata::new(&dex);
    for method in [None, Some(0), Some(1), Some(u32::MAX)] {
        let fresh = DexTypes::new(&dex, method);
        let cached = DexTypes::with_metadata(&metadata, method);
        assert_eq!(fresh.string_type(), cached.string_type());
        assert_eq!(fresh.enclosing_return(), cached.enclosing_return());
        for i in 0..dex.type_ids.len() as u32 + 2 {
            assert_eq!(fresh.type_at(i), cached.type_at(i));
        }
        for i in 0..dex.field_ids.len() as u32 + 2 {
            assert_eq!(fresh.field_type(i), cached.field_type(i));
        }
        for i in 0..dex.method_ids.len() as u32 + 2 {
            assert_eq!(fresh.method_return(i), cached.method_return(i));
            let mut a = vec![ManagedType::Int];
            let mut b = a.clone();
            assert_eq!(
                fresh.method_params(i, &mut a),
                cached.method_params(i, &mut b)
            );
            assert_eq!(a, b);
        }
        assert_eq!(exercise(&fresh, &dex), exercise(&cached, &dex));
        assert!(fresh.take_unresolved().is_empty());
        assert!(cached.take_unresolved().is_empty());
        // Failed-type deduplication survives draining, just as before.
        assert_eq!(cached.type_at(u32::MAX), None);
        assert!(cached.take_unresolved().is_empty());
    }
}

#[test]
fn parallel_contexts_isolate_diagnostics_and_returns() {
    let dex = fixture();
    let metadata = DexMetadata::new(&dex);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let metadata = &metadata;
                let dex = &dex;
                scope.spawn(move || {
                    let ctx = DexTypes::with_metadata(metadata, Some(i));
                    assert_eq!(
                        ctx.enclosing_return(),
                        DexTypes::new(dex, Some(i)).enclosing_return()
                    );
                    for _ in 0..i + 1 {
                        ctx.note_unresolved("worker", i);
                    }
                    assert_eq!(ctx.type_at(u32::MAX), None);
                    assert_eq!(ctx.type_at(u32::MAX), None);
                    assert_eq!(
                        ctx.take_unresolved(),
                        vec![("type", 1), ("worker", u64::from(i + 1))]
                    );
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    });
    assert!(
        DexTypes::with_metadata(&metadata, None)
            .take_unresolved()
            .is_empty()
    );
}

#[test]
fn snapshot_is_not_reused_for_another_pool_or_source_mutation() {
    let mut dex = fixture();
    let snapshot = DexMetadata::new(&dex);
    let before = DexTypes::with_metadata(&snapshot, None).type_at(0);
    dex.type_ids[0].descriptor_idx = u32::MAX;
    let changed = DexMetadata::new(&dex);
    assert_eq!(DexTypes::with_metadata(&changed, None).type_at(0), None);
    assert_eq!(DexTypes::with_metadata(&snapshot, None).type_at(0), before);
    assert!(before.is_some());
}

#[test]
#[ignore = "bounded metadata construction microbenchmark; run release --ignored --nocapture"]
fn metadata_construction_microbenchmark() {
    let dex = fixture();
    let iterations = 2000;
    let start = std::time::Instant::now();
    for i in 0..iterations {
        std::hint::black_box(DexTypes::new(&dex, Some(i % dex.method_ids.len() as u32)));
    }
    let uncached = start.elapsed();
    let start = std::time::Instant::now();
    let metadata = DexMetadata::new(&dex);
    for i in 0..iterations {
        std::hint::black_box(DexTypes::with_metadata(
            &metadata,
            Some(i % dex.method_ids.len() as u32),
        ));
    }
    let cached = start.elapsed();
    println!(
        "types={} protos={} methods={} fields={} contexts={} metadata_builds_uncached={} metadata_builds_cached=1 uncached_us={} cached_including_build_us={}",
        dex.type_ids.len(),
        dex.proto_ids.len(),
        dex.method_ids.len(),
        dex.field_ids.len(),
        iterations,
        iterations,
        uncached.as_micros(),
        cached.as_micros()
    );
}
