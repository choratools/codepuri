# Opt-in Java export profiling

Measured SmartThings results and ranked next fixes: [SMARTTHINGS_PROFILE.md](SMARTTHINGS_PROFILE.md).

`dex_java::emit_java_profiled(files, options, sink, &mut JavaProfile)` runs the
same export pipeline as `emit_java`, with caller-owned measurements and no
library stdout/stderr. Ordinary export does not read the profiling clock or
retain profiling records. Options, source ordering, rejection rules, resolver
sharing, closure and sink error behavior are unchanged. No scheduler change is
included.

Build and run one APK (output directory must not exist):

```sh
cargo build -p decomp --release --example java_profile
/usr/bin/time -v -o /tmp/opencode/decomphify-profile30-time.txt \
  target/release/examples/java_profile \
  tests/corpus/apks_bulk/com.samsung.android.oneconnect.apk \
  /tmp/opencode/decomphify-profile30 --workers 30
```

The diagnostic example writes `summary.txt`, `classes.tsv`, `batches.tsv` and,
unless `--no-io` is used, sources through `DirectorySink`. It records APK parse
time separately. `--class 'Lsome/ExactClass;'` selects using the existing
substring filter, retaining the same complete input pools and resolver setup.
Use separate output directories with `--workers 1`, `--workers 30`, `--no-io`
and `--unprofiled` to compare identical selections. The ordered FNV-1a checksum
includes path and source bytes with separators; compare reports and checksums.
It is a diagnostic checksum, not a cryptographic integrity guarantee.

## Interpreting the measurements

- `serial`: alias inventory, alias build, immutable resolver preparation,
  ordered sink callback. Percentages use export **wall time**.
- `task_stage`: frontend (recovering decode, lift, merge, SSA, infer, verify),
  structure/backend (including binding and member requirement checks), closure
  pruning, class source serialization, and buffered source collection.
  Percentages use **summed class-task elapsed time**, not export wall time.
- `nested_ssa`: type context preparation, decode/lift/merge and existing
  `SsaTimings`, summed across frontend attempts;
  percentages use aggregate frontend time. These are contained in frontend,
  so adding them again would double-count. On SSA errors, completed substage
  timings survive; time in an unfinished failing SSA substage is only in frontend.
- Task unattributed time includes metadata/flag rejection, header/field work,
  report construction, destruction and instrumentation overhead. Declaration-only
  methods do not enter frontend. Backend source generation is in backend;
  class-wrapper serialization is separately measured.
- `batch_wall_sum_s`: spawn-to-final-join time, excluding ordered sink delivery.
  `task_elapsed_sum_s / batch_wall_sum_s` is average active task occupancy,
  **not average CPU cores**. Scheduling, allocations and memory stalls can be
  inside active tasks. Process CPU utilization comes from `/usr/bin/time`.
- `capacity_slot_s = sum(batch_wall * configured_workers)`. The partition is
  task elapsed + startup idle + finished idle + empty final-batch slots.
  Finished idle is measured as `batch_end - task_finish`, summed per task;
  it proves barrier waiting, but does not identify how much a replacement
  scheduler could recover under contention. Filtered tasks are included.
- Classes and batches are retained in full, and the summary lists the longest
  20 of each. Profiling uses per-task records merged after joins without shared
  timing locks. Memory and clock/map overhead are opt-in; small selected runs
  with `--unprofiled` bound their effect, not full-workload overhead.
- Sink time includes the example's byte checksum and callback overhead; with
  DirectorySink it also includes actual directory/file operations. It measures
  synchronous callback latency, not deferred OS writeback or fsync durability.

Limits retain existing speculative batch work. Profile records can include
classes computed but discarded after reaching the output limit. Returned
errors retain completed measurements; panics retain existing unwind behavior.

`python3 bench/java_profile_analysis.py PROFILE_DIR --baseline-sources SOURCES`
computes batch idle-task counts, longest-batch occupancy, a fixed-duration
optimistic compute bound, and exact file parity. It separately reports parity
excluding unavailable-member comment lines; this is **not** exact byte parity.
`--compare-runs DIR...` compares selected-run complete reports and checksums.
