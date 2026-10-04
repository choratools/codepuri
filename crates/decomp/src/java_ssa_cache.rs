//! Export-session reuse of immutable SSA frontend results.
//!
//! Code addresses are identities only: callers must keep the defining DEX
//! inputs and their type metadata unchanged and alive for the entire session.
//! A cache must never outlive or be reused for another export's inputs.

use std::collections::{HashMap, hash_map::Entry};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

use dex_decompiler::{StageError, ssa::SsaMethod};
use dex_format::code::CodeItem;
use managed_ir::types::ManagedType;

/// Exact input identity under one export's immutable per-DEX type metadata.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct CacheKey {
    dex_index: usize,
    method_index: u32,
    code_address: usize,
    initial_registers: Vec<((u16, bool), ManagedType)>,
}

impl CacheKey {
    /// `initial_registers` must use the same deterministic order on every call.
    pub(super) fn new(
        dex_index: usize,
        method_index: u32,
        code: &CodeItem,
        initial_registers: Vec<((u16, bool), ManagedType)>,
    ) -> Self {
        Self {
            dex_index,
            method_index,
            // Store no borrow or dereferenceable raw pointer. Session ownership
            // guarantees that this address remains an immutable identity.
            code_address: std::ptr::from_ref(code) as usize,
            initial_registers,
        }
    }
}

type CachedResult = Result<Arc<SsaMethod>, StageError>;
type CacheCell = Arc<OnceLock<CachedResult>>;

/// Counters for stored computations; disabled caches keep every counter zero.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CacheStats {
    pub(super) hits: usize,
    pub(super) misses: usize,
    pub(super) entries: usize,
    pub(super) failures: usize,
}

/// One cache owned by one Java export, shared by its method workers.
pub(super) struct SsaCache {
    enabled: bool,
    entries: Mutex<HashMap<CacheKey, CacheCell>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
    failures: AtomicUsize,
}

