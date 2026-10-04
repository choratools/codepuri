# Current in-memory Java source replay validation

The default exporter reuses SSA, method/field resolutions, capability evidence,
object plan shapes and completed hierarchy assignment checks. `JavaSession`
additionally retains exact generated sources and the report for repeated exports
of its immutable APK, classpath and options. This session-level cache is the
fast path measured here. It survives repeated library calls, not process exits.

## Measured result

One frozen release executable, Samsung Pass 5.2.10.10, 30 workers, Java source
release 25, and four explicit genuine JDK17/Android35 classpath archives. Both
phases wrote all source files into distinct directories.

| Metric | First render and write | Cached replay and write |
|---|---:|---:|
| Export wall time | 297.882489 s | 1.584110 s |
| Java files | 31,100 | 31,100 |
| UTF-8 source text bytes | 300,481,064 | 300,481,064 |
| Complete ordinary methods | 137,909 | 137,909 |
| Complete constructors | 31,111 | 31,111 |
| Complete static initializers | 5,112 | 5,112 |
| Unsupported non-constructor methods | 17,094 | 17,094 |
| Unavailable constructors | 3,094 | 3,094 |
| Genuine bodyless declarations | 11,177 | 11,177 |

Setup (APK parsing and classpath loading) took 1.354804 s outside those timings.
The session performed exactly one render and one warm replay. An independent
SHA-256 comparison found no changed, added or missing files. Exact reports and
ordered checksums also match. This is a single cold/warm pair, approximately
188x replay throughput, not a controlled cold-decompiler acceleration claim.
Workspace tests ran concurrently: absolute cold time depends on that load.

Retained source text is 286.56 MiB, excluding metadata and allocator overhead.
Whole-process peak RSS was 12,896,428 KiB (12.30 GiB), including cold analysis.
The implementation allocates actual cache entries; it does not reserve the
host's entire 128GB. The host reported 125 GiB physical and 58 GiB available
before this run.

## API and cache identity

```rust,ignore
let files = dex_format::apk::parse_apk_path(apk)?;
let mut session = decomp::java_session::JavaSession::new(files, options)?;
let original = session.emit(&mut first_sink)?;
let replay = session.emit(&mut second_sink)?;
assert_eq!(original, replay);
```

Use a fresh sink directory per export. `sources()` accesses cached objects
without copying their text. `clear_cache()` drops sources/report. New APKs,
classpath or option changes require a new session. Rendering failures never
install partial snapshots; sink failure retains the valid snapshot for retry.
Unavailable bodies and initialization remain unavailable after caching.

The design follows existing mechanisms: [jadx ClassNode](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/core/dex/nodes/ClassNode.java)
looks up generated class code before generation, backed by
[InMemoryCodeCache](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/api/impl/InMemoryCodeCache.java).
[Ghidra DecompInterface](https://ghidra.re/ghidra_docs/api/ghidra/app/decompiler/DecompInterface.html)
retains initialization state across function requests and documents clearing
stale symbol/function data. These sources explain mechanisms, not same-workload
comparative speed.

## Evidence

Current full workspace verification finished successfully: **784 passed, zero
failed, 60 ignored**. Workspace/all-target strict Clippy, full formatting and
diff whitespace checks passed. All frozen crate source hashes remained unchanged
through benchmark completion; only evidence scripts and documentation were added
after the freeze.

- `samsung-pass-cache-current-comparison.json`: all-file verification and timings.
- `samsung-pass-cache-current-environment.json`: frozen binary hash, source hashes and command.
- `samsung-pass-cache-current-{run,time,stderr}.txt`: terminal run and resource evidence.
- `samsung-pass-cache-current-{workspace-tests,clippy,fmt}.txt`: current validation logs.
- `verify_java_session_replay.py`: reusable independent replay verifier.
- Output: `/tmp/opencode/samsung-pass-cache-current-1790980102076446329/replay/{cold,warm}/sources`.

No cold-cache-disabled A/B measurement was performed in this wave. Existing
reports retain the separate export-scoped cache experiments and limitations.
Exact cache parity does not establish whole-source javac success, Android
runtime equivalence, native recovery or achievement of the active 99% goal.
