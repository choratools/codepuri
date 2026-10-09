# Export architecture performance review

## Conclusion

Observed duplication and ownership boundaries justify a staged architectural
refactor, not a claim that the entire decompiler algorithm is wrong. Existing
timings do not establish an exact speedup achievable by that refactor.

## Findings

### 1. Shared immutable program data is copied into per-DEX resolvers

`java_integration.rs:362` declares owned `class_catalog`, `field_names` and
`input_owners` collections in `Resolver`. At approximately lines 3773–3780,
every DEX resolver clones all three program-wide collections. Deferred annotation
resolvers repeat these clones around lines 4051–4063. This scales collection
copying with DEX count times program inventory size, although the consumers
need read-only access. This is observed deep copying, not merely an Arc clone.

Recommended first ownership change: borrow one immutable program snapshot for
these fields, retaining separate per-DEX mutable lookup caches and publication
state. Do not conflate original descriptors and alias-projected source names.

### 2. Independent phases rebuild identical input indexes

`emit_inner` constructs `SuperIndex` and field `InputIndex` around lines
3429–3448. `java_source_throws_integration.rs:1202–1217` constructs both again,
plus per-DEX `DexMetadata`. Main preparation builds `DexMetadata` around
`java_integration.rs:3673`, and deferred annotation preparation builds it again
around line 4031. Overlapping these scans is not the same as eliminating them.

Create one export-scoped input snapshot and pass the same immutable indexes to
source-throws analysis and source emission. Verify metadata precedence,
duplicate-owner treatment, and failure ordering before removing constructors.

### 3. Validation and rendering do not share a complete method plan

`java_integration.rs:2688` trial-structures a method before the body backend
structures it again. The opt-in fast-proofs flag explicitly acknowledges this
duplication but can change rejection reasons. Static initializer lowering also
validates by rendering an ordinary method, then renders again under static-final
rules. Recent object-plan reuse addresses only part of that repeated work.

Prefer an immutable validated body plan consumed by renderers, not validation
by generating and discarding source. Separate shared CFG/type/allocation facts
from context-sensitive final assignment, exception scope, naming, and source
release decisions. A cache keyed only by method identity is insufficient.

### 4. Most ordinary output is retained until global publication audits

Class tasks collect and clone `GeneratedSource` into per-task vectors; ordered
slots feed `pending_sources` around lines 3850–3958. Ordinary sources are held
through annotation work and method/field closure audits (4141–4357). The bounded
writer channel at line 3345 does not bound these earlier source buffers.

This adds a global barrier and retains rendered strings alongside publication
records. A better split is class plans and dependency receipts first, global
publication decisions second, final text rendering last. Early dependency-unit
acknowledgements and sink failures are observable contracts and must be retained.
Simply streaming provisional sources early is not a safe substitute.

### 5. Profiling mixes work duration, wait duration, and overlapping phases

In initial preparation, timers around `scope.spawn` end immediately after
spawning; timers around `join` measure only remaining wait. For example,
`super_index` near zero does not imply the index took near-zero CPU/work time.
Other `serial` entries measure preparation on a worker overlapping source-throws.
Deferred subphase durations are nested inside `deferred_annotation_prepare`.
Reported task elapsed sums are not CPU seconds.

Add separate task start/end, worker duration, parent wait and enclosing wall
intervals. Record the uninstrumented remainder and avoid summing overlapping
entries as if they partition export time. Existing 15/30/60-worker measurements
showed no benefit from raising workers above 30 on Audible, but do not identify
whether contention, scheduling, or memory bandwidth caused the plateau.

## Proposed sequence and acceptance gates

1. Fix timing attribution so the critical path and memory lifetimes are visible.
2. Introduce a borrowed immutable export snapshot for the already identical
   program indexes and catalogs. Measure Audible and a many-DEX app separately.
3. Extract validated method-plan reuse in one bounded emitter path, preserving
   unsupported reasons as well as accepted bodies.
4. Replace rendered-source buffering with plan/receipt buffering where sink
   acknowledgement rules permit it; render after publication decisions.

For each step compare complete method identities, rejection reasons, publication
receipts and generated source, plus existing Java compile/runtime oracles. Track
wall time and peak RSS with alternating before/after runs. Preserve unknown
metadata and duplicate-owner rejection behavior. Do not claim an architectural
speedup based only on unchanged aggregate method counts.

This review makes no production code changes. Existing uncommitted performance
changes are separate from the recommendations above.

## Implemented follow-up: shared export inventories

The first ownership/input-preparation refactor is now implemented:

- `java_shared_snapshot.rs` provides an export-scoped Arc snapshot for global
  class catalogs, field-name catalogs and input-owner sets. Resolver clones now
  share these inventories instead of duplicating their trees and strings.
- Mutable fixture setup uses copy-on-write isolation. Separate exports do not
  share snapshots, and per-DEX mutable resolution caches remain separate.
- Source-throws analysis accepts the exporter-prepared field and super indexes
  and the same per-DEX inference metadata. Standalone collectors retain their
  own preparation path; an unavailable prepared super index remains unavailable.
- Deferred annotation resolvers reuse the existing inference metadata rather
  than rebuilding it. Source-throws input-owner inventories are constructed
  once before resolver construction and shared across input pools.

This addresses the observed inventory copies and index duplication; validated
method-plan extraction and late text rendering remain unimplemented. Profiling
attribution limitations described above also remain.

### Validation and observations

The decomp library suite passed 231 tests (69 ignored) with explicit JDK17.
New snapshot tests establish shared allocation before mutation and isolation
after mutation. The existing source-throws equivalence test now also compares
prepared-index collection, including an absent super index, against standalone
collection. Clippy with warnings denied, format/diff checks and release builds
passed.

30-worker, Android35, file-output-enabled single-run observations:

| Audible measurement | Before snapshot sharing | Shared catalogs | Shared input indexes too |
|---|---:|---:|---:|
| Export wall | 132.54 s | 119.97 s | 126.33 s |
| Resolver preparation | 3.95 s | 0.85 s | 0.78 s |
| Deferred resolver preparation | 1.76 s | 0.55 s | 0.81 s |

Catalog preparation consistently shrank in these samples; whole-export times
vary, so the added index reuse is not claimed to further improve wall time.
Inference metadata itself measured only 0.05 s after being made explicit.
Final peak RSS was 17.64 GiB for Audible and 14.34 GiB for Notes; these values
are observations, not a matched before/after memory reduction claim.

Audible's 133,456 files were byte-identical to the immediately preceding
source-throws run, checksum `7315fdf622f06c8c`. Notes exported 66,345 files in
95.33 s; comparison against the earlier paired CLI baseline found one
diagnostic-comment-only difference and no non-comment source differences.
These examples do not establish whole-APK compilation or runtime equivalence.
