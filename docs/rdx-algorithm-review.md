# RDX algorithm review

Reviewed RDX commit `6c7016c79f41c2f9399439b90e4f2b51a0741153` from https://github.com/Ch0pin/rdx.
This is a source inspection, not a throughput or coverage benchmark.

## Concrete algorithmic advantage: dominance queries

Both projects use Cooper–Harvey–Kennedy immediate dominator construction.
RDX additionally builds DFS entry/exit intervals over the dominator tree
(`src/native_dominators.rs`, tree interval construction starting at line 157).
Dominance can therefore be answered by interval containment in constant time.
Codepuri's `crates/decomp-core/src/analysis/dominance.rs:25` walks parent links
with a hash lookup per step: O(tree height) per query. Natural-loop discovery
calls this predicate for edges and predecessor traversal. This is a concrete
data-structure improvement worth evaluating, not evidence of an end-to-end win.
Add analogous intervals for post-dominance if query profiles justify it.
Preserve unreachable-node and entry semantics; validate against the existing
parent-walk reference on chains, diamonds, loops and disconnected graphs.

## Useful architecture: selective class reconstruction

`src/native_engine.rs:64–95` renders a selected class and its nested children
against an Arc-shared class table. This offers a useful interactive path without
requiring whole-program Java publication to finish. Codepuri's global exception
contracts and publication checks remain necessary for its current export contract;
a selective viewing API should make its different guarantees explicit.

## Useful search algorithm: trigram rejection before disk reads

`src/search_index.rs:65–95` maintains a 2048-bit, two-hash trigram filter per
document. ASCII literal queries of at least three bytes can reject nonmatching
documents before loading/searching their text. Regex, short queries and relevant
Unicode/case-insensitive cases bypass filtering. False positives are acceptable;
matching must still examine actual text. The index stores source as raw UTF-8
rather than JSON-escaped text and bounds disk, metadata and document sizes.
This benefits a future interactive search feature, not cold whole-APK export.

## Readability: typed expression substitution plus effect trace validation

`src/native_java/shrink.rs:87–140` checks exact expression type and single use,
substitutes a capture into an expression tree, and accepts it only when
`render_checked` validates the candidate against the original effect trace.
It also preserves live-after captures and bounds shrinking to 16 captures.
This is a useful organizing principle for expanding Codepuri's current guarded
inliners. It is not intrinsically faster: candidate cloning and repeated checked
rendering can add work. The separate platform-normalized trace path must not be
confused with exact DEX behavior.

## SSA and exception handling

`src/native_ssa.rs` uses live-in-pruned iterated dominance frontiers and
dominator-tree renaming. Normal-success write blocks preserve pre-write register
state on exceptional edges. Values represent register words; long/double pair
coherence is left to consumers. This is worth comparing against Codepuri's
exception-state SSA tests, but no relative correctness or speed claim was
established in this review.

## Reasons not to equate responsiveness with export throughput

- Unsupported Java patterns remain visible as DEX disassembly.
- The native SSA stage itself does not infer Java types or simplify phis.
- Work budgets can stop analysis and fall back; this trades coverage for bounds.
- Search reuses session source caches and a disk index.
- RDX has repeated decode/dominator/SSA calls in multiple recovery paths too
  (`native_java/method.rs`, `finally_regions.rs`, and loop recovery modules).
  It is not uniformly a single-analysis architecture.
- The engine documentation explicitly acknowledges allocation timing differences
  in readable constructor staging.

## Ranked adoption candidates

1. Dominator-tree intervals: directly comparable, bounded implementation scope.
2. Typed expression plans with reusable effect evidence: readability design.
3. Selected-class viewing API: substantial interactive latency opportunity.
4. Trigram-filtered bounded disk index: search feature, not exporter optimization.

Before claiming a performance win, compare cold analysis and Java reconstruction
coverage on the same APK/method identities. Mixed Java/disassembly search timings
cannot establish full Java export parity.
