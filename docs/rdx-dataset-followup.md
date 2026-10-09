# RDX review follow-up against the 17-app method dataset

This supplements, rather than rewrites, `rdx-algorithm-review.md`. The RDX review
is source inspection of the pinned RDX revision, not a matched export benchmark.
The new dataset uses Codepuri profiling and JADX JSON exports; it contains no
RDX runtime measurements and no newly executed per-method correctness oracles.

## Dataset evidence

- 4,894,560 original definitions across 17 APKs; 4,894,344 are in Codepuri's
  root-DEX scope. 4,620,319 definitions have code, with no strict decode errors
  in the successfully parsed DEX inputs.
- 132,188 cross-app symbolic-code fingerprint candidate groups with at least
  eight code units. These are candidates for shared regression fixtures, not
  proof that the APKs contain the same complete library or equivalent behavior.
- 591,312 root-scope identities are explicitly unavailable/unpublished in
  Codepuri but have marker-free JADX body observations: 265,800 constructors,
  27,435 static initializers and 298,077 ordinary methods.
- Conversely, 26,672 Codepuri body observations have explicit JADX diagnostic
  markers. Neither asymmetry establishes correctness without an original-method
  compiler/runtime oracle.
- 4,225 original methods across all 17 apps contain at least 100 decoded branch
  instructions. This is a useful structured-control-flow population; it is not
  a measured count of dominator-tree nodes or dominance queries.

## 1. Common dominance intervals and method analysis reuse — highest speed priority

The original RDX review correctly identifies the parent walk in
`decomp-core/src/analysis/dominance.rs:25`. Natural-loop discovery calls it for
every edge and predecessor. Worst-case query work scales with graph edges times
dominator-tree height, even for negative back-edge queries on long chains.

However, Codepuri already implements interval queries in:

- `managed-ir/src/verify/uses.rs` (`DominanceIndex`), with parent-walk comparison
  tests including unreachable nodes;
- `dex-decompiler/src/ssa/phis.rs` (`dominance_intervals`), also validated against
  parent walks.

Thus the opportunity is to consolidate an existing technique into the shared
analysis result, not add a new technique everywhere independently.

There is a second cost that intervals alone cannot fix:

- `backend-java/src/object_conditional_allocations.rs:147` constructs dominators
  inside a value availability query.
- `backend-java/src/object_argument_regions.rs:106` scans all instructions for
  each allocation root; line 127 attempts whole-method structure planning;
  lines 193–209 repeat the full initializer scan for nested allocations.

Recommended experiment: count dominator/structure constructions and query
counts in one immutable method session; share the analysis result, then add
interval lookup. Preserve original CFG/exception-edge identity, unreachable
semantics and context-sensitive source decisions. Measure construction time,
query time, extra memory and complete output, separately.

### Concrete workload

Wallet's `TicketDAO_Impl` class consumed 22.33 s, with 22.30 s attributed to
`structure_backend` in this diagnostic run. Its largest `k`, `p`, `q` methods
each have 2,797 code units, 169 branches, 361 invocations and one try range.
Several finish explicitly unavailable with
`invoke checked exceptions are not declared by caller`, while JADX emits
marker-free body records. This is evidence of expensive rejected processing,
not proof that a specific dominance query caused the 22 s.

It should become a bounded fixture for measuring repeated analysis and checking
whether earlier *equivalent* rejection is possible. Catch coverage and checked
bridge recovery can make a naive early rejection incorrect; do not simply skip
the expensive proof or apply a smaller work budget.

## 2. Initialization/enum plan architecture — highest recovery priority

In the 591,312-identity asymmetric queue, coarse reason buckets are:

| Ordered reason bucket | Identities |
|---|---:|
| Initialization | 261,866 |
| Enum | 76,947 |
| Fields | 98,533 |
| Allocation/initializer backend | 31,445 |
| Exact super binding | 24,389 |
| Other | 98,132 |