impl SsaCache {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            entries: Mutex::new(HashMap::new()),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
        }
    }

    /// Reuses successes and failures, with at most one completed computation
    /// per key. Expensive frontend work runs outside the map mutex.
    pub(super) fn get_or_compute(
        &self,
        key: CacheKey,
        compute: impl FnOnce() -> Result<SsaMethod, StageError>,
    ) -> CachedResult {
        if !self.enabled {
            return compute().map(Arc::new);
        }
        let cell = {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            match entries.entry(key) {
                Entry::Occupied(entry) => {
                    self.hits.fetch_add(1, Ordering::Relaxed);
                    Arc::clone(entry.get())
                }
                Entry::Vacant(entry) => {
                    self.misses.fetch_add(1, Ordering::Relaxed);
                    Arc::clone(entry.insert(Arc::new(OnceLock::new())))
                }
            }
        };
        cell.get_or_init(|| {
            let result = compute().map(Arc::new);
            if result.is_err() {
                self.failures.fetch_add(1, Ordering::Relaxed);
            }
            result
        })
        .clone()
    }

    /// Exact after workers finish; concurrent observations are a snapshot.
    pub(super) fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries: self.entries.lock().unwrap_or_else(|e| e.into_inner()).len(),
            failures: self.failures.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use dex_decompiler::{DexPasses, TypeContext};
    use std::sync::Barrier;

    fn code(units: &[u16]) -> CodeItem {
        CodeItem {
            registers_size: 2,
            ins_size: 2,
            outs_size: 0,
            debug_info_off: 0,
            insns_size: units.len() as u32,
            insns: units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
            tries: vec![],
            handlers: vec![],
            span: dex_format::RawSpan {
                offset: 0,
                size: (units.len() * 2) as u32,
            },
        }
    }

    fn key(code: &CodeItem) -> CacheKey {
        CacheKey::new(0, 0, code, vec![])
    }

    #[test]
    fn concurrent_same_key_runs_frontend_once_and_shares_result() {
        let code = code(&[0x000e]); // return-void
        let cache = SsaCache::new(true);
        let calls = AtomicUsize::new(0);
        let barrier = Barrier::new(8);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        cache
                            .get_or_compute(key(&code), || {
                                calls.fetch_add(1, Ordering::Relaxed);
                                DexPasses::run_merged(&code)
                            })
                            .unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(results.iter().all(|ssa| Arc::ptr_eq(ssa, &results[0])));
        assert_eq!(
            cache.stats(),
            CacheStats {
                hits: 7,
                misses: 1,
                entries: 1,
                failures: 0
            }
        );
    }

    #[test]
    fn every_input_identity_component_distinguishes_entries() {
        let code = code(&[0x000e]);
        let other_code = code.clone();
        let seeds = vec![
            ((0, false), ManagedType::Int),
            ((1, false), ManagedType::Float),
        ];
        let cache = SsaCache::new(true);
        let keys = [
            CacheKey::new(0, 0, &code, seeds.clone()),
            CacheKey::new(1, 0, &code, seeds.clone()),
            CacheKey::new(0, 1, &code, seeds.clone()),
            CacheKey::new(0, 0, &other_code, seeds.clone()),
            CacheKey::new(0, 0, &code, vec![]),
            CacheKey::new(
                0,
                0,
                &code,
                vec![
                    ((0, false), ManagedType::Int),
                    ((1, false), ManagedType::Int),
                ],
            ),
            CacheKey::new(
                0,
                0,
                &code,
                vec![
                    ((0, true), ManagedType::Int),
                    ((1, false), ManagedType::Float),
                ],
            ),
            CacheKey::new(
                0,
                0,
                &code,
                vec![
                    ((1, false), ManagedType::Int),
                    ((1, false), ManagedType::Float),
                ],
            ),
            CacheKey::new(0, 0, &code, seeds.into_iter().rev().collect()),
        ];
        let results: Vec<_> = keys
            .iter()
            .map(|key| {
                cache
                    .get_or_compute(key.clone(), || DexPasses::run_merged(&code))
                    .unwrap()
            })
            .collect();
        for (key, original) in keys.iter().zip(&results) {
            let reused = cache
                .get_or_compute(key.clone(), || panic!("key should hit"))
                .unwrap();
            assert!(Arc::ptr_eq(original, &reused));
        }
        for (index, result) in results.iter().enumerate() {
            assert!(
                results[..index]
                    .iter()
                    .all(|other| !Arc::ptr_eq(other, result))
            );
        }
        assert_eq!(cache.stats().entries, keys.len());
        assert_eq!(cache.stats().misses, keys.len());
        assert_eq!(cache.stats().hits, keys.len());
    }

    #[test]
    fn changed_seed_types_produce_separate_typed_frontend_results() {
        struct Seed(ManagedType);
        impl TypeContext for Seed {
            fn initial_register_type(&self, register: u16, wide: bool) -> Option<ManagedType> {
                (register == 0 && !wide).then(|| self.0.clone())
            }
        }
        let code = code(&[0x000f]); // return v0
        let cache = SsaCache::new(true);
        for ty in [ManagedType::Int, ManagedType::Float] {
            let context = Seed(ty.clone());
            let ssa = cache
                .get_or_compute(
                    CacheKey::new(0, 0, &code, vec![((0, false), ty.clone())]),
                    || DexPasses::run_ssa_with(&code, &context),
                )
                .unwrap();
            let slot = ssa
                .ir()
                .register_slots()
                .values()
                .find(|s| s.index() == 0)
                .unwrap();
            assert!(
                slot.values()
                    .iter()
                    .any(|value| ssa.ir().value(*value).unwrap().ty() == &ty)
            );
        }
        assert_eq!(cache.stats().misses, 2);
    }

    #[test]
    fn frontend_errors_are_cached_and_counted_once() {
        // filled-new-array with no resolvable array descriptor fails lifting.
        let code = code(&[0x0025, 0, 0, 0x000e]);
        let cache = SsaCache::new(true);
        let expected = DexPasses::run_merged(&code).unwrap_err();
        let first = cache.get_or_compute(key(&code), || DexPasses::run_merged(&code));
        let second = cache.get_or_compute(key(&code), || panic!("failed entry should hit"));
        assert_eq!(first.unwrap_err(), expected);
        assert_eq!(second.unwrap_err(), expected);
        assert_eq!(
            cache.stats(),
            CacheStats {
                hits: 1,
                misses: 1,
                entries: 1,
                failures: 1
            }
        );
    }

    #[test]
    fn disabled_cache_computes_every_time_and_retains_nothing() {
        let code = code(&[0x000e]);
        let cache = SsaCache::new(false);
        let calls = AtomicUsize::new(0);
        let run = || {
            cache
                .get_or_compute(key(&code), || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    DexPasses::run_merged(&code)
                })
                .unwrap()
        };
        let first = run();
        let second = run();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        let failed = code.clone();
        assert!(
            cache
                .get_or_compute(key(&failed), || Err(StageError::Ssa(
                    dex_decompiler::SsaError::Structure("fixture failure".into()),
                )))
                .is_err()
        );
        assert_eq!(cache.stats(), CacheStats::default());
    }

    #[test]
    fn export_sessions_do_not_share_entries() {
        let code = code(&[0x000e]);
        let first_cache = SsaCache::new(true);
        let second_cache = SsaCache::new(true);
        let first = first_cache
            .get_or_compute(key(&code), || DexPasses::run_merged(&code))
            .unwrap();
        let second = second_cache
            .get_or_compute(key(&code), || DexPasses::run_merged(&code))
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        for cache in [&first_cache, &second_cache] {
            assert_eq!(cache.stats().misses, 1);
            assert_eq!(cache.stats().hits, 0);
        }
    }

    #[test]
    fn frontend_computation_does_not_hold_global_map_lock() {
        let code = code(&[0x000e]);
        let cache = SsaCache::new(true);
        cache
            .get_or_compute(key(&code), || {
                assert!(cache.entries.try_lock().is_ok());
                let nested = cache
                    .get_or_compute(CacheKey::new(0, 1, &code, vec![]), || {
                        DexPasses::run_merged(&code)
                    })
                    .unwrap();
                assert!(nested.ir().cfg().validate().is_ok());
                DexPasses::run_merged(&code)
            })
            .unwrap();
        assert_eq!(cache.stats().entries, 2);
    }
}
