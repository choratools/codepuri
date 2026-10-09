# Linear allocation availability indexing

## Evidence

Focused instrumentation of Notes' `SpenInputEmojiList` recorded 6,067 calls to
`after_initializer` over its large CFG, totaling 14.28 s. The same run recorded
3,791 argument `read_index` calls, totaling 1.89 s. Each availability query
walked the CFG twice to find instructions reachable only after successful
completion of a selected initializer/definition.

## Change

`ObjectPlanning` now lazily indexes a fully reachable, unique unconditional
chain once per emitter plan. Availability is its suffix starting at the queried
instruction, including the successful boundary exactly as before. Exception
edges, joins, cycles, duplicated instruction payloads, and incomplete coverage
retain the existing general graph traversal. Off-chain instruction queries
produce an empty available set on the proven linear path.

The index retains O(block-count) state rather than caching an O(block-count)
set for every initializer. Returned availability sets are still constructed per
query; this is not a claim that all allocation planning is linear-time. Static
initializer validation/rendering can reuse the same order evidence.

## Single-run measurements

30 workers, Android35 metadata; full exports below include file output.

| Measurement | Before | After |
|---|---:|---:|
| Notes focused export, no I/O, including whole-input preparation | 64.32 s | 51.23 s |
| Notes focused class task | 20.86 s | 8.13 s |
| Notes full profiled export | 95.33 s | 90.03 s |
| Audible full profiled export | 126.33 s | 119.91 s |

Focused class checksum remained `e8d3dc6151968364` and size 722,866 bytes.
Full Notes and Audible file sets and report counters were unchanged. One Notes
file and two Audible files differed only in diagnostic comments; no non-comment
source differences were found. These are individual runs, not medians.

## Validation

- Backend library suite: 368 passed, 14 ignored, with explicit JDK17.
- Added linear availability boundary/cache regression: passed separately.
- Existing branch, Phi, exceptional allocation and capture runtime tests passed.
- Clippy with warnings denied passed; temporary instrumentation removed.

Local artifacts: `/tmp/opencode/adb-major10/profile-investigation/allocation-*`
and the `profile-linear-order` folders in the Notes/Audible APK directories.

## Compact-range follow-up

Linear queries previously still constructed a HashSet of the entire suffix.
Capture checks also intersected two such sets. `InstructionAvailability` now
represents a proven linear suffix by a shared order index and a start position.
Membership uses the position map, and intersections sharing that index use the
maximum start position. Both operations avoid suffix-set allocation. General
CFG queries retain their explicit sets; exception dispatch materializes a set
only at the existing overlay boundary that needs mutation.

| Measurement | Suffix sets | Compact ranges |
|---|---:|---:|
| Notes focused class task | 8.13 s | 5.17 s |
| Notes focused export, no I/O, including preparation | 51.23 s | 49.62 s |
| Notes full profiled export | 90.03 s | 85.78 s |
| Audible full profiled export | 119.91 s | 120.49 s |

Notes improved about 4.7% in the full export observation. Audible did not improve
in this run, so no whole-corpus gain is claimed. Focused Notes source checksum
remained `e8d3dc6151968364`. Full file sets and report counters stayed equal;
three Notes files and one Audible file differed only in diagnostic comments,
with no non-comment source differences.

Backend suite: 369 passed, 14 ignored with JDK17. The linear availability test
also checks shared-index intersections and mixed graph/range intersections.
Clippy with warnings denied passed.
