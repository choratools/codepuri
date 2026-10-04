# Java limitation expansion: integrated implementation and evidence

## Status and measurement boundary

The four enhancements are wired into production, rather than merely registered
as standalone modules. The final tree passes workspace tests, strict all-target
clippy, formatting, release example compilation, and diff whitespace checks.
JDK 17 at `/home/jtjisgod/tools/jdk-17.0.13+11/bin` was on PATH for real Java tests.

**The single full APK measurement below precedes two late production corrections:**
the shared local-definition check now accepts explicitly planned Null/Zero
constants, and widened invocation arguments/receivers and field receivers are
cast to their descriptor types to preserve overload/member selection. Final
workspace tests and release build include these corrections. The APK counters
are exact for the measured snapshot, not claimed as final-tree APK counters.
No second full APK run was made; the requested one-full-run limit was preserved.

## Supported production paths

| Former limitation | Integrated support | Remaining boundary |
|---|---|---|
| Filled allocations | Managed `NewFilledArray` dispatches to `filled_arrays`; packed/range lowering, optional move-result, element order, SSA reads/defs, discarded-result local allocation, Boolean/narrow/float-bit interpretation, reference elements and typed null contexts. | Exact array descriptor required; long/double/void components rejected; malformed width/type contracts remain errors. |
| Nonadjacent initialization | Existing `object_plan` hooks handle a single initializer, proven move aliases, and an allowlisted straight-line pure computation gap. Argument assignment now uses resolver hierarchy evidence. | Effectful/may-throw gaps, allocations, control-flow barriers, preinitialization use, wrong owner, repeated initialization and explicit uninitialized types fail closed. |
| External metadata | `java_members::external_metadata` remains the original API; `java_external_metadata` re-exports the same module/types. `emit_with_metadata` uses a declaration catalog from all input DEX files and a caller-supplied provider. | Pool references are not declarations. Unknown exceptions/staticness and ambiguous input owners are not repaired by external fallback. External protected/private/package-access calls remain rejected. |
| Assignment/local typing | Public backend `type_compatibility`, facade `java_hierarchy`, and default-compatible `JavaResolver::assignable` are wired into arguments, fields, returns, array writes, filled elements and allocation/constructor arguments. Directed class/interface upcasts and reference-array covariance use evidence. | No guessed Unknown type, fabricated downcast, primitive-array covariance or guessed external ancestry. Phi/local joins still require the existing exact-type plans. |

The default resolver permits exact non-void identity and universal Java reference
rules, including Object and array marker interfaces. Nontrivial ancestry requires
a declaration graph. The facade builds this graph once from **all input DEX
class definitions**, plus `ExternalMetadata::hierarchy()` declarations. Every
node, superclass and interface edge is translated through the same source alias
inventory used by signatures; source aliases are never compared with original
hierarchy names. Classpath hierarchy names are original dotted class names.
Conflicting or malformed hierarchy ingestion currently disables graph-based
proofs for the export (empty hierarchy fallback); it does not invent ancestry.

Const processing calls the context planner **before** nominal `ty()` lookup.
Only proven `Constant::Null`/`Constant::Zero` values whose managed type is Null or
Zero qualify. Direct return, invocation, field-store, array-store and filled-array
contexts provide exact declaration types. All collected contexts must agree;
primitive Null and absent/conflicting contexts fail closed. The shared definition
path retains definition-identity/duplicate checks and has a constant-only opt-in;
ordinary Unknown definitions cannot use it. Indirect alias/merge inference is
not supplied by this path.

Widened invocation arguments are cast to exact descriptor parameter types, so
Java overload resolution does not choose an accidental subclass overload.
Widened invocation and field receivers use the resolved declaring owner type.

## Fundamental Java constraint and required inputs

Java source cannot express arbitrary bytecode uninitialized references. There is
no source-level operation for reserving an object independently of invoking its
constructor. The supported plan is a constrained reconstruction, not a solution
for arbitrary allocation/initialization graphs. Allocation/OOM observability
across moved pure computations and debug-local visibility remain limitations;
throwing or effectful intervening work is explicitly rejected.

For external members, the caller must supply exact original `MethodKey` owner,
name, parameter and return descriptors, declaration access flags/staticness, and
an exact exception descriptor list. `Some([])` asserts no declared exceptions;
`None` is unknown and remains unsupported. The declarations must actually exist
on the intended javac/runtime classpath. Hierarchy-dependent assignments also
require actual superclass/interface declarations, supplied through `hierarchy()`;
the default empty implementation keeps existing providers source-compatible.
No automatic JAR/android.jar reader or external trust verification was added.
External field closure remains a separate unsupported boundary.

## Supporting fixtures and actual Java execution

* `crates/dex-decompiler/tests/filled_arrays.rs`:
  `packed_and_range_preserve_elements_and_fuse_object_result`,
  `discarded_allocation_survives_and_wide_components_fail`,
  `component_constraints_interpret_raw_bits_and_zero`, and
  `range_expands_all_six_words_in_register_order` all pass.
