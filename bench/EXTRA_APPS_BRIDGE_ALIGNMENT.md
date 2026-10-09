# Additional installed-app profiling and bridge ancestor reuse

Seven additional single-APK apps were pulled from ADB device `RFCXA1KQ0DX`.
Local APK paths, SHA-256 hashes and DEX inventory are recorded in
`/tmp/opencode/adb-extra7/manifest.json`; APKs are not included in the repository.

Measurements use the release `java_profile` example, 30 workers, Android 35
`android.jar`, fresh output directories and file output enabled. Export times
exclude APK parsing and profiler report serialization; profiler checksumming is
included. Each value is a single run, not a repeated-run median.

| App | DEX MiB | Export seconds | Java files | Complete ordinary methods |
|---|---:|---:|---:|---:|
| Audible | 105.4 | 164.6 | 133,456 | 413,832 |
| Samsung Internet | 36.8 | 38.4 | 41,416 | 157,685 |
| Newegg | 33.2 | 34.8 | 32,797 | 130,807 |
| eero | 56.0 | 64.3 | 53,690 | 201,617 |
| Amazon Vendor | 11.6 | 13.0 | 11,988 | 39,670 |
| Acronis | 8.3 | 8.7 | 9,350 | 41,067 |
| Bitmoji | 16.8 | 19.1 | 15,943 | 91,576 |

## Audible bottleneck and change

Bridge alias alignment consumed 43.56 s (26.5%) of the baseline export.
Focused instrumentation found 2,583 seed candidates; seed collection completed
in 1.82 s and indexing by 4.45 s, with total alignment reaching 42.83 s.
Only six families were committed. Repeated ancestry traversal during family
validation was an avoidable cost.

`java_aliases_bridge.rs` now reuses the existing alignment-local ancestor cache
for seed owners and narrow-signature candidates, in addition to the existing
wide-family path. Input and external metadata are immutable during this call;
missing ancestry still rejects the candidate rather than being guessed.

| Audible measurement | Before | After |
|---|---:|---:|
| Bridge alias alignment | 43.56 s | 34.58 s |
| Full profiled export with file output | 164.60 s | 155.07 s |

All 133,456 generated Java files were byte-identical, including comments.
The ordered checksum remained `72b381e0d7895595` and output size remained
1,083,385,202 bytes. An earlier source-name/signature indexing experiment did
not improve timing and was removed.

Validation: 10 default bridge-filtered tests passed (15 ignored), the explicitly
enabled genuine covariant bridge family test passed, Clippy with warnings denied
passed, formatting/diff checks passed, and the release CLI build passed.

Remaining measured costs include Audible's 34.58 s bridge alignment and its
whole-program source-throws analysis (21.84 s in the baseline). These remain
optimization targets; this change does not eliminate them.

## Follow-up: named descendant index

Further candidate-level instrumentation attributed about 27 s to repeated
wide-family scans for Kotlin `invoke()` bridges. Roughly 2,560 candidates each
scanned all operands with the same name and parameters, even though most owners
were unrelated. The existing ancestry cache avoided repeated graph traversal but
did not avoid this repeated family-wide scan.

Build a descendant-method index once per original name/parameter pair. Candidate
checks visit only operands descended from the target owner. The index still
requires complete ancestry for *all* original named operands; unknown ancestry
invalidates the indexed family, preserving the previous fail-closed behavior.
Keys are indexed in their original ordered traversal, and current alias values
are still checked after lookup.

| Measurement | Initial baseline | Ancestor cache | Descendant index |
|---|---:|---:|---:|
| Audible bridge alignment | 43.56 s | 34.58 s | 15.20 s |
| Audible full profiled export | 164.60 s | 155.07 s | 136.96 s |
| eero bridge alignment | 11.55 s | not measured | 4.98 s |
| eero full profiled export | 64.34 s | not measured | 59.09 s |

Audible retained all 133,456 files and eero all 53,690. Full report counters
were unchanged for both. File-by-file comparisons found one Audible file and
two eero files with diagnostic-comment differences, and no non-comment source
differences. Unlike the earlier ancestor-cache run, these complete outputs are
not byte-identical.

All five `bridge_families` tests passed with ignored tests explicitly enabled,
including original descriptor dispatch, final-owner descendant restrictions,
genuine Samsung covariant bridges and fail-closed mutation cases. Clippy with
warnings denied passed. Measurements remain single-run observations.

## Follow-up: narrow candidates and direct root lookup

Reuse the named descendant index for exact narrow-return candidates as well.
An accepted wide family already requires ancestry completeness for every named
operand, so moving that check earlier does not admit unknown or ambiguous
families. Method selection and current source aliases remain independently
checked. This measured 13.90 s alignment and 134.43 s full Audible export.

The broad-root check also scanned every operand with the bridge signature for
each seed. Replace this with exact `MethodKey` lookup for each known ancestor:
same name, parameters and return descriptor, restricted to genuine input
declarations. Ancestors retain sorted owner order, matching the old filtered
method-key order. This avoids unrelated operands without relaxing root checks.

| Measurement | Previous descendant index | Narrow/root lookup |
|---|---:|---:|
| Audible bridge alignment | 15.20 s | 6.13 s |
| Audible full profiled export | 136.96 s | 127.19 s |
| eero bridge alignment | 4.98 s | 2.72 s |
| eero full profiled export | 59.09 s | 59.94 s |

Audible is about 23% faster than its initial 164.60 s observation. eero's
alignment improved, but its full export did not improve versus the preceding
run; individual whole-export timings include variation in other phases.

Both apps retained identical file sets and report counters compared with their
initial baselines. Audible had one diagnostic-comment-only file difference;
eero had three. No non-comment source differences were found. All five bridge
family tests, including explicitly enabled original-bytecode dispatch and
genuine APK cases, passed again. Clippy with warnings denied passed.

## Source-throws fixed-point investigation

Focused Audible instrumentation measured cumulative `derive_program` times:
preparation 2.39 s, fixed point 5.84 s, result chunks 6.33 s, merge 6.95 s.
Thus fixed-point propagation consumed about 3.44 s. Replace repeated
`failures.contains_key(key)` calls for every event with a local failure flag,
updated at the same failure insertion. Event ordering and error precedence are
unchanged, and each worklist visit refreshes its state from the failure map.

The full export measured 132.54 s versus the earlier 127.19 s observation;
`derive_program` measured 6.87 s versus 7.13 s previously. This small phase
delta is not evidence of an overall speedup. All 133,456 generated files were
byte-identical to the root-lookup run (`7315fdf622f06c8c`). Source-throws tests:
65 passed, 22 ignored, including the owned-reference differential test and
Java exception identity checks. Clippy passed.
