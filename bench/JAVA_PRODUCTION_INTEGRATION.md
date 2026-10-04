# Production integration and final recovery status

This measurement supersedes the production status and final-run counters in
`JAVA_WHILE_LOOPS.md` and `JAVA_LIMITATIONS_FINAL.md`. The exporter remains an
incremental supported subset, not full jadx parity or a recovered runnable APK.

## Integrated capabilities

* **Acyclic primitive-int switches:** facade chooses the extended structurer;
  the backend plans and renders actual switches. Signed labels, grouped cases,
  original defaults (including a shared case/default target), empty arms and
  terminal arms are preserved. Edge-specific entry Phi transfers use an entry
  switch when necessary; the grouped body executes once. Join transfers snapshot
  all sources before assignment. Only continuing arms contribute to post-switch
  availability. Selectors stay materialized at their original instruction site.
  No fallthrough or overlapping-body duplication is invented.
* **Nested pretest loops and multiple unconditional latches:** the existing
  extended natural-loop grammar now has production backend execution evidence.
  Header evaluation executes every iteration and on the final/zero-iteration
  test; every original latch gets its own edge-bound parallel copies. Definitions
  are hoisted once, assignments remain in place, and body-only values cannot
  escape a zero-iteration loop. Nested loops with two distinct outer latch Phi
  inputs compile and execute in the ordinary backend.
* **Conservative explicit try/catch:** structure/backend modules are registered
  publicly, and the facade bridges DEX `code.tries` and `SsaMethod::offset` into
  exact post-SSA scope metadata. The grammar is one linear protected path,
  disjoint linear typed handlers and an exclusive shared continuation. Synthetic
  normal-success commits require a single incoming edge and a read of the
  immediately preceding offset-bearing protected operation's result. Other
  offsetless nodes do not inherit an arbitrary surrounding source span. Handler
  entries must agree with encoded handler addresses and exact caught type indices.
  Unproven shapes/provenance produce specific `exception scope:` rejections.
  The frontend places a caught-object binding at its proven move-exception handler
  entry instead of method entry. The transactional typed emitter checks ordinary
  instructions and enables hierarchy-proven checked-call allowances only inside
  the protected body; those allowances are disabled in catches and continuation.
  Catch ordering currently requires a proven subtype-to-supertype chain;
  unrelated catches lack sufficient hierarchy/non-shadowing evidence and reject.
* **Annotation elements/defaults:** the facade emits validated code-free elements
  in the frozen class/member alias domain. Primitive/string/array/class-literal
  defaults use the atomic default planner. Referenced class literals require input
  or supplied classpath declaration evidence (with basic Java metadata exceptions).
  Enum-valued defaults reject specifically because enum constant source closure
  has not been proved. Declaration-only elements are counted separately from bodies.

Important corpus distinction: explicit try/catch is wired and passes an actual
DEX-to-SSA-to-Java runtime fixture, but **this final APK run emits zero try sites**.
The 48 scopes that reach the planner still lack a proven Throwable hierarchy in
the unsupplied external classpath. This is not claimed as corpus try recovery.

## Helper-only and deferred

* `java_class_kinds::initializers::encoded_field_initializations` remains a
  helper. The production facade has no complete cross-DEX field-write inventory
  covering dropped/failed bodies, so encoded finals and initializer recovery are
  not approved by a partial successful-body scan. Existing field/body gates remain.
* `trusted_constructor_superclass` remains a helper. Production constructors
  retain the Object-root restriction. Non-Object constructor integration still
  needs complete trusted input/classpath ancestry, superclass legality/access,
  exact overload/throws and same-alias accessibility evidence.
* `enum_initialization` is an initialization plan only. Production enum headers
  still reject. No implicit Java `values()`/`valueOf(String)` implementation is
  approved without proving the original synthetic methods and backing-field
  references; helper compilation is not proof of those original methods.