* Backend `filled_arrays` unit fixtures validate discarded allocation position,
  name reservation, forbidden components and category-one source forms.
* Backend `integrated_filled_array_and_contextual_zero_execute` runs javac/java
  on actual emitted methods: retained/discarded int filled arrays, typed Null
  return, Zero reference return and Boolean zero. Unknown/Zero remains rejected.
* Backend `integrated_hierarchy_argument_field_return_and_array_store_execute`
  runs actual emitted String-to-CharSequence argument/field/return/array stores,
  filled reference elements and a Null element. A deliberately throwing String
  overload proves the descriptor-selected CharSequence overload is used. Removing
  hierarchy evidence rejects the same IR.
* `object_plan_tests::alias_and_pure_gap_compile_and_preserve_constructor_exception`
  compiles/runs actual emitted allocation aliases and a pure computation gap,
  checking constructor results and null/bounds exception/effect behavior.
  `effects_throwing_allocation_preinit_use_and_repeated_init_reject` checks all 12
  barriers; `explicit_branch_between_allocation_and_constructor_is_a_barrier`
  preserves the control-flow rejection.
* Backend `tests/type_compatibility.rs` passes directed/array rules,
  missing/cyclic evidence boundaries and real javac/java contextual-type checks.
  Facade `tests/java_hierarchy.rs` passes DEX/classpath ingestion checks.
* Facade `crossdex_constructor_uses_declaration_flags_and_exceptions` proves
  cross-DEX declaration use, duplicate-owner rejection, absent declaration,
  unknown exception and static-constructor negatives. Its explicit accessibility
  probe accepts public external constructor metadata and rejects protected,
  private and package access; hierarchy support adds no protected bypass.

Four **unchanged real APK output units** from the measured full artifact compiled
with JDK 17 and executed through `bench/JavaArrayObjectCastSmoke.java`:

1. `androidx/constraintlayout/core/motion/utils/TypedBundle.java`
2. `androidx/compose/runtime/IntStack.java`
3. `androidx/camera/core/FocusMeteringResult.java`
4. `androidx/compose/runtime/external/kotlinx/collections/immutable/internal/DeltaCounter.java`

Compiled classes: `/tmp/opencode/decomphify-java-expanded-validation`.
Runtime output: **`PASS unchanged OKcashbag array/object source units`**.
These units establish local array/object behavior, not whole-APK compilability.

## One full OKcashbag30 measurement

Command:

```sh
/usr/bin/time -v target/release/examples/java_corpus --workers 30 \
  --app tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk
```

Fresh artifact root:
`/tmp/opencode/decomphify-java-corpus/run-1790831718269568046-364271`.
Per-app report and sources are under `com.skmc.okcashbag.home_google/`;
the root contains `summary.txt`. Report available: true. Errors: **0**.
Classes seen: **96,744**. Skipped classes: **0**. Exit status: **0**.

| Counter | User-supplied latest baseline | Measured snapshot | Delta |
|---|---:|---:|---:|
| Generated/written files | 36,407 | 38,191 | +1,784 |
| Ordinary method bodies | 50,912 | 61,921 | +11,009 |
| Complete constructors | 17,023 | 17,211 | +188 |
| Complete static initializers | 1,959 | 2,017 | +58 |
| Unsupported methods | 409,838 | 398,771 | -11,067 |
| Unavailable constructors | 75,776 | 75,588 | -188 |
| Declaration-only methods | 33,173 | 33,173 | 0 |
| Fields emitted | 26,827 | 28,760 | +1,933 |
| Per-app export seconds | 23.19 | 30.881 | +7.691 |
| Peak RSS KiB | 1,508,248 | 1,760,152 | +251,904 |

Runner TOTAL seconds: **30.909**. `/usr/bin/time` wall seconds: **31.19**;
user CPU: 203.03 seconds; system CPU: 5.94 seconds. This is a single observed
measurement, not a repeatable performance conclusion or attribution by feature.

## Final gates and remaining risks

Passed on the final production tree:

```sh
PATH=/home/jtjisgod/tools/jdk-17.0.13+11/bin:$PATH cargo test --workspace --quiet
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release -p decomp --example java_corpus
git diff --check
```

Backend unit tests: **37 passed**; backend branching integration: **15 passed**;
type compatibility integration: **4 passed**; facade unit tests: **43 passed**.
All workspace suites passed; existing ignored tests remain ignored. No broad
production lint suppression was introduced; fixture-only panic/ordering allowances
are scoped to test code. The final release binary was rebuilt after the late fixes.

Remaining unresolved: arbitrary uninitialized reference graphs; effectful or
exceptional allocation gaps; Unknown/conflicting local types; exact-type Phi joins;
missing/cyclic/conflicting ancestry; unavailable constructors/member closure;
external metadata trust and actual compile dependencies; external fields;
protected access; whole-APK compilation/execution. Final-tree full-APK counters
remain unmeasured because the one permitted full run preceded the late fixes.
No Git commit or full 101-app corpus run was performed.
