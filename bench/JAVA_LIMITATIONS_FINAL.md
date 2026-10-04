# Current-tree final OKcashbag Java release measurement

This is a historical snapshot. The latest integration status and fresh final
measurement are in [`JAVA_PRODUCTION_INTEGRATION.md`](JAVA_PRODUCTION_INTEGRATION.md).

This fresh full-APK measurement includes the late production corrections documented
in `JAVA_LIMITATIONS_EXPANDED.md` and supersedes its pre-correction APK counters.
No production code was modified and no commit was made. Previously passing whole
workspace tests were not repeated.

## Build and completed run

```sh
cargo build --release -p decomp --example java_corpus
/usr/bin/time -v -o /tmp/opencode/decomphify-java-final-time.txt \
  target/release/examples/java_corpus --workers 30 \
  --app tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk \
  > /tmp/opencode/decomphify-java-final-stdout.txt \
  2> /tmp/opencode/decomphify-java-final-stderr.txt
```

Release build succeeded (`Finished release profile [optimized] ... in 0.05s`).
One app worker and **30 internal class workers**; no class filter or limit.
The command was awaited through completion. Exit status **0**; stderr empty;
report available **true**; errors **0**. Artifact summary confirms selected=1,
completed=1, reports_available=1. Neither report nor summary contains an error row.

Fresh root: `/tmp/opencode/decomphify-java-corpus/run-1790832652145312482-472667`.

Exact reports (including all rejection reasons):

* `/tmp/opencode/decomphify-java-corpus/run-1790832652145312482-472667/summary.txt`
* `/tmp/opencode/decomphify-java-corpus/run-1790832652145312482-472667/com.skmc.okcashbag.home_google/report.txt`
* Sources: same per-app directory, `sources/`.
* Raw stdout, stderr and time logs: the three `/tmp/opencode/decomphify-java-final-*.txt` files above.

## Exact stdout

```text
START apps=1 workers=30 artifacts=/tmp/opencode/decomphify-java-corpus/run-1790832652145312482-472667
PARALLELISM app_workers=1 class_workers=30
DONE com.skmc.okcashbag.home_google seconds=31.745 report=true errors=0
app=TOTAL seconds=31.817 report_available=true classes_seen=96744 classes_generated=38547 complete_method_bodies=64058 unsupported_methods=396634 unavailable_constructors=75486 skipped_classes=0 classes_written=38547 complete_constructors=17313 complete_static_initializers=2017 declaration_only_methods=33173 fields_emitted=29505
COMPLETE artifacts=/tmp/opencode/decomphify-java-corpus/run-1790832652145312482-472667
```

## Counter comparison

Baseline and previous snapshot values are from `JAVA_LIMITATIONS_EXPANDED.md`.

| Counter | Baseline | Previous snapshot | Final current tree | Delta vs previous | Delta vs baseline |
|---|---:|---:|---:|---:|---:|
| Generated/written files | 36,407 | 38,191 | 38,547 | +356 | +2,140 |
| Ordinary complete method bodies | 50,912 | 61,921 | 64,058 | +2,137 | +13,146 |
| Complete constructors | 17,023 | 17,211 | 17,313 | +102 | +290 |
| Complete static initializers | 1,959 | 2,017 | 2,017 | 0 | +58 |
| Unsupported methods | 409,838 | 398,771 | 396,634 | -2,137 | -13,204 |
| Unavailable constructors | 75,776 | 75,588 | 75,486 | -102 | -290 |
| Declaration-only methods | 33,173 | 33,173 | 33,173 | 0 | 0 |
| Fields emitted | 26,827 | 28,760 | 29,505 | +745 | +2,678 |
| Per-app export seconds | 23.19 | 30.881 | 31.745 | +0.864 | +8.555 |
| Peak RSS KiB | 1,508,248 | 1,760,152 | 1,764,028 | +3,876 | +255,780 |

Classes seen **96,744**, skipped classes **0**. Generated and written counts both
**38,547**; independent `rg --files -g '*.java' .../sources | wc -l` inventory
also returned **38547**. Complete ordinary plus unsupported methods total
**460,692**, unchanged from the prior snapshot. Complete plus unavailable
constructors total **92,799**, likewise unchanged.

Timing: runner TOTAL **31.817 s**; `/usr/bin/time -v` wall **0:32.12**;
user CPU **204.40 s**; system CPU **6.14 s**; CPU **655%**;
maximum RSS **1,764,028 KiB**; swaps **0**; exit status **0**.
These are single-run observations, not a repeatable performance attribution.

## Four unchanged real output units: compile and execute

The following files were compiled directly from the fresh artifact, with no edits,
alongside the existing `bench/JavaArrayObjectCastSmoke.java` harness:

1. `androidx/constraintlayout/core/motion/utils/TypedBundle.java`
2. `androidx/compose/runtime/IntStack.java`
3. `androidx/camera/core/FocusMeteringResult.java`
4. `androidx/compose/runtime/external/kotlinx/collections/immutable/internal/DeltaCounter.java`

Reproduction (run from repository root):

```sh
export PATH=/home/jtjisgod/tools/jdk-17.0.13+11/bin:$PATH
S=/tmp/opencode/decomphify-java-corpus/run-1790832652145312482-472667/com.skmc.okcashbag.home_google/sources
javac -d /tmp/opencode/decomphify-java-final-realunits \
  "$S/androidx/constraintlayout/core/motion/utils/TypedBundle.java" \
  "$S/androidx/compose/runtime/IntStack.java" \
  "$S/androidx/camera/core/FocusMeteringResult.java" \
  "$S/androidx/compose/runtime/external/kotlinx/collections/immutable/internal/DeltaCounter.java" \
  bench/JavaArrayObjectCastSmoke.java
java -cp /tmp/opencode/decomphify-java-final-realunits \
  androidx.constraintlayout.core.motion.utils.JavaArrayObjectCastSmoke
```

Both commands succeeded. Actual runtime stdout:

```text
PASS unchanged OKcashbag array/object source units
```

The existing assertions preserve their meaning: array allocation/defaults and
clear, array loads/bounds exceptions, boolean constructor arguments, and object
identity/state through constructor/copy operations.

## Limits

Errors=0 means traversal/export succeeded; it does not turn **396,634 unsupported
methods** or **75,486 unavailable constructors** into implemented bodies.
The **33,173 declaration-only methods** are not ordinary complete method bodies.
Counts describe generated bodies, not complete classes or a runnable APK.
Only the four representative unchanged output units above received javac/runtime
validation; whole-APK compilation/execution remains unverified.

Remaining support boundaries retain the meaning documented in
`JAVA_LIMITATIONS_EXPANDED.md`: arbitrary uninitialized reference graphs,
effectful/exceptional allocation gaps, Unknown/conflicting local types, exact-type
Phi joins, missing/cyclic/conflicting ancestry, external metadata and actual
classpath dependencies, external field closure and protected access. Exact
rejection strings and counts are preserved in the fresh report artifact.
