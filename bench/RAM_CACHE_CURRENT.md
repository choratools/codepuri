# Whole-export field proof caching and reusable Java output

## Current Samsung Pass measurement

One frozen executable, immutable original APK and explicit JDK17/Android35
classpath, source release 25, 30 workers, one live JavaSession. Both phases
write actual files; parsing/classpath setup of 1.366085 s is outside these timings.

| Metric | First decompile and write | Cached source replay and write |
|---|---:|---:|
| Export seconds | 317.551165 | 1.584327 |
| Java files | 31196 | 31196 |
| Retained UTF-8 text bytes | 315868104 | same |

This single cold/warm pair gives 200.43x replay throughput.
Every source path and SHA-256 and both full reports match: zero added, missing,
or changed files. Sources remain partial decompilation results with their exact
original completeness/diagnostic states; caching does not prove whole-app javac
or Android execution. This does not measure a faster initial decompilation.

Retained source text: 301.24MiB. Whole-process peak RSS:
14.79GiB, including the first analysis and
its frontend caches. There were no exporter swaps. No fixed fraction of the
128GB host is reserved; storage grows with completed results.

## New cold-export reuse

Completed contextual field resolution, source spelling and receiver proofs are
cached lazily. A single FieldUseCache is shared across all DEX resolvers during
one export. Key identity includes the original symbolic FieldKey, original
caller, optional receiver descriptor and opcode staticness. Frozen input,
metadata-provider and aliases belong to that cache instance.

Only completed top-level proofs are retained; recursive path-local cycle
checks and mutable declaration/initialization/publication closure remain outside.
Each stage uses OnceLock so concurrent consumers compute a stage once. Typed
failures remain typed failures. Resolution failure still invokes the legacy
receiver fallback; later receiver/source failures still reject retention.

The existing cache_resolutions option enables this by default. When disabled,
production uses the original uncached path, without extra cache-wrapper lookups.
JavaProfile.field_use_cache_* and java_profile's field_use_cache line expose
retained full keys and aggregate stage hits, computations and failures. Stage
computations are not distinct key counts. A controlled full cold A/B speedup of
this new field cache has NOT been established.

## Existing reusable source session

JavaSession keeps parsed input and its loaded classpath snapshot. The first emit
runs the decompiler; later emit calls replay exact sources/report into a fresh
sink. sources() gives borrowed access without copying. clear_cache() releases
rendered output. A new APK, options or metadata snapshot needs a new session.

A separately restarted one-shot CLI cannot access another process's RAM cache.
The core library's same live session is the reuse boundary; no disk cache or
cross-version reuse is introduced.

## Validation and reproduction

- New field-proof module:7 focused tests passed, including real javac metadata,
  cache ON/OFF proof equality, protected/private/static/hiding negatives,
  concurrent positive/negative initialization, full key/snapshot isolation,
  multi-DEX shared operand reuse, lazy external stages and panic retry.
- Production facade: cache ON/OFF,1/30 workers, selection limits and exact fixture
  report/source equality passed.
- Current session and strict workspace Clippy receipts are recorded in
  samsung-pass-ram-cache-current-validation.json.
- 409 frozen Rust/Cargo source files, executable, APK and classpath hashes unchanged
  throughout the full process; exit 0.

Evidence root: `/tmp/opencode/samsung-pass-ram-cache-20261003-8lvz4t_x`. Source snapshots and full run/stderr/resource logs are
retained there. Reproduce with the exact argv recorded in
samsung-pass-ram-cache-current-environment.json; verify file/report equality with
bench/verify_java_session_replay.py.

## Primary-source basis

[jadx ClassNode](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/core/dex/nodes/ClassNode.java)
checks the code cache before class generation, and
[InMemoryCodeCache](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/api/impl/InMemoryCodeCache.java)
stores generated code in a concurrent map. Its
[JadxDecompiler](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/api/JadxDecompiler.java)
also schedules source-save tasks in parallel. These mechanisms do not establish
relative complete-APK throughput without a matched workload.

[Ghidra DecompInterface](https://ghidra.re/ghidra_docs/api/ghidra/app/decompiler/DecompInterface.html)
retains initialization in a persistent process and caches function/symbol data,
with explicit flush guidance for changed metadata.
[ILSpy MetadataModule](https://github.com/icsharpcode/ILSpy/blob/master/ICSharpCode.Decompiler/TypeSystem/MetadataModule.cs)
lazily retains known attribute types and instances. Decomphify adopts immutable
reuse boundaries rather than relying on untracked mutable metadata.
