# Java rejection repairs — final production measurement

## Result and scope

Final full OKcashbag export: **69,658 written Java units, 188,617 ordinary
supported bodies, 24,020 constructors, 2,627 static initializers**. Compared with
the supplied baseline, this is **+26,781 units and +118,442 ordinary bodies**.
Export errors: **0**. All **96,744 input class definitions were visited**;
69,658 received source units (approximately **72.00%**, not 100% recovery).

These remain explicitly **Fallback/subset** units. Body counters do not establish
whole-unit compilability, full initialization, published dependency closure, or
APK executability. **45,567 written units report unavailable initialization**.
The larger classpath and code repairs changed together; the improvement is not
an isolated attribution to any one repair.

## Actually wired production behavior

* **Physical signature seeds:** the facade validates the full incoming descriptor
  word layout, including receiver and wide parameters. It supplies types by
  `(physical low register, width)` to `DexTypes`. The lifter applies these types
  when allocating actual synthetic initial reads in `ValueTable::read_typed`,
  before merge/SSA. Later file-order writes are not substituted for entry inputs.
* **SSA fallback boundary:** newly created `0x8003` fallback initials obtain
  descriptor types by their actual register lineage. Their physical histories,
  including both wide words, are retained. The inference context freezes seeds
  keyed to identities in that same IR, preserves conflict diagnostics, and never
  reuses a `ValueId` from another lift. `SsaMethod::initial_registers()` supplies
  attested words to `parameter_bindings_with_provenance`; unused descriptor
  positions remain explicit declaration positions.
* **Actual constant evidence:** production invocation arguments, field stores and
  returns consult `contextual_locals::evidence`/`expression`. Only actual pure
  Null/Zero definitions and their exact copy chains qualify for literal roles.
  Phi selection is not replaced with a literal. Shared category-one zero can
  serve independent numeric/reference operand roles. Long/double category-two
  zero is not inferred from category-one zero. Exact byte/short locals are kept
  at invocation operands; an unavailable exact narrow local rejects rather than
  emitting an illegal int argument or inventing a narrowing cast.
* **Genuine partial field dependencies:** required declarations may retain actual
  `final`, volatile/transient, descriptor, staticness and frozen member names
  without fabricating an initializer. Final body writes require the existing
  atomic initialization route; ordinary fallback emission cannot claim that
  proof. Declaration availability alone does not mean a blank final is Java
  definite-assigned or that an encoded value/body initializer survived.
* **Scoped write uncertainty:** the whole-input inventory is scanned once before
  filtering. Symbol-specific unresolved writes no longer contaminate unrelated
  field exclusivity queries; genuinely global opaque effects still block proofs.
  The parent checks the global unknown slice and scoped map directly, avoiding
  repeated allocation of the complete unknown-evidence vector per final field.
  There are still **5,527 global unknown observations** in this corpus.
* **Initializer coverage/status:** `check_class_initializer_coverage` checks the
  exact declared constructor/clinit identities. Publication additionally records
  whether encoded/final field initialization requires an unavailable atomic
  plan. Unit comments distinguish initializer-body coverage from field
  initialization proof. `JavaReport.unavailable_initialization_units` is reported
  separately from body/declaration counters; `SourceStatus::Fallback` is retained.
* **Access and private calls:** the facade uses declaration flags and class
  accessibility, original package identities, alias-package preservation, and
  checked protected receiver/hierarchy evidence. Same-owner private static calls
  and exact direct/private instance calls are permitted. The backend receives
  genuine `JavaResolver::method_access` flags and invokes the special-dispatch
  planner for private direct dispatch.
* **Bodyless declarations:** actually retained native/abstract declarations can
  satisfy same-class reference closure without being counted as complete bodies.
  This does not establish whole-input cross-class source publication closure.
* **Explicit platform metadata:** repeatable CLI `--classpath` paths remain empty
  by default. Genuine JDK17 and Android35 files are explicitly supplied below.
  Their overlapping `java.*` definitions require **ordered path precedence**:
  the first complete declaration wins, and members from shadowed classes are
  never merged. `add_path`/single-archive duplicate validation remains strict.
  Real javac fixtures verify both path orders and absence of member blending.
* **Bounded rejection accounting:** the corpus report retains ordinary,
  constructor and static-initializer histograms separately. Source rejection
  comments retain original `MethodKey` identities. The audit normalizes arena
  identities and emits bounded identity samples instead of a gigantic log.

### Boundaries intentionally still enforced

