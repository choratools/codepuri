# Source-throws cold-run cache candidates

Measured wave13 source_throws48.735466s and alias_build22.888832s are separate stages under loaded-host conditions. Existing collect_cached retains frontend SSA; final throw contracts/provenance are still rebuilt. Warm source replay is not cold-run acceleration.

1. Per-collection immutable declaration/contract results and failures keyed by original dex index + method index. Super selection must use caller identity + reference, not a plain method key. Reuse caller contract and descriptor/exception decoding.
2. Per-method validated instruction-offset and unique handler-offset indexes. Preserve duplicate/overlapping handler rejection and original prethrow states. Do not overwrite duplicates in a map.
3. Completed handler-root and override-contract memoization, never partial recursion results; retain Phi/cycle failure semantics and full original method identity.

Collection-local cache is scoped after ordered inputs, external metadata, hierarchy and aliases are frozen. A cross-collection cache also requires content/classpath/catalog/alias/proof-version provenance. SSA identity alone is insufficient.

Before implementation credit: cached vs uncached exact equality for unavailable roots, original/derived/source throws, identities, dependencies and provenance, including super dispatch, unknown/changed metadata, duplicate handlers and Move/Phi cycles. Measure cold misses/hits and stage time separately from replay. derive_program already uses reverse dependency worklist; no invented SCC attribution. No implementation or speedup claimed in this note.
