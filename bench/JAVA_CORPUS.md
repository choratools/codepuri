# Full Java first-slice corpus export

```sh
cargo test --workspace
cargo clippy -p decomp --example java_corpus -- -D warnings
cargo build --release -p decomp --example java_corpus
/usr/bin/time -v target/release/examples/java_corpus --workers 8 tests/corpus/apks_device tests/corpus/apks_bulk
```

Worker controls now use `--workers N` as a total budget, with
`--app-workers A` (default 1) concurrent APKs and `floor(N/A)` internal
class workers per APK. Use `--workers 8 --app-workers 8` to retain the
APK-only scheduling of the historical command above. See
`smartthings30-workers.md` for class scheduling details.

The runner processes APKs with bounded workers and streams classes
through the existing `dex_java::emit_java` and `DirectorySink` APIs. Each run
creates a fresh `/tmp/opencode/decomphify-java-corpus/run-<timestamp>-<pid>`;
each APK stem has a `sources/` directory and `report.txt`. Duplicate stems are
rejected before export. Inputs stay local and unchanged.

The final name-sorted report is saved to `bench/java_corpus_first_slice.txt`
and the run directory's `summary.txt`. Per-app elapsed times and exact facade
reason strings are included. SSA failures remain method rejection reasons.
Filesystem failures are recorded and traversal continues: `classes_generated`
counts facade-generated units, while `classes_written` counts accepted writes.
Complete method counts describe generated bodies, including bodies in units
whose writes failed. Any facade-level failure or panic explicitly marks its
report unavailable; its missing method counts must not be interpreted as zero
coverage. A completed traversal can contain reported failures.

Scope is the current conservative linear-flow Java subset, with validation
gates retained. Original constructors, fields, inheritance, unsupported methods
and other rejected constructs are unavailable. There is no full Java app,
decryption/protection bypass, or corpus Java compilation verification claim.

Validation/run logs are under `/tmp/opencode/decomphify-java-*`. The initial
200-second timed invocation was interrupted; the authoritative full run uses
the `*-persistent.log` logs and its distinct artifact directory.

## Completed first run

All **101/101** APKs completed with available reports, using `--workers 8`.
Authoritative artifacts:
`/tmp/opencode/decomphify-java-corpus/run-1790780430256226401-2367066`.
Independent artifact inventory confirms 101 per-app reports and 21,215 Java files.

| Metric | Total |
| --- | ---: |
| Classes examined | 2,830,663 |
| Subset classes generated | 21,282 |
| Subset classes written | 21,215 |
| Complete ordinary method bodies generated | 39,822 |
| Unsupported non-constructor methods | 13,846,634 |
| Unavailable original constructors | 2,645,957 |
| Skipped classes | 0 |
| Output path collisions | 67 |
| Frontend/SSA rejection reasons | 0 |
| App parse/facade failures or panics | 0 |

Grouped method rejections (exact detailed reasons remain in the report):
member dependencies 10,886,341; interface/enum/annotation 1,079,253;
class initializer 659,649; missing/ambiguous initial register 329,573;
method flags 296,677; missing incoming register 248,253;
method name 192,365; unresolved signature 72,340; backend 63,489;
structure 18,694. These sum to exactly 13,846,634.

`/usr/bin/time -v` recorded wall time **56:58.53**, maximum RSS
**4,760,220 KiB** (approximately 4.54 GiB), and exit status **0**.
Workspace tests passed (an existing unused-variable warning in `scratch_dump`
was present), targeted Clippy with `-D warnings` passed, and release build
passed. `git diff --check` passed. `javac` remains unavailable.
