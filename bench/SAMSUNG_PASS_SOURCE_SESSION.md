# Reusable Java source sessions

`decomp::java_session::JavaSession` owns parsed DEX inputs, fixed `JavaOptions`,
and a classpath snapshot loaded at construction. A successful first render
retains the exact `GeneratedSource` objects and `JavaReport`. Subsequent `emit`
calls replay that snapshot in original order; they do not run decoding, SSA,
source throws, CFG structuring or initialization planning again.

This is an **in-process repeated-export cache**, not a faster cold decompiler,
not a disk cache, and not reuse across changed APKs or different options.
Source completion/fallback status, original provenance, diagnostics and unresolved
method/initialization counters remain unchanged. RAM is allocated for actual
results rather than reserved to fill the host's 128GB capacity.

```rust,ignore
let files = dex_format::apk::parse_apk_path(apk)?;
let mut session = decomp::java_session::JavaSession::new(files, options)?;
let first_report = session.emit(&mut first_sink)?; // decompile + deliver
let same_report = session.emit(&mut second_sink)?; // source replay + deliver
assert_eq!(first_report, same_report);
```

Use a new directory sink root per delivery. `sources()` allows read-only access
without copying text. `stats()` reports completed renders, replays, retained units
and exact UTF-8 text bytes; this byte count excludes metadata, DEX and allocator
storage, and is not RSS. `clear_cache()` releases generated sources/report while
retaining the DEX/classpath snapshot. A new session is required to reload changed
classpath files or change input/options.

Failed decompiler rendering never installs a partial snapshot. A sink failure
returns an error and retains the valid render for retry, but cannot roll back
already accepted output effects. Retries start at the first source again.

## Validation

Six release-mode tests passed, including:

- Exact source objects, order, reports and cache lifetime across repeated output.
- Failed sink retry; failed rendering never cached.
- Successful empty selections and independent sessions.
- A body calling a genuine external class-file declaration after removal of its
  original classpath file, including fresh rendering after cache clear.
- Actual Samsung Pass Fragment static initialization and exact warm replay.

The Fragment regression uncovered an integration error from the separately
implemented attested static-initializer path: a successful original-SSA proof
was replaced by an earlier, failed encoded-value preflight. Production now renders
**the actual successful plan** through frozen field aliases and retains the same
publication-closure checks. It does not introduce missing field assignments or
claim global initialization completeness. Global field-publication evidence can
remain conservatively unavailable where the earlier preflight still rejects.

Test log: `samsung-pass-session-tests-release.txt`.
Independent read-only review found no concrete session correctness defect.

## Primary-source basis

[jadx ClassNode](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/core/dex/nodes/ClassNode.java)
checks the code cache before generation and stores successful class output.
[jadx InMemoryCodeCache](https://github.com/skylot/jadx/blob/master/jadx-core/src/main/java/jadx/api/impl/InMemoryCodeCache.java)
retains code in a concurrent map. This supports the repeated-result reuse design;
it does not prove a relative cold whole-APK speed advantage.

[Ghidra DecompInterface](https://ghidra.re/ghidra_docs/api/ghidra/app/decompiler/DecompInterface.html)
retains process/initialization state across function requests and explicitly
documents clearing stale function/symbol information. Decomphify instead makes
session inputs and metadata immutable, avoiding stale external-file reuse.

## Samsung Pass cold/warm measurement

One process, one fixed session, 30 workers, genuine JDK17/Android35 classpath,
and real file writes in both phases. APK parsing and classpath loading took
1.433 s, outside the export timings below.

| Measurement | First render + write | Cached source replay + write |
|---|---:|---:|
| Export wall time | 189.117 s | 1.436 s |
| Written Java files | 30,543 | 30,543 |
| Exact source text bytes | 278,620,077 | 278,620,077 |
| Report, original paths and source SHA-256 | preserved | exact match |

Replay took 99.24% less elapsed time (131.66x throughput). This is a single
cold/warm pair, not a statistically repeated benchmark or a faster first render.
Retained source text was about 265.71 MiB. Whole-process peak RSS was about
9.48 GiB; this includes cold frontend analysis and is not the source-cache size.

Every one of the 30,543 files was independently SHA-256 compared: **zero changed,
added or missing files**. Reports also matched exactly. The session recorded
one successful decompiler render and one successful warm replay.

Outputs remain source subsets: 23,792 unsupported non-constructor methods and
9,137 unavailable constructors are still reported. Caching does not change their
status or establish whole-app compilation or runtime equivalence.

Artifacts:

- `samsung-pass-session-environment.json`: APK/binary hashes and fixed classpath.
- `samsung-pass-session-verified-{run,time,stderr}.txt`: successful run and RSS.
- `samsung-pass-session-comparison.json`: exact file/report preservation.
- `/tmp/opencode/samsung-pass-session-replay-verified/{cold,warm}/sources`.

The earlier attempts remain recorded: the first harness attempt lacked parent
output directories; the second hit the attested-prefix integration error fixed
above. Neither installed a successful session snapshot or is used in the timing
comparison.

Final scoped validation: strict Clippy for the decomp library, session benchmark
and session tests passed; full workspace formatting and diff-whitespace checks
passed. The initializer and field-publication suites contributed another
30 passing tests (one pre-existing corpus probe remained ignored). The separately
staged dispatch adapter is explicitly expected to be unused in non-test builds;
its production selection is not claimed integrated by this cache work.
