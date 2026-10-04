# Reusing completed Java assignment checks

The backend now retains completed class-assignment results within each
`Hierarchy`. Repeated checks of the same ordered `(actual, expected)` class pair
reuse the exact `Compatibility` result, including incompatible, missing-metadata
and cycle outcomes. Primitive, identity and Object shortcuts retain their
existing behavior; array covariance continues through the original rules.

Only completed top-level queries are stored. Intermediate DFS cycle observations
depend on the active path and are not independently cached. The graph walk runs
outside the cache lock. Concurrent first requests can duplicate a walk but retain
the same immutable result. New accepted declarations invalidate all cached
results; unchanged or rejected declarations do not. Clones and renamed graphs
have independent caches.

Memory grows with queried class pairs rather than reserving the machine's RAM.
There is no process-global cache, cross-APK reuse or disk persistence. Existing
SSA/member caches remain enabled by default, and `JavaSession` continues to
retain exact sources/reports for identical repeated exports.

## Controlled release microbenchmark

One synthetic inheritance chain, depth 64, 20,000 repeated queries of one pair:

| Path | Time |
|---|---:|
| Original uncached ancestor walk | 0.360498 s |
| Memoized assignment query, including first miss | 0.000838 s |

This favorable repeated-query case measured approximately 430x throughput. It
is not a whole-APK benchmark or a prediction of cold Samsung Pass speedup.

Command:

```sh
cargo test --release -p backend-java hierarchy_cache_tests -- --include-ignored --nocapture
```

All five tests passed: invalidation/negative-result reuse, clone/name isolation,
uncached parity on cyclic/missing/diamond graphs and arrays, concurrent queries,
and the separately labeled microbenchmark.

Twelve coupled type/throws/phi regression tests passed (one existing contextual
corpus test remained ignored). Backend all-target strict Clippy and touched-file
format checks passed. Exact test counts and source hashes are recorded in
`hierarchy-memory-cache-validation.json`.

Existing session tests were also rerun: five ordinary tests passed, followed by
the opt-in actual Samsung Fragment snapshot test (one passed in 46.67 s). This
validates source replay and static-initializer preservation, not a new full-APK
cold/warm timing pair. The previous frozen full-export measurement remains
189.117 s cold versus 1.436 s warm, with exact file/report parity; see
[the session evidence](SAMSUNG_PASS_SOURCE_SESSION.md).

The current recovery work changes backend coverage independently. A fresh
frozen whole-APK comparison is required to claim overall speedup for this new
hierarchy cache.