Buckets are text-based and order-sensitive, not an exhaustive bug taxonomy.
Large individual reasons include `EnumConstructorsUnavailable` (68,960),
initialization `InvalidMetadata` (43,100), unresolved field metadata (39,812),
required genuine field declarations (32,692), and exact super binding (24,389).

Inspect the *first* failed proof and its original dependencies before assuming
missing bytecode support. A rejected family can erase many original identities;
class-level family rejection and per-body support must be reported separately.
Keep genuine final assignment, allocation timing, reentrancy and exception
identity obligations rather than importing RDX's readable staging policy.

`GeneratedLayoutsKt` is a cross-app target: Notes 12.18 s, SmartThings 7.92 s,
Audible 5.17 s class time, largely initialization planning. Notes' `<clinit>` has
16,786 code units and 1,140 decoded allocations and is explicitly unavailable
with `FieldInitializationUnavailable`. Versions differ; compare exact
fingerprints and proof paths instead of treating those timings as repetitions
of identical code.

## 3. Typed plans with shared effect evidence — adopt design, not trace normalization

The RDX shrinker design in the original review is useful: typed expressions,
single-use constraints, live-after capture preservation and validated effects.
Codepuri also has guarded inliners and allocation/effect proofs; the architectural
gap is making immutable evidence reusable across validation and rendering.

The observed repeated initializer scans and availability work support this
direction. Begin with one allocation-expression path, not a generic rewrite of
all output. Preserve resolving-string/class-literal behavior, wide register
identity, successful constructor commit and OOM-before-arguments semantics.
Candidate cloning plus repeated rendering would recreate the current problem.

## 4. Selective class viewing — latency opportunity, separate contract

RDX's selected-class reconstruction can improve interactive latency. Codepuri's
class filter still performs whole-input preparation and source-throws analysis;
earlier focused Notes runs spent much of their wall time outside the selected
class task. An export-scoped shared snapshot is groundwork, not on-demand analysis.

Propose an explicit analysis/view API with bounded dependency discovery and
unresolved external/source-publication receipts. It must not silently claim the
same global publication/closure guarantee as full export. Measure cold selected
class latency and subsequent reuse separately from whole-APK throughput.

## 5. Trigram search index — defer for export performance

The RDX trigram filter is relevant to browsing the many generated source files,
not a fix for cold decompilation. It is useful only after a search feature exists;
exact matching must still read candidate text, with Unicode/regex bypass rules.

## Next bounded experiments

1. Add pass-construction and dominance-query counters to the Wallet DAO fixture;
   distinguish rejected work from accepted rendering and graph query costs.
2. Consolidate dominance interval logic in an immutable analysis snapshot and
   compare against existing parent-walk/property tests before end-to-end claims.
3. Choose exact enum/initializer asymmetric identities across multiple apps and
   execute original-method source/runtime oracles; start with the first failing
   dependency, not just the final family-level reason.
4. Use shared-code fingerprint candidates to build a small, diverse regression
   corpus. Retain app-specific metadata/alias snapshots to detect environmental
   differences, rather than counting repeated libraries as independent evidence.

Dataset queries and reproduction: `bench/METHOD_COMPARISON.md` and
`bench/method_comparison_queries.sql`. Missing per-method timings, pass counts
and correctness validations remain explicit in the dataset configuration.

## Implemented after this review

The core dominance/post-dominance query recommendation is now implemented with
lazy shared intervals, and SSA/IR verification use those core queries. Backend
allocation analysis caches dominators, post-dominators, extended structure
success/errors and sparse operand/constructor/condition relationships.
Successful-completion cuts use bounded shared facts followed by a single
analysis-only success-dominance index; 19,683 small normal/exception graphs
matched the prior traversal, including intersections.

This does not make initial CHK construction or output generation O(1), and it
does not alter RDX throughput evidence. Paired APK results and precise complexity
limits are recorded in `bench/QUERY_CACHE_OPTIMIZATION.md`.
