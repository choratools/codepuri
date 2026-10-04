# SmartThings measured bottlenecks — 2026-10-01

## Scope and implementation

One full APK export, class workers 30, unchanged fixed-batch scheduling and
conservative Java behavior. Added caller-owned `emit_java_profiled`, class and
batch records, existing `SsaTimings` aggregation, diagnostic example and analysis
script. No scheduling or type-context optimization was implemented.

Full command (run in persistent tmux session `decomphify-profile30`, waited to exit):

```sh
/usr/bin/time -v -o /tmp/opencode/decomphify-profile30-time.txt \
  /storage_0/decomphify/target/release/examples/java_profile \
  /storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.oneconnect.apk \
  /tmp/opencode/decomphify-profile30 --workers 30 \
  > /tmp/opencode/decomphify-profile30-run.txt 2>&1
```

Paseo terminal creation returned “OpenCode session is not bound to a Paseo
agent”; tmux provided the persistent fallback. Exit status 0. No full 101-app
run, installation, commit, or optimization benchmark was performed.

## Full-run measurements

- Export wall: **249.872 s**; parse: **0.997 s**; process wall: **252.26 s**.
- Process user/system: **1196.07 / 40.14 s**, CPU **490%**, peak RSS
  **2,262,172 KiB**. Previous artifact: 244.715 s app / 245.08 s process,
  CPU 524%. This is not a controlled overhead comparison: cache, machine
  activity, runner checksumming and diagnostic serialization differ. A short
  release build/test overlapped the full run; no other benchmark ran concurrently.
- 117,272 class tasks in **3,914 batches**; 38,925 generated files,
  96,004,104 source bytes. All previous headline output counts match.

| Caller-thread stage | Seconds | % export wall |
|---|---:|---:|
| Alias inventory | 2.277 | 0.911% |
| Alias build | 3.951 | 1.581% |
| Immutable Java resolver preparation | 0.314 | 0.126% |
| Ordered sink callback / DirectorySink IO | 34.229 | 13.699% |
| Batch spawn through final join | 207.887 | 83.197% |
| Remaining caller overhead | 1.214 | 0.486% |

Summed class-task elapsed time is **1195.315 s**, overlapping across workers.
These percentages have a different denominator from the table above:

| Task stage | Aggregate seconds | % summed task elapsed |
|---|---:|---:|
| Frontend, including per-method type-context creation | 1142.036 | 95.543% |
| Structure/backend, bindings and requirements | 38.532 | 3.224% |
| Class source serialization | 1.313 | 0.110% |
| Closure pruning | 0.161 | 0.013% |
| Buffered source collection | 0.059 | 0.005% |
| Unattributed task work | 13.213 | 1.105% |

Nested SSA totals: analysis 5.647 s, placement 10.446 s, liveness 8.308 s,
phi placement 2.589 s, rename 4.827 s, phi cleanup 2.160 s, materialization
8.699 s, inference 14.360 s, verification 23.443 s. Together **80.478 s**, or
**7.047% of frontend**. Verification alone is 2.053% and inference 1.257% of
frontend. The remaining **1061.558 s (92.953%)** is outside completed SSA
substages: context preparation, decode/lift/merge, cleanup and failing SSA stages.
The full run preceded the finer context/decode/lift/merge instrumentation;
do not interpret the selected-class breakdown below as a full-app percentage.

## Proven barrier imbalance

- Average active class tasks during batch intervals: **5.750 of 30**,
  **19.166% occupancy**. This is elapsed task occupancy, not CPU utilization.
- Available capacity: **6236.596 worker-slot seconds**. Partition:
  active tasks 1195.315; startup idle 94.791; finished-task barrier idle
  **4923.015 (78.938%)**; unused final-batch slots 23.475.
- Tasks waiting after completion: **111,763 >1 ms**, **92,522 >10 ms**,
  **7,636 >100 ms**. Each count is a task/barrier instance, not a persistent
  worker-thread count: the current implementation spawns a new thread per class.
- Sum of last-finishing task elapsed durations: **203.392 s**, close to the
  207.887 s batch wall. Longest-batch evidence is in `batches.tsv` and analysis JSON.

