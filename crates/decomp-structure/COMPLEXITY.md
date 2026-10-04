# Acyclic structuring: complexity and evidence

Both public structuring APIs and region enums are unchanged. Cycles, exceptions,
unreachable blocks, malformed dispatch and overlapping regions remain explicit
errors. Basic structuring continues to reject switches.

## Algorithm

After the existing validation and Kahn cycle check, visit blocks in reverse
topological order. Every successor already belongs to the postdominator tree.
The intersection of successor postdominator chains is their lowest common
ancestor. Attach the block to that ancestor. Terminals attach to a virtual exit,
which maps to `None` and is never emitted. Binary lifting performs LCA without
walking long chains or storing transitive sets. This is a DAG-specific alternative
to the general core CHK fixed-point algorithm; it avoids CHK's depth-dependent
intersection cost and does not change supported control flow.

Dense indices are private and do not depend on block ID numeric layout. Duplicate
successor edges remain present for the indegree check and edge provenance.
Hash-indexed switch grouping retains first-seen target order, label order and
default flags. Region construction uses explicit continuation frames, processing
true before false and switch arms in their original order. The nearest join is
excluded from arms and resumed in the enclosing sequence, exactly as before.

## Before / after

Let B be live blocks, E original edges and
X = sum(outdegree² + indegree²), the existing core validation cost.
Hash operations use expected/amortized bounds.

| Work | Before | After |
|---|---|---|
| Postdominator joins, time | O(B² + B E); O(B²) for basic grammar | O((B + E) log B) |
| Postdominator joins, space | O(B²) | O(B log B) |
| Switch target grouping | O(sum dispatch_degree²) | O(E) expected |
| Region construction | O(B + E), recursive nesting | O(B + E), explicit heap frames |
| Whole structurer, time | O(B² + B E + E + X), plus switch grouping | O((B + E) log B + X) |
| Whole structurer, extra space including output | O(B² + E) | O(B log B + E) |

No whole-CFG linear-time claim: `Cfg::validate` still rescans adjacency lists,
which matters for high-degree switches. Public enums retain their original
derived recursive Drop/Clone/Eq/Debug behavior; iterative construction does not
make arbitrary downstream operations on extremely deep trees stack-independent.

## Differential evidence

`src/postdom.rs` retains the original HashSet join algorithm under `cfg(test)`.
`lib.rs` and `extended.rs` retain their original recursive builders under
`cfg(test)`, including the original switch `.position` grouping. Oracles share
unchanged input validation, shape validation, cycle detection and provenance
collection with production. They independently select joins and build regions.

`src/differential.rs` compares entire results, including exact nested region
trees, joins, polarity, arm ordering, edge provenance and typed errors:

- 6,000 deterministic randomized DAGs, 2–31 blocks, multiple exits, empty arms,
  coincident targets, reversed branch adjacency, switches and shared labels.
  Basic accepted 401 and extended accepted 641; every result matched.
- Join maps are compared independently on every random DAG, including those
  rejected before region construction, preventing validation from hiding an LCA
  discrepancy.
- Successful trees receive independent iterative exactly-once block coverage
  checks.
- Chains of diamonds, nested conditionals and nested grouped switches at depths
  1, 2, 8, 32 and 128 compare exactly with the original.
- Explicit cyclic/exception/malformed dispatch rejects compare with the oracle.
  Existing regression tests also cover irreducible loops, crossing arms,
  separate terminals and empty/default arms.
- Both builders successfully construct 20,000-level conditional nesting.
  That test drains the public trees iteratively to isolate construction depth.

## Release scaling benchmark

Run:

```sh
cargo test -p decomp-structure
cargo clippy -p decomp-structure --all-targets -- -D warnings
cargo test -p decomp-structure --release scaling_benchmark -- --ignored --nocapture
```

Measured 2026-10-01 in this workspace. Single-sample wall times, not statistical
performance guarantees. Timings include complete structuring, validation and
output construction; input generation and output destruction are excluded.
The original runs stop at 4,096 chain links/diamonds to bound quadratic memory.
Ancestor slots report the actual binary-lifting table allocation count, not RSS.

| Shape | B | E | Original basic | Optimized basic | Optimized extended | Ancestor slots |
|---|---:|---:|---:|---:|---:|---:|
| Chain | 257 | 256 | 1.684 ms | 0.318 ms | 0.313 ms | 2,322 |
| Chain | 1,025 | 1,024 | 23.266 ms | 1.370 ms | 3.789 ms | 11,286 |
| Chain | 4,097 | 4,096 | 427.723 ms | 5.590 ms | 5.775 ms | 53,274 |
| Chain | 16,385 | 16,384 | — | 25.752 ms | 29.516 ms | 245,790 |
| Chain | 65,537 | 65,536 | — | 143.648 ms | 158.950 ms | 1,114,146 |
| Diamonds | 769 | 1,024 | 5.766 ms | 0.867 ms | 1.079 ms | 7,700 |
| Diamonds | 3,073 | 4,096 | 90.802 ms | 3.960 ms | 4.943 ms | 36,888 |
| Diamonds | 12,289 | 16,384 | 1,886.940 ms | 19.008 ms | 24.861 ms | 172,060 |
| Diamonds | 49,153 | 65,536 | — | 112.914 ms | 130.275 ms | 786,464 |
| Diamonds | 196,609 | 262,144 | — | 605.219 ms | 626.467 ms | 3,538,980 |

At the largest original comparison sizes, basic structuring improves about
76.5× for chains and 99.3× for diamonds. For a chain the original stores
B(B+3)/2 set memberships, versus (B+1) floor(log2(B)+1) ancestor slots now.
The benchmark reaches 196,609 blocks without constructing quadratic node sets.
