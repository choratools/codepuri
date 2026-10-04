//! Dense typed lookup results owned by one export session and one DEX pool.
//!
//! Every cache instance belongs to exactly one immutable pool. Reusing an
//! instance for another pool or after mutation would invalidate its results.

use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

/// Completed lookup counters; entries equal misses after workers finish.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LookupStats {
    pub(super) hits: usize,
    pub(super) misses: usize,
    pub(super) failures: usize,
}

/// One immutable DEX pool's typed results, including unsuccessful lookups.
pub(super) struct LookupCache<T: Clone> {
    enabled: bool,
    entries: Vec<OnceLock<Option<T>>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
    failures: AtomicUsize,
}

impl<T: Clone> LookupCache<T> {
    pub(super) fn new(enabled: bool, size: usize) -> Self {
        Self {
            enabled,
            entries: (0..if enabled { size } else { 0 })
                .map(|_| OnceLock::new())
                .collect(),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
        }
    }

    /// Computes at most once per valid index, with no shared map mutex.
    /// Disabled caches delegate every lookup, retaining no values or counters.
    /// An enabled cache returns `None` for an index outside its defining pool.
    pub(super) fn get_or_compute(
        &self,
        index: u32,
        compute: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        if !self.enabled {
            return compute();
        }
        let cell = self.entries.get(usize::try_from(index).ok()?)?;
        if let Some(result) = cell.get() {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return result.clone();
        }
        let mut computed = false;
        let result = cell.get_or_init(|| {
            let result = compute();
            computed = true;
            self.misses.fetch_add(1, Ordering::Relaxed);
            if result.is_none() {
                self.failures.fetch_add(1, Ordering::Relaxed);
            }
            result
        });
        // A caller that waited for another initializer is still a hit.
        if !computed {
            self.hits.fetch_add(1, Ordering::Relaxed);
        }
        result.clone()
    }

    /// Exact after workers finish; observations during initialization may lag.
    pub(super) fn stats(&self) -> LookupStats {
        LookupStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn concurrent_lookup_computes_once_including_none() {
        for expected in [Some("resolved".to_owned()), None] {
            let cache = LookupCache::new(true, 1);
            let calls = AtomicUsize::new(0);
            let barrier = Barrier::new(8);
            let results = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..8)
                    .map(|_| {
                        scope.spawn(|| {
                            barrier.wait();
                            cache.get_or_compute(0, || {
                                calls.fetch_add(1, Ordering::Relaxed);
                                expected.clone()
                            })
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap())
                    .collect::<Vec<_>>()
            });
            assert!(results.iter().all(|result| result == &expected));
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(
                cache.stats(),
                LookupStats {
                    hits: 7,
                    misses: 1,
                    failures: usize::from(expected.is_none()),
                }
            );
            assert_eq!(
                cache.get_or_compute(0, || panic!("initialized entry should hit")),
                expected
            );
            assert_eq!(cache.stats().hits, 8);
        }
    }

    #[test]
    fn indexes_and_pool_instances_are_isolated_and_results_are_cloned() {
        let first_pool = LookupCache::new(true, 2);
        let second_pool = LookupCache::new(true, 2);
        assert_eq!(
            first_pool.get_or_compute(0, || Some(vec![1])),
            Some(vec![1])
        );
        assert_eq!(
            first_pool.get_or_compute(1, || Some(vec![2])),
            Some(vec![2])
        );
        assert_eq!(
            second_pool.get_or_compute(0, || Some(vec![3])),
            Some(vec![3])
        );
        let mut result = first_pool
            .get_or_compute(0, || panic!("should hit"))
            .unwrap();
        result.push(9);
        assert_eq!(
            first_pool.get_or_compute(0, || panic!("should hit")),
            Some(vec![1])
        );
        assert_eq!(
            first_pool.get_or_compute(1, || panic!("should hit")),
            Some(vec![2])
        );
        assert_eq!(
            second_pool.get_or_compute(0, || panic!("should hit")),
            Some(vec![3])
        );
        assert_eq!(first_pool.stats().misses, 2);
        assert_eq!(second_pool.stats().misses, 1);
        assert_eq!(first_pool.stats().failures, 0);
    }

    #[test]
    fn disabled_cache_recomputes_without_values_or_counters() {
        let cache = LookupCache::new(false, 2);
        let calls = AtomicUsize::new(0);
        for index in [0, 0, 1, u32::MAX] {
            let expected = calls.load(Ordering::Relaxed);
            assert_eq!(
                cache.get_or_compute(index, || { Some(calls.fetch_add(1, Ordering::Relaxed)) }),
                Some(expected)
            );
        }
        assert_eq!(cache.get_or_compute(0, || None), None);
        assert_eq!(calls.load(Ordering::Relaxed), 4);
        assert!(cache.entries.is_empty());
        assert_eq!(cache.stats(), LookupStats::default());
    }

    #[test]
    fn out_of_bounds_indexes_do_not_invoke_lookup() {
        let cache = LookupCache::<String>::new(true, 2);
        for index in [2, u32::MAX] {
            assert_eq!(
                cache.get_or_compute(index, || panic!("invalid index")),
                None
            );
        }
        let empty = LookupCache::<String>::new(true, 0);
        assert_eq!(empty.get_or_compute(0, || panic!("empty pool")), None);
        assert_eq!(cache.stats(), LookupStats::default());
        assert_eq!(empty.stats(), LookupStats::default());
    }
}