The contextual Phi family helper is available and tested, but **candidate-family
adoption is not integrated across all production branch/switch/exception
consumers**. Existing exact Phi/edge-copy checks remain enforced. No contextual
candidate is adopted without the missing all-consumer proof. Exact incoming edge
selection and simultaneous copies remain in the existing planner.

Exact Java-super-versus-DEX-super target selection lacks sufficient production
provenance. These calls reject specifically as
`special invocation: exact Java super target proof unavailable`; they are not
turned into virtual calls. Protected accesses involving aliased hierarchy
identities also reject specifically. General external field declarations,
canonical source spelling of nested external classes, complete initialization
families and cross-class source-subset closure remain incomplete.

## Final gates and retained-source validation

After the last production change:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
env PATH=/home/jtjisgod/tools/jdk-17.0.13+11/bin:$PATH \
  cargo test --workspace --quiet
cargo build --release -p decomp --examples
```

**405 passed, 0 failed, 11 ignored/opt-in tests**. The ignored tests are not
reported as passing. Log: `/tmp/opencode/decomphify-rejection-tests.log`.
Focused production regressions include newly allocated SSA fallback slots,
wide-entry word provenance, same-owner private static/direct calls, native
declaration references, partial final dependencies, actual mixed-role zero
operands, and compiled byte-argument locals. All-target Clippy and formatting
checks pass.

The **unchanged final artifact files** below compiled together with JDK
**17.0.13+11**, the genuine Android35 JAR on the classpath, and an empty sourcepath
directory (no invented dependency declarations):

1. `androidx/camera/core/CameraState$Type.java`
2. `androidx/camera/core/impl/Timebase.java`
3. `androidx/biometric/ErrorUtils.java`
4. `androidx/annotation/ChecksSdkIntAtLeast.java`
5. `com/enliple/keyboard/imgmodule/gifencoder/c.java`
6. `androidx/core/view/VelocityTrackerCompat$Api34Impl.java`

Compilation log is empty:
`/tmp/opencode/decomphify-rejection-final-javac.log`.
Class files: `/tmp/opencode/decomphify-rejection-latest-realunits`.
The last unit includes genuine Android API method references; it was **compiled
only**, not executed against the Android stub implementations.

`bench/JavaRejectionRepairsSmoke.java` and the existing
`bench/JavaProductionApkSmoke.java` ran against these class files:

```text
PASS unchanged final APK enum names/order/ordinals/valueOf/fresh arrays
PASS unchanged APK grouped/default switch, nested loops, annotation defaults
```

The enum test checks original names/order/ordinals/valueOf and fresh values
arrays. The production test checks 36 switch inputs, actual annotation defaults,
and the nested sorting/Phi/index method with explicitly supplied field state.
Supplying that state is not constructor recovery evidence. Neither harness calls
Android stub methods.

### Negative compilation evidence

A broader sourcepath compilation of the previous four smoke harnesses against
the measured predecessor of this final artifact failed on four **missing subset
dependencies**: the two AutoValue constructor signatures, `z.isNumeric`, and
`Intrinsics.checkNotNullExpressionValue`. Log:
`/tmp/opencode/decomphify-rejection-broad-javac.log` (artifact
`run-1790866737219522792-200515`, identical final body counters).
The earlier byte/short zero invocation errors were repaired and are absent from
that retry. No emitted sources were edited to make this compile.

An additional unchanged `MotionEventBuilder.java` compilation exposed the
unresolved external nested source spelling `MotionEvent$PointerProperties` /
`MotionEvent$PointerCoords`. Log:
`/tmp/opencode/decomphify-rejection-android-javac.log`. This is a known remaining
source-spelling boundary, not a successful Android compile/runtime claim.
Whole-export compilation and Android execution are **not established**.

## Final full OKcashbag30 run

```sh
/usr/bin/time -v -o /tmp/opencode/decomphify-rejection-final-time.txt \
  target/release/examples/java_corpus --workers 30 \
  --classpath /home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod \
  --classpath /home/jtjisgod/Android/Sdk/platforms/android-35/android.jar \
  --app tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk \
  > /tmp/opencode/decomphify-rejection-final-stdout.txt \
  2> /tmp/opencode/decomphify-rejection-final-stderr.txt
