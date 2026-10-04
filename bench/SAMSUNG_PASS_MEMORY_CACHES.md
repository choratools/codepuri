# Export-session memory caches

The Java exporter enables three caches by default. They retain immutable results
within one export, then release them before returning. They do not persist between
separate CLI invocations or serve as a disk cache.

For repeated exports in one process, the separate `JavaSession` additionally
retains generated sources and reports. Its measured Samsung Pass cold render
took 189.12 s and cached replay took 1.44 s, with exact file/report parity. See
[the source-session report](SAMSUNG_PASS_SOURCE_SESSION.md) for workload and limits.

| Option | Reused result | Identity and validity |
|---|---|---|
| `JavaOptions::cache_ssa` | Seeded frontend SSA, including typed failures | Exact DEX, method, CodeItem identity and sorted initial-register types; one immutable export |
| `JavaOptions::cache_resolutions` | Java method/field signatures, including unavailable lookups | Per-DEX pool index after aliases, metadata and throws preparation are frozen |
| `JavaOptions::cache_inventory` | Native/reflection capability evidence | Immutable whole-input inventory snapshot, shared with `Arc` rather than copied per class |

SSA uses per-key `OnceLock` initialization: concurrent requests compute the same
method once, outside the map mutex. The resolution caches use dense per-pool
`OnceLock` entries. Public inventory APIs still return their original owned evidence;
internal shared APIs use the same proof functions and retain order, duplicates,
observation kinds and reasons. Inventory mutation invalidates its shared snapshot.

All six frontend routes and the source-throws prepass use the SSA cache. Cache
release time is included in export wall time. Caller-owned metadata must remain
unchanged during an export. No cache entries are reused for a different APK.

## Controlled Samsung Pass measurement

Two sequential exports used the **same frozen release executable**, APK and
explicit JDK17/Android35 classpath, 30 workers and actual source writes. Only
`cache_inventory` varied; SSA and resolution caching remained enabled in both.

| Measurement | Inventory copies | Shared inventory cache |
|---|---:|---:|
| Export wall time | 183.848050 s | 179.400434 s |
| User CPU time | 2,232.02 s | 2,204.25 s |
| Peak RSS | 9,627,836 KiB | 9,776,120 KiB |
| Generated Java files | 30,530 | 30,530 |
| Explicit complete bodies | 160,431 | 160,431 |
| Proven enum equivalents | 332 | 332 |
| Original definitions audited | 205,497 | 205,497 |

This single pair measured **2.42% less wall time** (1.0248x throughput). It does not
establish a statistically reliable speedup. Earlier SSA/resolution cache A/B pairs
did not establish overall acceleration; extra RAM alone does not remove backend
and initialization work.

The shared run recorded 274,525 SSA hits and 194,286 misses, with zero frontend
failures. Resolution recorded 4,705,168 hits, 400,794 misses and 71 unavailable
results. Unavailable metadata results are cached, not silently repaired.

Peak exporter RSS was about **9.32 GiB**. The host reported 125 GiB physical memory
and about 57 GiB available at the start. Cache storage grows with actual input
results; the exporter does not reserve or fill all physical RAM.

## Preservation and validation

- Original method identity/status comparison: **zero gained, zero lost, zero status
  changes**, with the same 205,497-definition denominator.
- Exact source-byte comparison: **failed** for 4,067 existing files, with no added
  or missing paths. Generated label/state ordering can vary between exports; this
  is not an exhaustive semantic-equivalence proof for every changed file.
- Cache-focused unit tests: **17 passed** (single initialization, exact key
  separation, failed-result reuse, session isolation, inventory snapshots and
  shared/public proof parity).
- Facade regression: **passed**, comparing cache ON/OFF, 1/30 workers, profiled and
  unprofiled calls, limits 0/3/unlimited, exact report and source bytes for the fixture.
- Independent read-only review found no concrete cache correctness defect.
- Current workspace tests, all-target strict Clippy and formatting checks passed;
  exact totals are recorded in `samsung-pass-memory-cache-summary.json` and the
  full test output in `samsung-pass-memory-cache-workspace-tests.txt`.

The full export is a source subset. This benchmark does not establish whole-app
Java compilation, native implementation recovery or Android execution.

## Reproduction and evidence

Profiler options `--no-ssa-cache`, `--no-resolution-cache` and
`--no-inventory-cache` disable the respective cache for comparisons.

- Configuration and executable/input hashes: `samsung-pass-memory-cache-environment.json`.
- Commands and process resource measurements: `samsung-pass-memory-cache-{off,on}-time.txt`.
- Output roots: `/tmp/opencode/samsung-pass-memory-cache-{off,on}`.
- Full source comparison: `samsung-pass-memory-cache-comparison.json`.
- Exact original identities: `samsung-pass-memory-cache-identity-comparison.json`.
- Original-input inventories: `samsung-pass-memory-cache-{off,on}-inventory.json`.
- Tests: `samsung-pass-memory-cache-tests.txt`, `samsung-pass-memory-cache-facade-tests.txt`.

Inventory `report.txt` files explicitly identify their origin as
`java_profile_summary`; counters are translated from the actual profiler report.

## Primary-source comparison

Other decompilers cache at different layers:

- [jadx ClassNode](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/core/dex/nodes/ClassNode.java)
  checks its code cache before generation and stores the resulting class code;
  [InMemoryCodeCache](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/api/impl/InMemoryCodeCache.java)
  uses a concurrent map. This explains reuse of an already generated class, not
  comparative cold whole-APK throughput.
- [ILSpy MetadataModule](https://github.com/icsharpcode/ILSpy/blob/master/ICSharpCode.Decompiler/TypeSystem/MetadataModule.cs)
  lazily caches known attribute types and instances in indexed arrays.
- [Ghidra DecompInterface](https://ghidra.re/ghidra_docs/api/ghidra/app/decompiler/DecompInterface.html)
  keeps a persistent process interface and initialization state; its documentation
  also requires attention to flushing stale function/symbol information.

These are implementation mechanisms, not evidence that one tool is faster on
the same complete workload. Decomphify provides export-scoped analysis reuse and
the separate `JavaSession` for in-process generated-source replay. Neither cache
persists across processes; see the source-session report above for measurements.