* Source break/continue, do/posttest loops, irreducible flow, abrupt body exits,
  branching loop evaluation prefixes, switch/exception loop bodies, finally,
  monitor/synchronized recovery, multiple/nested/nonlinear try scopes and general
  enums remain deferred. The generated loop guard's normal-exit `break` is not
  general source break recovery. Switch-containing cyclic methods still reject.
* External metadata APIs exist, but this run supplied no external classpath.
  Missing classpath, protected access, arbitrary uninitialized-reference graphs,
  Unknown/conflicting types, exact Phi-type restrictions and dependency closure
  remain real limits. Annotation use-site/retention recovery is not claimed.

## Final gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
env PATH=/home/jtjisgod/tools/jdk-17.0.13+11/bin:$PATH cargo test --workspace
cargo build --release -p decomp --example java_corpus
```

All passed: **354 tests passed, 0 failed, 6 existing ignored**, with **zero test
warnings**. The original 318-test workspace suite is preserved along with the
specialist helper tests and added integration regressions. JDK execution was
enabled; no JDK checks were skipped. Workspace log:
`/tmp/opencode/decomphify-production-workspace-tests.log`.

Evidence includes production grouped/default switches with dispatch/join Phis
and one-time selector effects; actual nested loops/two distinct latches; ordered
typed catches preserving prethrow state and success commits; protected-only
checked-exception allowances with catch rejection; exact DEX offset/commit scope
runtime and encoded handler-address mismatch rejection; production annotation
default reflection. Existing initializer, constructor, enum-helper, annotation,
array/object and interface tests also passed. Helper-only tests do not promote
their features to production recovery.

## Fresh final release run

The final release build completed in **9.47 s**. After the last production/test
changes and gates, ran the whole OKcashbag APK with one app worker and **30 internal
class workers**, no filter or limit:

```sh
/usr/bin/time -v -o /tmp/opencode/decomphify-production-final-time.txt \
  target/release/examples/java_corpus --workers 30 \
  --app tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk \
  > /tmp/opencode/decomphify-production-final-stdout.txt \
  2> /tmp/opencode/decomphify-production-final-stderr.txt