```

One app worker, **30 class workers**, no filter/limit. Exit **0**, report available
**true**, export errors **0**, stderr empty. Final artifact:

`/tmp/opencode/decomphify-java-corpus/run-1790867394314641377-302174`

| Counter | Supplied baseline | Final | Delta |
|---|---:|---:|---:|
| Classes seen | 96,744 | 96,744 | 0 |
| Generated/written Java units | 42,877 | 69,658 | +26,781 |
| Ordinary supported bodies | 70,175 | 188,617 | +118,442 |
| Complete constructors | 21,316 | 24,020 | +2,704 |
| Complete static initializers | 2,424 | 2,627 | +203 |
| Unsupported non-constructor methods | 389,493 | 270,848 | −118,645 |
| Unavailable constructors | 71,483 | 68,779 | −2,704 |
| Declaration-only methods | 33,790 | 33,790 | 0 |
| Fields emitted | 32,705 | 97,796 | +65,091 |
| Skipped classes | 0 | 0 | 0 |
| Wall seconds | 208.16 | 266.84 | +58.68 |
| Peak RSS KiB | 3,209,500 | 3,618,448 | +408,948 |

App time **266.393 s**, runner **266.437 s**, user CPU **2289.03 s**, system CPU
**19.73 s**, swaps **0**. Baseline used JDK17 metadata alone; final uses JDK17
plus explicitly supplied Android35. No isolated code-only speedup or coverage
attribution is claimed. Earlier attempts/intermediate completed exports are not
the final table. The first two-platform attempt stopped on duplicate platform
classes before ordered precedence was implemented and is excluded.

Accounting identities remain intact:

* ordinary + unsupported + declarations + static initializers = **495,882**;
* constructors + unavailable constructors = **92,799**;
* unsupported = **256,165 ordinary + 14,683 static-initializer rejections**.

The existing counters include previously proved compiler-equivalent enum
absorption; they are not all literal Java declarations.

## Latency investigation

A full **no-I/O, 30-worker** profile of the repair-stage predecessor is retained
at `/tmp/opencode/decomphify-rejection-profile/summary.txt` and `classes.tsv`.
It measured **255.355 s export wall**, all **96,744 class tasks**, approximately
**10.237 effective active workers**:

| Stage | Summed task seconds | Task-time share |
|---|---:|---:|
| Structure/backend | 1339.257 | 56.678% |
| Frontend | 504.285 | 21.342% |
| Initialization family | 479.968 | 20.313% |

This is diagnostic predecessor evidence, not the final benchmark timing.
Inventory and shared metadata are already prepared once per input/DEX. The
obvious repeated whole-unknown-vector allocation in the parent final-field gate
was removed afterward. The final measured wall time remains above baseline;
no claim that inventory replay alone explains the increase is supported by this
profile. Source recovery performs substantially more successful backend work.

## Final categorized rejection evidence

These are **recorded failing proof frequencies**, not an exhaustive list of all
defects inside every rejected method. Arena identities are normalized; original
method identities remain available in bounded audit samples and source comments.

| Ordinary rule | Records |
|---|---:|
| Zero lacks a supported nominal/local path | 44,181 |
| Required retained same-class method/declaration unavailable | 40,097 |
| External field source closure unavailable | 35,425 |
| Overlapping structured region | 25,327 |
| Field declaration metadata unresolved | 20,015 |
| Exact Java-super target proof unavailable | 9,617 |
| Invocation argument count mismatch | 7,752 |
| Observable/throwing operation before allocation initializer | 7,674 |
| Enum constructor proof unavailable | 7,161 |
| Multiple/nested exception source ranges | 7,066 |
| Unsupported branch-plan instruction | 6,516 |
| Exact branch/Phi type mismatch | 5,748 |

| Constructor rule | Records |
|---|---:|
| Whole-input final-write proof indeterminate | 58,230 |
| Unused constructor parameter needs family declaration support | 4,906 |
| Atomic field initialization proof unavailable | 2,325 |
| Enum constructor proof unavailable | 1,741 |
| Initialization metadata invalid/unavailable | 579 |

| Static-initializer rule | Records |
|---|---:|
| Whole-input final-write proof indeterminate | 12,470 |
| Enum constructor proof unavailable | 1,653 |
| External field source closure unavailable | 140 |
| Final write needs atomic initialization proof | 125 |
| Atomic initialization proof unavailable | 79 |

Audit/reproduction:

```sh
python3 bench/java_rejection_inventory.py \
  /tmp/opencode/decomphify-java-corpus/run-1790867394314641377-302174/com.skmc.okcashbag.home_google \
  /tmp/opencode/decomphify-rejection-tests.log
```

Retained audit: `/tmp/opencode/decomphify-rejection-inventory.json`.
No Git commit/push or full 101-app run was performed.