| Longest class | DEX ordinal | Task elapsed | Main measured group |
|---|---:|---:|---|
| `chip/devicecontroller/ClusterReadMapping` | 3 | 8.019 s | frontend 7.760 s; 5736 methods |
| `org/bouncycastle/pqc/crypto/hqc/ReedSolomon` | 2 | 2.472 s | structure/backend 2.389 s; 8 methods |
| `…/webplugin/jsinterface/WebPluginJSInterface` | 9 | 2.281 s | frontend 2.237 s; 1284 methods |
| `chip/devicecontroller/ClusterInfoMapping` | 3 | 2.143 s | frontend 2.021 s; 1451 methods |
| `wx1/d` | 2 | 1.800 s | structure/backend 1.732 s; 1 method |

The ClusterReadMapping batch lasted **8.0226 s**. All **29 other tasks finished
before half its wall time**; mean active tasks 1.480, mean finished-idle slots
28.513. Its summed task time was 11.870 s and post-finish idle 228.751 slot-s.
ReedSolomon's batch likewise had 29 other tasks finished by halfway through
2.5115 s, with only 1.059 mean active tasks.

## Selected-class controlled probes

Same original APK and complete metadata pools; no class-data mutation, no limit.
Selected `Lorg/bouncycastle/pqc/crypto/hqc/ReedSolomon;` (8 methods). Runs were
sequential in tmux session `decomphify-selected` and completed successfully.

| Output artifact directory under `/tmp/opencode/` | Workers | Sink | Profiling | Export wall | Selected class elapsed | Structure/backend | Sink callback |
|---|---:|---|---|---:|---:|---:|---:|
| `decomphify-selected-noio1` | 1 | checksum only | on | 17.684 s | 2.524 s | 2.422 s | 0.018 ms |
| `decomphify-selected-noio30` | 30 | checksum only | on | 13.459 s | 2.437 s | 2.382 s | 0.031 ms |
| `decomphify-selected-dir30` | 30 | DirectorySink | on | 13.830 s | 2.458 s | 2.400 s | 0.638 ms |
| `decomphify-selected-unprofiled30` | 30 | checksum only | off | 13.588 s | unavailable | unavailable | unavailable |

All four complete reports and ordered path/source checksums match exactly:
**`0bb1be23ae7c4a69`**, one 8,739-byte source. The class itself gains little
from 30 workers; the wall difference includes creating threads for 117,271
filtered classes, alias preparation and traversal. The 0.371 s wall difference
between sink modes is **not** an IO-cost estimate; the measured callback
difference is approximately 0.607 ms. Profiling-on wall was 0.129 s below off
in this one sample, demonstrating run variation rather than negative overhead.

Additional selected longest-class probe: `decomphify-selected-cluster30`,
`Lchip/devicecontroller/ClusterReadMapping;`, workers 30, checksum-only sink.
Export wall 17.924 s, selected task **6.769 s**, frontend **6.516 s**:

- **`DexTypes::new` preparation: 6.101 s, 93.636% of frontend**.
- Decode 0.008 s, lift 0.057 s, merge 0.090 s.
- Verification 0.079 s, inference 0.042 s; other SSA stages 0.127 s combined.
- Structure/backend 0.185 s; class serialization 0.009 s.

Code evidence: `java_integration::method_source` creates a fresh `DexTypes`
for every body attempt; `infer/context.rs::DexTypes::new` loops over all DEX
type IDs and rebuilds `by_descriptor`. This is separate from `Resolver::new`,
which is already shared per DEX and measured at only 0.314 s over the full app.

Selected command pattern:

```sh
target/release/examples/java_profile \
  tests/corpus/apks_bulk/com.samsung.android.oneconnect.apk \
  /tmp/opencode/decomphify-selected-noio30 --workers 30 \
  --class 'Lorg/bouncycastle/pqc/crypto/hqc/ReedSolomon;' --no-io
```

## Output parity and limitations

Full generated path sets and every source byte **excluding unavailable-member
diagnostic comment lines** match the prior full run. Exact whole-file parity
does **not** hold: **2831 files** differ only in those comment lines; **1864**
of those differ solely in `EdgeId` numbers. No non-comment source differences.