```

Exit **0**, report available **true**, errors **0**, stderr empty. Fresh root:
`/tmp/opencode/decomphify-java-corpus/run-1790850551448556691-2337418`.
`summary.txt`, per-app `report.txt`, and `sources/` are preserved there. No full
101-app export or Git operation was performed.

| Counter | Supplied baseline | Final | Delta |
|---|---:|---:|---:|
| Classes seen | 96,744 | 96,744 | 0 |
| Generated/written Java files | 38,553 | 38,956 | +403 |
| Ordinary complete bodies | 64,126 | 64,205 | +79 |
| Complete constructors | 17,313 | 17,313 | 0 |
| Complete static initializers | 2,017 | 2,017 | 0 |
| Unsupported methods | 396,566 | 395,886 | -680 |
| Unavailable constructors | 75,486 | 75,486 | 0 |
| Declaration-only methods | 33,173 | 33,774 | +601 |
| Fields emitted | 29,535 | 29,537 | +2 |
| Skipped classes | 0 | 0 | 0 |
| Wall seconds | 32.14 | 36.22 | +4.08 |
| Peak RSS KiB | 1,728,660 | 1,735,328 | +6,668 |

App time **36.018 s**, runner total **36.027 s**, user CPU **223.57 s**, system CPU
**6.08 s**, CPU **634%**, swaps **0**. This single run is about 12.7% slower by wall
time; no statistical performance attribution is claimed. The ordinary total plus
unsupported plus declaration-only total remains **493,865**. Constructor total
remains **92,799**. Unsupported decreased partly because **601 annotation
declarations** moved to declaration-only, not because all 680 became bodies.

## Feature sites and examples

Audited against the preserved baseline root
`run-1790846887623015276-1913258`, without rerunning it. Reproduction:
`bench/java_production_inventory.py <final-per-app> <baseline-per-app> <test-log>`.
Full output: `/tmp/opencode/decomphify-production-inventory.json`.

| Emitted syntax | Baseline | Final | Files in final |
|---|---:|---:|---:|
| While statements | 78 | 88 | 63 |
| Nested while statements | 0 | 5 | 3 |
| Switch statements | 0 | 87 | 67 |
| Try statements | 0 | 0 | 0 |
| Annotation declarations | 0 | 383 | 383 |
| Annotation default elements | 0 | 289 | 169 |

Method-provenance comparison finds **79 added retained bodies**, including **75
switch-containing methods** and **3 while-containing methods**, plus **601 added
declarations**. **Zero previously retained method identities disappeared**. Syntax
counts are not original CFG switch counts: an entry-copy switch and body switch
can represent one original dispatch. APK multiple-latch site counts were not
independently classified; that capability's execution evidence is the explicit
two-latch backend fixture.

Representative exact source examples (relative to final `sources/`):

* `androidx/biometric/ErrorUtils.java`: grouped cases and case 6 sharing default;
  `isKnownError` is a newly recovered switch body.
* `androidx/camera/core/impl/EncoderProfilesProxy.java`: two emitted switches.
* `com/enliple/keyboard/imgmodule/gifencoder/c.java`: `inxbuild` has two nested
  while sites, selection-sort Phi transfers, array mutation and index construction.
* `com/google/zxing/aztec/encoder/Encoder.java`: newly recovered `drawBullsEye`
  nested loops; external BitMatrix dependency remains explicit.
* `androidx/annotation/ChecksSdkIntAtLeast.java`: five actual defaults, including
  `int api() default -1;` and `String codename() default "";`.

### Unchanged APK source compile/run

JDK **17.0.13+11** compiled the actual final artifact units, with the artifact
`sources/` as `-sourcepath`, into
`/tmp/opencode/decomphify-production-realunits`. Units: ErrorUtils,
ChecksSdkIntAtLeast, gifencoder/c, TypedBundle, IntStack, FocusMeteringResult,
DeltaCounter and exp4j/function/Function. Javac also compiled the real artifact
TypedValues interface dependency; no source stubs or edits were used.

Harnesses: `bench/JavaProductionApkSmoke.java`, `bench/JavaArrayObjectCastSmoke.java`,
`bench/JavaWhileApkSmoke.java`. Actual runtime outputs:

```text
PASS unchanged APK grouped/default switch, nested loops, annotation defaults
PASS unchanged OKcashbag array/object source units
APK sequential while loops: 3 exact alphabet checks passed
```

The new harness checks 36 switch inputs, annotation defaults, and a full 256-row
nested selection sort with exact row/index state. It supplies gifencoder field
state explicitly; implicit fixture construction is not recovered DEX-constructor
evidence. Whole-APK compilation/execution remains unverified.

## Typed residuals in this final artifact

The report contains all exact rejection rows. These are recorded reasons at the
first failing gate, not estimates of every latent unsupported feature:

| Exception reason | Count |
|---|---:|
| Catch-all/finally scope unproven | 9,332 |
| Exactly one DEX try range required | 7,066 |
| Protected normal path nonlinear | 4,426 |
| Nonlinear/prematurely terminal scope path | 3,704 |
| Unproven synthetic protected provenance | 2,792 |
| Initializer exception lowering deferred | 1,465 |
| No exclusive normal continuation | 16 |
| Unproven synthetic handler provenance | 16 |
| Scope overlap/cycle | 1 |
| Caught type not proven Throwable | 48 |

The `exception scope` category totals **28,818**; another **48** fail the backend
Throwable proof. Annotation dependencies reject **77** enum-default closures and
**16** missing class declarations; **19** annotation elements fail abstract
declaration legality. Other typed diagnostic filters: **10,407** cyclic-flow,
**23,051** overlapping-region, **1,191** branch type mismatches, **542** unsupported
branch instructions, and **12,708** enum-declaration-deferred records. The
Object-root constructor hierarchy gate still rejects **19,160** constructors.

Traversal errors=0 does not erase **395,886 unsupported methods** or **75,486
unavailable constructors**. Recovery status above intentionally distinguishes
integrated syntax, metadata/proof restrictions, helper plans and deferred work.
