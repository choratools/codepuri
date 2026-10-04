# Real Java while-loop recovery

Historical single-latch measurement. Current nested/multiple-latch production
status, switch/exception integration and final APK counters are documented in
[`JAVA_PRODUCTION_INTEGRATION.md`](JAVA_PRODUCTION_INTEGRATION.md).

## Implemented grammar and semantics

`crates/decomp-structure/src/loops.rs` recognizes disjoint reducible natural
loops using the existing core dominators/natural-loop analysis. Each supported
loop has one entry header, one unconditional latch/backedge, a linear header
evaluation prefix, a true/false condition, and one normal exit. The body may
contain nested acyclic `If` regions. Sequential loops and loops inside acyclic
conditional arms are supported.

`Region::While` retains the original header, evaluation blocks, conditional
block, body-edge polarity, body, and normal exit. Original edge provenance and
revision are retained. Every original block occurs once. The body view removes
the backedge; the enclosing view collapses the loop to a header-to-exit edge.
Both views pass Kahn cycle detection before DAG postdominators run. No DAG
algorithm is run directly on a cyclic graph. The acyclic structuring path is
unchanged.

`crates/backend-java/src/branching.rs` lowers the evaluation prefix inside
`Step::While`. The existing original-edge Phi batches provide preheader
initialization, body-entry copies, backedge updates and exit copies. All sources
are snapshotted before any destination assignment, preserving swaps. Body-only
definitions are rolled back from the availability environment after lowering;
zero iterations cannot make those definitions available at the exit. Header
definitions remain available because the final condition evaluation executes
even on a zero-iteration loop. Identity checks, exact Phi input coverage,
Boolean conditions and exact Phi source/result types remain enforced.

`crates/backend-java/src/lib.rs` emits a real `while (true)` with header evaluation
followed by a polarity-aware normal-exit guard and body. The guard's `break` is
the recovered loop's normal exit. This is not a dispatcher/state-machine
fallback. Ordinary instructions remain ordered assignments into typed hoisted
locals; initializers and effects are not hoisted.

The facade already dispatches through the basic structurer and backend planner;
no facade edit was necessary. Counter APIs, filtering, unsupported-member
reporting, type inference and the type verifier were retained.

### Explicitly deferred

Nested/overlapping natural loops, multiple latches/continue paths, body breaks
or early returns, irreducible/multiple-entry cycles, posttest loops, empty
self-edge bodies and branching header-evaluation prefixes remain unsupported.
They return typed structuring errors, not partial loop output. Switch/exception
loop bodies remain unsupported. The extended switch structuring API remains
acyclic. Existing Unknown/Null/uninitialized type and genuine dependency-closure
rejections still apply to otherwise recognizable loops.
The dedicated constructor/static-initializer lowering remains linear; loop
recovery in this change is for the ordinary structured method backend.

## Validation on the final code

```sh
env PATH=/home/jtjisgod/tools/jdk-17.0.13+11/bin:$PATH cargo test --workspace --quiet
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release -p decomp --example java_corpus
```

All passed: **318 tests passed, 0 failed, 6 existing tests ignored**. Backend
unit tests: 37; branch integration tests: 15; type compatibility tests: 4; new
while integration tests: 2. Facade unit tests: 43. No JDK execution checks were
skipped. The workspace gate was rerun after the last implementation/test edits.

`crates/backend-java/tests/while_loops.rs` compiles six emitted methods using
JDK 17 and executes **152 Java equality assertions** across inputs 0 through 9
and negative zero-iteration cases:

* sum and count loops;
* both condition polarities;
* simultaneous swap Phi backedge copies (odd and even iteration counts);
* condition effects exactly `n + 1` times and body effects exactly `n` times;
* effectful initialization exactly once, including zero iterations;
* nested if arms with exact selected-arm effect counts.

The other new integration test checks missing Phi inputs, unavailable initial
Phi sources, body-only values used after the loop or in the header condition,
and mismatched Phi types. Structurer tests check original-block/edge coverage,
polarity, break exits, multiple backedges, multiple entries, and posttest
rejection. Existing irreducible-cycle and acyclic differential/scaling tests
pass.

Final compiled fixture source:
`/tmp/decomphify-while-1889845/WhileMain.java`.

## APK evidence and filtered runs

Input: `tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk`.
Each invocation used the final release binary, `--workers 30 --app <input>`.
Artifact names below are relative to `/tmp/opencode/decomphify-java-corpus/`.
Every invocation completed with a report and **zero app/output errors**.

