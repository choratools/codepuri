# Method-local query indexes and caches

## Scope and complexity

This change caches immutable method facts and analysis results within one
frozen IR/resolver session. It preserves live materialization, pending allocation
and source publication checks. Static initializer validation/rendering share
the same immutable analysis where their input IR is identical.

| Repeated operation | Before | After preparation |
|---|---|---|
| Dominance/post-dominance query | Parent walk, O(tree height) | Expected O(1) ID lookup and interval containment |
| Backend dominator/post-dominator construction | Repeated per allocation/availability check | One lazy snapshot per method |
| Extended CFG structure feasibility | Repeated per allocation root | Cached success/error snapshot, shared by Rc |
| Instruction to CFG block | Full search in local argument-region path | Expected O(1), preserving first duplicate payload |
| Operand consumers | Full arena pass per wanted-value batch | One raw index; cached per-value lists, O(1) lookup plus returned uses |
| Wide invocation normalization | Repeated method lookup and operand normalization | Lazy relevant-signature and per-instruction normalization caches, including unknown metadata |
| Constructor receiver/branch condition lookup | Full arena scans | Expected O(1) lookup plus matching candidates |
| Synthetic payload register membership | Repeated register-slot scan | One membership index; expected O(1) query |
| Constant consumer constraints | Full MethodIndex rebuild per constant | Shared ID-backed MethodIndex; only relevant consumers rechecked |
| Allocation root | Repeated copy-chain walk | Path-compressed positive/negative cache |
| Definition type evidence | Repeated immutable definition traversal | Shared result/error cache |
| Successful-completion availability | Two graph walks and set/intersection allocation per cut | Shared small-cut facts, then one success-dominance index; warm membership queries expected O(1) |

The initial graph/operand analysis is not O(1). Returning K matched uses remains
O(K), and final source generation remains proportional to output. Core intervals
are lazily built in O(B) on the first nontrivial query; root/self/immediate-parent
queries need no interval construction. SSA phi checking and IR verification now
use the common core intervals rather than building separate copies.

## Successful-completion index

An analysis-only graph inserts one success node for each original source with
normal outgoing transfers. Exceptional edges bypass that node. Dominance of
the success node is exactly the original cut traversal's normal-completion
condition. Original instruction identity is retained, and the queried instruction
itself follows the existing inclusion rule. The original CFG is not mutated.

The index excludes unreachable instructions. Duplicate payloads or invalid CFG
structure retain the original traversal. A method caches at most four explicit
cut results (including failures), then promotes further distinct queries to the
shared index. This avoids paying for an extra whole-graph analysis on one-off
queries and bounds retained set memory by a constant multiple of graph size.
Pairwise cached-set or indexed-cut intersections remain compact predicates;
actual sets are materialized only where an existing consumer requires them.

## Validation

- Exhaustive differential comparison against the previous cut traversal on all
  19,683 three-node graphs with absent/normal/exceptional edges, including loops,
  joins and unreachable nodes. Membership and intersections agree.
- Duplicate-payload fallback and small-cache promotion tests.
- Raw/normalized read indexes preserve arena order, operand multiplicity,
  invalid-wide fallback and off-CFG consumers. Relevant resolver calls are reused;
  unrelated invokes are not resolved eagerly.
- Positive/error evidence cache tests retain dynamic allocation readability.
- Dominance/property tests match brute-force graphs; an 8,193-block chain checks
  reachability, post-dominance and iterative construction.
- Final full workspace run: 1,278 passed, 175 ignored, zero failures, with
  explicit JDK17. Core analysis/property tests and original-bytecode/source
  runtime fixtures passed. Clippy with all targets/features and warnings denied
  passed; format/diff checks and the release CLI build passed.

## Controlled query scaling

The ignored release test `repeated_dominance_query_scaling` measures negative
neighboring-chain back-edge queries against a parent-walk reference. It is a
microbenchmark, not an APK throughput claim.

| Chain nodes | Parent visits | Indexed queries | Parent walk | Indexed |
|---|---:|---:|---:|---:|
| 1,024 | 523,776 | 1,023 | 21.25 ms | 49.22 us |
| 4,096 | 8,386,560 | 4,095 | 314.61 ms | 210.42 us |
| 16,384 | 134,209,536 | 16,383 | 6.83 s | 998.77 us |

```bash
cargo test -p decomp-core --release --test analysis_cases \
  repeated_dominance_query_scaling -- --ignored --nocapture
```

## Paired APK measurements

Reference: detached build of `3d54ef3`. Same APK, Android35 `android.jar`,
30 workers and fresh roots. Two before/after pairs alternate tool order.
These are profiled/checksummed exports, not plain-CLI/JADX comparisons.

| Workload | Old export median | New export median | Old user CPU | New user CPU | Peak RSS old/new |
|---|---:|---:|---:|---:|---:|
| Wallet, file output | 113.74 s | 110.89 s | 1,029.62 s | 969.63 s | 19.08 / 19.06 GiB |
| Notes, file output | 85.44 s | 83.94 s | 700.98 s | 650.59 s | 14.39 / 14.37 GiB |
| Audible, no file output | 123.34 s | 118.67 s | 1,205.67 s | 1,188.36 s | 17.61 / 17.60 GiB |

Class-task medians in the full file-output runs:

| Class | Before | After |
|---|---:|---:|
| Wallet `TicketDAO_Impl` | 22.32 s | 20.10 s |
| Notes `SpenInputEmojiList` | 9.72 s | 6.61 s |
| Notes `GeneratedLayoutsKt` | 12.04 s | 7.19 s |

A focused `GeneratedLayoutsKt` run measured 6.96 to 2.78 s class work with
the same source checksum, but that isolated result is not the full-export gain.
Earlier exploratory runs had substantial host contention; their 100–400 s
whole-export variation is not counted as improvement. An eager success-index
variant improved the layout class but regressed Wallet full export, motivating
the bounded small-query cache before promotion.

In both file-output pairs, all 103,045 Wallet and 66,345 Notes paths, report
counters and original emitted/unavailable/unpublished identity outcomes were
unchanged. No non-comment Java differences were found. Wallet had 8 and 7
comment-only file differences; Notes had 1 and 2. Exact rejection text can vary
in selected offending invoke/SSA IDs; this also occurs between unmodified
reference runs. Exact diagnostic-string identity is not claimed.

Local artifacts: `/tmp/opencode/query-cache-bench/`, including
`adaptive-performance.json`, `adaptive-validation.json`, before/after logs and
GNU time receipts. The reference worktree is local and not committed.