Two additional **unprofiled** runs of `Landroid/support/customtabs/a;` reproduce
the variation: same source count, byte length and headline report counts, but
checksums `ac46e3fa0c4b6b9b` and `2b19b5f800cbc9c5`, with different `EdgeId`
reason distributions. Thus exact diagnostic-comment stability is already absent
without enabling profiling. The complete reason map is not claimed identical
to the old full benchmark. No diagnostic-order behavior was changed here.

No full-app workers=1 comparison or full-app profiling-overhead A/B was run.
The previously cited 938 s is historical, not a newly reproduced measurement.
Measurements are elapsed clocks rather than sampled CPU stacks; no perf run
was needed. Selected-class measurements do not estimate full-app context cost
exactly. Synchronous sink timing excludes deferred OS writeback. Structure and
backend are grouped, so their individual contributions remain unresolved.

## Ranked next fixes and Amdahl interpretation

1. **Share the immutable DEX descriptor index across method contexts.** Keep
   per-method enclosing-return state, type caches and unresolved diagnostics
   isolated. The selected longest class proves repeated index construction is
   expensive; this is narrower and better evidenced than more SSA optimization.
   Require source/report parity on deterministic fixtures and a selected-class
   before/after benchmark before extending to a full app.
2. **Replace fixed barriers with a bounded persistent class queue and ordered
   results.** Preserve input-order sink/error/limit semantics and bounded memory.
   The 78.938% finished-idle slot fraction proves imbalance, but is not itself
   a promised speedup. First measure after index sharing: faster classes will
   change the distribution. Persistent workers also address the measured
   thread-startup costs and filtered-class traversal overhead.
3. **Investigate DirectorySink operations/checksum cost.** The measured serial
   callback is 13.699% of export wall. Eliminating it entirely would give only
   **1.159x** at otherwise fixed timing; it cannot explain the frontend gap.
4. **Split structure/backend timing for large straight-line methods.** The
   ReedSolomon 2.38–2.42 s selected group is reproducible. Structuring's
   quadratic postdominator storage is a code-inspection candidate, not yet
   measured as the individual root cause. Avoid optimizing it from inspection
   alone.

The measured explicit serial inventory/build/resolver/sink total is **40.772 s
(16.317%)**. Increasing worker count cannot remove that floor. With elapsed
task durations frozen from this run, summing per-DEX
`max(task_sum/30, longest_class)` gives an optimistic compute lower bound
**45.117 s**. Including serial work and the current 1.214 s caller remainder
yields approximately **87.10 s / 2.87x** as an optimistic scheduler-only bound,
**not a measured prediction**: contention, allocation, memory bandwidth,
ordering/backpressure and startup costs can change. Index reuse could reduce
the underlying work rather than just rearrange it. The current low average
CPU is consistent with directly measured barrier idle; hardware scaling cannot
be inferred from CPU% alone.

## Artifacts and validation

- Full: `/tmp/opencode/decomphify-profile30/{summary.txt,classes.tsv,batches.tsv,sources/}`.
- Process logs: `/tmp/opencode/decomphify-profile30-{run,time}.txt`.
- Analysis and full source comparison: `/tmp/opencode/decomphify-profile30-analysis.json`.
- Selected directories: the five `decomphify-selected-*` directories listed above;
  each has `summary.txt`, `classes.tsv`, `batches.tsv`, plus sources for dir30.
  Exact stdout logs: `/tmp/opencode/decomphify-selected-{noio1,noio30,dir30,unprofiled30,cluster30}.log`.
- Unprofiled variation: `/tmp/opencode/decomphify-parity-unprofiled-{a,b}/summary.txt`,
  corresponding `.log` files, and `/tmp/opencode/decomphify-unprofiled-nondeterminism.json`.
- Baseline: `/tmp/opencode/decomphify-java-corpus/run-1790816220880748278-1817843/com.samsung.android.oneconnect/sources/`.
- `cargo test -p decomp --release --test java_integration`: **5 passed**, including
  profiling on/off, workers 1/30, ordered bytes/reports, limits and sink-error
  timing retention. Batch slot partition is asserted exactly in nanoseconds.
- `cargo clippy -p decomp --release --example java_profile --test java_integration -- -D warnings`: **passed**.