| Class filter | Limit | Artifact directory | Written | Ordinary bodies | Unsupported | Complete ctors | Unavailable ctors | App seconds | Wall seconds | Peak RSS KiB |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `kotlin/collections/ArraysKt___ArraysKt` | 1 | `run-1790846687205559706-1884702` | 0 | 0 | 0 | 0 | 0 | 19.747 | 19.94 | 1,584,640 |
| `kotlin/UByteArray` | 1 | `run-1790846779633479475-1901020` | 1 | 1 | 38 | 0 | 2 | 17.967 | 18.11 | 1,585,392 |
| `com/google/common/hash/LittleEndianByteArray` | 1 | `run-1790846822106994162-1905439` | 1 | 0 | 0 | 0 | 0 | 19.441 | 19.62 | 1,586,220 |
| `androidx/transition/FloatArrayEvaluator` | 1 | `run-1790846822108367121-1905440` | 1 | 0 | 2 | 1 | 0 | 19.480 | 19.69 | 1,584,932 |
| `com/google/common/hash/LittleEndianByteArray` | none | `run-1790846858519369636-1909748` | 2 | 0 | 25 | 1 | 7 | 18.763 | 18.91 | 1,585,544 |
| `com/google/common/primitives/Booleans` | none | `run-1790846858534186711-1909751` | 2 | 1 | 51 | 1 | 4 | 19.075 | 19.22 | 1,585,528 |
| `com/google/zxing/oned/rss/RSSUtils` | 1 | `run-1790846950367025625-1919957` | 1 | 1 | 1 | 1 | 0 | 18.874 | 19.02 | 1,585,152 |
| `androidx/constraintlayout/core/motion/utils/TypedBundle` | 1 | `run-1790846950368283747-1919959` | 1 | 2 | 7 | 1 | 0 | 18.563 | 18.71 | 1,585,092 |
| `com/nbt/oss/exp4j/function/Function` | 1 | `run-1790846992707230625-1924769` | 1 | 1 | 3 | 0 | 2 | 23.050 | 23.25 | 1,586,184 |

The first six filtered attempts preceded the full run. ArraysKt was a no-match
attempt. The limited LittleEndian filter selected an interface, not the intended
outer loop class. FloatArrayEvaluator retained its unresolved Null/Unknown type
rejections. These attempts are not claimed as successful loop emission. After
the single full run, its actual emitted loops were used to select RSSUtils,
TypedBundle and Function for additional filtered evidence; no code changes or
second full run followed.

RSSUtils `combins` has two sequential loops and a nested conditional. TypedBundle
`applyDelta` contains four loops with field reads in each header and actual
`setValue` calls in the bodies. Function `getAllowedFunctionCharacters` has two
sequential loops with char-array stores.

### Compiled actual APK source

```sh
/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac \
  -d /tmp/opencode/decomphify-while-apk-validation \
  /tmp/opencode/decomphify-java-corpus/run-1790846992707230625-1924769/com.skmc.okcashbag.home_google/sources/com/nbt/oss/exp4j/function/Function.java \
  bench/JavaWhileApkSmoke.java
/home/jtjisgod/tools/jdk-17.0.13+11/bin/java \
  -cp /tmp/opencode/decomphify-while-apk-validation JavaWhileApkSmoke
```

Both passed, using the actual generated class without dependency stubs or
signature changes. Output: `APK sequential while loops: 3 exact alphabet checks
passed`. The expected array is precisely
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_`.

Actual source excerpt from that class (first loop):

```java
int edge26 = v2;
int edge27 = v3;
v22 = edge26;
v24 = edge27;
while (true) {
    v4 = 91;
    v5 = (v24 >= v4);
    if (v5) {
        break;
    }
    v7 = (v22 + v6);
    v8 = (char) v24;
    v1[v22] = v8;
    v10 = (v24 + v9);
    v11 = v7;
    int edge28 = v11;
    int edge29 = v10;
    v22 = edge28;
    v24 = edge29;
}
```

## Single full final-build OKcashbag measurement

After implementation, tests, lint, formatting, release build and initial
filtered attempts, executed exactly one full export:

```sh
/usr/bin/time -v target/release/examples/java_corpus --workers 30 \
  --app tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk
```

Fresh artifact root:
`/tmp/opencode/decomphify-java-corpus/run-1790846887623015276-1913258`.
Report: `<root>/com.skmc.okcashbag.home_google/report.txt`.
Sources: `<root>/com.skmc.okcashbag.home_google/sources/`.
Combined summary: `<root>/summary.txt`.

The supplied baseline counters match the preserved report at
`/tmp/opencode/decomphify-java-corpus/run-1790832652145312482-472667/com.skmc.okcashbag.home_google/report.txt`.
No extra baseline export was performed.

| Metric | Supplied baseline | Final build | Difference |
|---|---:|---:|---:|
| Classes seen | 96,744 | 96,744 | 0 |
| Classes generated / written files | 38,547 | 38,553 | +6 |
| Ordinary complete bodies | 64,058 | 64,126 | +68 |
| Complete constructors | 17,313 | 17,313 | 0 |
| Complete static initializers | 2,017 | 2,017 | 0 |
| Unsupported methods | 396,634 | 396,566 | −68 |
| Unavailable constructors | 75,486 | 75,486 | 0 |
| Declaration-only methods | 33,173 | 33,173 | 0 |
| Fields emitted | 29,505 | 29,535 | +30 |
| Wall seconds (`time -v`) | 32.12 | 32.14 | +0.02 |
| Maximum RSS KiB | 1,764,028 | 1,728,660 | −35,368 |

Final app processing time: **31.872 seconds**; aggregate reported time: 31.884
seconds; user CPU: 210.86 seconds; system CPU: 5.96 seconds; exit status: 0;
app/output errors: 0. These are final-build results, not intermediate counts.
Source inventory found **78 emitted while sites in 62 Java files**. The body
increase is a report counter, distinct from the number of syntactic loop sites.

The APK export is still an honest supported subset. Unsupported members remain
reported in the ordinary counters and source diagnostics. No unsupported paths
were suppressed to obtain these results. No full 101-app run or Git commit was
performed. The wall/RSS difference is a single-run observation, not a claim of
statistically measured performance improvement.
