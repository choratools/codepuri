# Samsung Pass: current in-memory cache validation

The exporter enables seeded SSA, immutable member-resolution and shared inventory
caches by default. `JavaSession` additionally retains generated Java sources and
the report for identical repeated exports. Analysis caches live for one export;
the generated source snapshot lives until the session is cleared or dropped.
Neither cache persists across process exits.

## Latest full APK cold/warm measurement

One frozen release executable, Samsung Pass5.2.10.10,30 workers, source release25,
and four explicit genuine JDK17/Android35 classpath archives. Both phases wrote
all sources to separate fresh directories.

| Metric | First render and delivery | Cached replay and delivery |
|---|---:|---:|
| Export wall time | 350.646405s | 1.507423s |
| Java files | 31,188 | 31,188 |
| UTF-8 source text bytes | 308,670,036 | 308,670,036 |

APK/classpath setup took1.417369s outside those export timings. Warm replay was
232.61x faster in this single pair. This measures avoiding repeated decompilation,
not accelerating the first decompilation by that factor. Workspace tests ran
concurrently; cold times from earlier differently loaded runs are not a
controlled performance A/B.

Independent SHA-256 comparison of every generated file found zero changed,
added or missing files. Exact reports and ordered checksums also match. Counters
confirm one render and one replay. All205,497 original method identities retain
their wave5 status; zero accepted bodies gained or lost. Code-bearing coverage
remains90.782661%, with17942 original definitions rejected.

The retained text payload is294.37MiB, excluding metadata/allocator overhead.
Whole-process peak RSS is12,857,980KiB (12.26GiB), including first-render analysis.
The process recorded zero swaps. Actual entries are allocated as needed; the
implementation does not reserve all128GB physical memory.

Current gates:829 workspace tests passed, zero failed,78 ignored; strict
workspace/all-target Clippy, fmt and release build passed. Frozen source hashes
remain unchanged. The source syntax-gate result is in
`samsung-pass-cache-wave6-syntax.txt`; whole-app compilation and Android runtime
equivalence are not established by cache parity.

## Added original-input inheritance index

`java_source_throws_integration.rs` now caches each candidate owner's completed
original-input ancestor closure while building immutable abstract-to-implementation
links. The index retains full original method keys, returns, parameter order,
candidate order and duplicates. Unknown/duplicate/external owner boundaries and
cycles retain the previous traversal semantics. Complete alias, override, native,
metadata and external checked-exception bounds still run for every consumed link.
The cache never crosses exports or changes inputs.

All11,201 genuine original abstract link vectors match the previous scan exactly:
36,518 positive links from1,452,305 candidate pairs. Canonical old/new vectors
have identical SHA-256: `fdc8135fb82d536ed7dd3425924987f892fd705325ba91d4a7747791714f16d6`.

The isolated linkage benchmark measured3.108213s for the old scan versus
0.311934s index construction plus0.004767s for all lookups:0.316701s total,
approximately9.81x for this stage alone. It is not a whole-export speedup claim.
Retained index payload lower bound is12,176,015bytes; temporary ancestor payload
lower bound is4,571,483bytes. BTree node/allocator and other temporary overhead
are excluded.22,593 completed owner closures cover89,740 encountered names.

Validation includes three synthetic old-oracle tests, entire-APK ordered-vector
parity, exact original Glide/GMS source contracts and independent exhaustive
reachability-model parity over36,864 comparisons.

## API and evidence

Keep the same `JavaSession` alive for repeated `emit` calls, using a fresh sink
directory each time. `sources()` borrows cached source objects without copying
text; `stats()` reports retained text and render/replay counters. `clear_cache()`
releases sources/report. Changed APKs, classpaths or options require a new session.

- `samsung-pass-cache-wave6-{environment,comparison,inventory,coverage}.json`
- `samsung-pass-cache-wave6-{run,time,verification,workspace-tests,clippy,fmt}.txt`
- `/tmp/opencode/samsung-pass-cache-wave6/replay/{cold,warm}/sources`
- `/tmp/opencode/wave6-throws-index/evidence.json` and canonical vector files

Existing examples of these cache mechanisms include
[jadx's generated-code cache](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/api/impl/InMemoryCodeCache.java)
and [Ghidra's persistent initialization interface](https://ghidra.re/ghidra_docs/api/ghidra/app/decompiler/DecompInterface.html).
Those sources explain reuse mechanisms, not comparative same-workload speed.
The active99% recovery goal remains incomplete.
