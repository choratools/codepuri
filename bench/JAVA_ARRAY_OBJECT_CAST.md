# Java arrays, objects, and casts — implementation and actual validation

## Scope and implementation

Implemented in `crates/backend-java/src/managed_ops.rs`, dispatched by the existing
ordered emitter in `src/lib.rs`. Narrow facade changes are in
`crates/decomp/src/java_integration.rs`. Existing aliases, filtering, scheduler,
worker defaults, SSA, type inference, width checks, and loop support were retained.

| Managed operation | Supported behavior / boundary |
|---|---|
| `NewArray` | Exact resolved component/full-array descriptor; int length; primitive/reference arrays; multidimensional descriptors allocate the outer dimension only (`new int[n][]`). Negative sizes and allocation exceptions retain Java behavior. |
| `ArrayGet` | Typed component load, with byte/short/char optionally widening to int; exact Boolean, long, float, double, reference, and nested-array results. |
| `ArrayPut` | Typed store; int-to-byte/short/char truncation is explicit. Other components require exact compatibility. Array/index/value operands are already materialized in instruction order. |
| `ArrayLength` | Real `.length` read at its original instruction, with an int result. |
| `FillArray` | Primitive components; all inputs materialized; whole payload bounds and null checked before mutation; element stores in payload order. Empty payload still checks null. Existing raw-bit constant emission is preserved. |
| `Cast` | Numeric conversion using the exact instruction target; byte/short/char narrowing remains explicit even if the SSA result is int. Reference check-casts use `(Target) (java.lang.Object) value` without refining the source into the target. |
| `InstanceOf` | Reference/array targets with an Object expression bridge; null returns false. |
| `NewInstance` + initializer | One adjacent, uniquely reached special `<init>` call, exact allocated owner, non-static void descriptor, exact parameter types, already evaluated arguments, and declared checked exceptions. Emits `new Owner((DescriptorType) arg, ...)` at the allocation site; elides only that paired initializer. |

`JavaResolver::instruction_type(TypeRef)` is a default-failing metadata hook, so
existing resolvers do not silently invent answers. The facade resolves real DEX
descriptors with the existing alias inventory and explicitly maps the synthetic
numeric opcode target space `0x80000080..=0x8000008f`. In particular, `0x8f` resolves
to **short**, even though the frontend currently stores its result as int.
Same-class initializer dependencies now enter the existing complete-body closure;
an unavailable constructor still prunes its callers.

### Explicitly deferred / rejected

* DEX `filled-new-array` / range remain **Opaque in the existing IR**, rather than
  `NewArray` plus fill. Their register/width contract is not guessed by the backend.
* Nonadjacent initializer calls, aliases or uses before initialization, argument
  computations between allocation and initialization, mismatched owners, malformed
  signatures, repeated/unpaired initialization, and explicit uninitialized types
  fail with typed `Unsupported` results. No zero-argument constructor is invented.
* External constructors still require trusted exception/member metadata and the
  existing class closure. A DEX method-pool signature alone cannot establish checked
  exceptions or source availability, so no external constructor signature is faked.
* Ambiguous Unknown/Zero/Null local types, unresolved targets, mismatched array
  widths, non-int indices, numeric/Boolean casts, reference/numeric casts, and
  inheritance-based assignment guesses remain unsupported. Concrete typed null
  references preserve Java null behavior.

## Tests and gates

Local tools were located using file globs, without installing anything:

```
/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac
/home/jtjisgod/tools/jdk-21.0.6+7/bin/javac
```

The JDK 17 directory was added to PATH for execution tests:

```sh
env PATH=/home/jtjisgod/tools/jdk-17.0.13+11/bin:$PATH cargo test --workspace --quiet
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release -p decomp --example java_corpus
```

All commands passed. Workspace tests include the existing type-inference, array/
check-cast, SSA, category-2 width, constructor, static-initializer, and branch/Phi
regressions; existing ignored tests remain ignored. Backend unit tests: **30 passed**;
backend branch integration tests: **15 passed**; facade unit tests: **42 passed**.

Twelve new backend tests cover source shape, actual compiled-Java execution, and
typed rejection. Ten of those compile/run Java: byte store truncation/sign extension;
short narrowing into an int result; Object check-cast and instance-of with null and
wrong classes; outer-only multidimensional allocation; all-or-nothing fill bounds;
exact char-array constructor arguments; Boolean/char/long/float/double array round
trips; once-only index/value effects including before a null-store failure; raw
negative-zero and NaN payload fill; and numeric NaN/saturation/truncation/char wrap.
Invalid targets, incompatible casts, unpaired/nonadjacent allocation, and long
indices are explicitly rejected. A new facade test validates all sixteen synthetic
numeric targets and invalid synthetic references.

No Java fixture runtime checks were skipped in the JDK-enabled workspace gate.

## Filtered APK gates (30 workers)

Input: `tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk` (76,686,842 bytes).
Commands used the unchanged runner with `--workers 30 --app <input>`, plus the
filter and limit below. All completed with available reports and zero app/output
errors. Artifact directories have prefix `/tmp/opencode/decomphify-java-corpus/`.

| Filter / limit | Artifact directory | Written classes | Ordinary bodies | Unsupported | Complete ctors | Unavailable ctors | Seconds |
|---|---|---:|---:|---:|---:|---:|---:|
| `android/support/v4/util/ContainerHelpers` / 2 | `run-1790829223350962483-64013` | 0 | 0 | 0 | 0 | 0 | 10.823 |
| `androidx/core/content/res/GrowingArrayUtils` / 1 | `run-1790829241756632727-66301` | 1 | 1 | 8 | 1 | 0 | 10.523 |
| `kotlin/UByteArray` / 1 | `run-1790829267729494015-69106` | 1 | 1 | 38 | 0 | 2 | 10.700 |

The first filter matched no class and is recorded as a no-match attempt, not a
semantic success. UByteArray still has Unknown-type and external-closure rejection
reasons; its unsupported methods were not counted as successful array emission.
The existing runner's filtering/limit behavior was preserved.

## Single full OKcashbag run

After the test, lint, build, and filtered gates, executed **one** full export:

```sh
/usr/bin/time -v target/release/examples/java_corpus --workers 30 \
  --app tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk
```

Fresh artifact root:

```
/tmp/opencode/decomphify-java-corpus/run-1790829294905991115-72256
```

Full report: `<root>/com.skmc.okcashbag.home_google/report.txt`.
Sources: `<root>/com.skmc.okcashbag.home_google/sources/`.
Combined report: `<root>/summary.txt`.

The latest available preserved OKcashbag full report was
`run-1790811305913942573-1251883/com.skmc.okcashbag.home_google/report.txt`, matching
the baseline in `JAVA_INTEGRATION.md`. Later preserved full reports were for
SmartThings, not OKcashbag; no extra baseline full run was made.

| Counter | Preserved OKcashbag baseline | New run | Difference |
|---|---:|---:|---:|
| Classes seen | 96,744 | 96,744 | 0 |
| Classes generated / written | 36,076 | 36,407 | +331 |
| Ordinary complete method bodies | 49,630 | 50,912 | +1,282 |
| Unsupported methods | 411,214 | 409,838 | −1,376 |
| Complete constructors | 16,975 | 17,023 | +48 |
| Unavailable constructors | 75,824 | 75,776 | −48 |
| Complete static initializers | 1,865 | 1,959 | +94 |
| Declaration-only methods | 33,173 | 33,173 | 0 |
| Fields emitted | 26,395 | 26,827 | +432 |
| Skipped classes | 0 | 0 | 0 |
| App/output errors | 0 | 0 | 0 |

New app time: **22.957 seconds**; runner total: **22.968 seconds**.
`time -v`: elapsed **23.19 seconds**, user **186.54 seconds**, system **5.47 seconds**,
maximum RSS **1,508,248 KiB**, zero swaps, exit status 0. The preserved baseline
elapsed 631.128 seconds on the earlier implementation/environment; the difference
is **not attributed to these operations**, because scheduler/cache/performance work
and concurrency changed in between.

Concrete rejection evidence: 1,678 ordinary-body and 192 static-initializer
rejections explicitly say `allocation requires adjacent initializer with
pre-evaluated arguments`. Their allocation/initialization effects were not moved.

Artifact-text checks (`rg -l <anchored expression> <sources> | wc -l`) found:
141 files with primitive array allocation statements; 124 with instance-of
expressions; 23 with descriptor-cast constructor arguments; 61 with fill bounds
guards. These are source-file counts, not operation counts or whole-app validation.

## Unchanged real APK source compilation/execution

Four actual output units from the full artifact were compiled **without editing
their generated source**:

* `androidx/constraintlayout/core/motion/utils/TypedBundle.java`: its actual
  constructor now allocates int/float/Boolean/String arrays with correct lengths
  and default values.
* `androidx/compose/runtime/IntStack.java`: actual array allocation and `peek(int)`
  load preserve default zero and out-of-range exceptions.
* `androidx/camera/core/FocusMeteringResult.java`: `create(boolean)` allocates with
  the exact private Boolean constructor; both true and false survive the field.
* `androidx/compose/runtime/external/kotlinx/collections/immutable/internal/DeltaCounter.java`:
  `copy(int)` calls the exact constructor, creates a distinct object, and preserves
  independent state under subsequent mutation.

Reproducible harness: `bench/JavaArrayObjectCastSmoke.java`. Compiled classes:
`/tmp/opencode/decomphify-java-ops-validation/`. The JDK 17 `javac -d` command used
that harness and the four full-artifact source paths; execution was:

```sh
/home/jtjisgod/tools/jdk-17.0.13+11/bin/java \
  -cp /tmp/opencode/decomphify-java-ops-validation \
  androidx.constraintlayout.core.motion.utils.JavaArrayObjectCastSmoke
```

Output: **`PASS unchanged OKcashbag array/object source units`**.
Numeric cast artifact evidence is also present in `kotlin/UByte.java`: its
`toShort_u2d_impl(byte)` contains `int v1 = (short) p0;` and a later short narrowing;
`toLong_u2d_impl(byte)` contains `(long) p0` before masking. Those Kotlin subset
classes were not asserted to compile as complete Comparable implementations.

## Residual limits

Compiled/executed fixtures and the four real source units establish the stated
local semantics. **The entire APK source tree was not compiled or executed.**
The facade still exports an honest subset; omitted interface implementations,
external dependencies, unavailable constructors, unresolved SSA types, loops,
exception reconstruction, and opaque filled-new-array operations can prevent
whole-class/application equivalence. No stub bodies, installs, model changes,
Git commit/push, or protection bypass were performed.
