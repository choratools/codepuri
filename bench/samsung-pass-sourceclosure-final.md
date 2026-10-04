# Samsung Pass source closure — fresh final

AnalystAll output. Global declaration availability and initialization evidence are separate from semantic bodies and legacy strict same-class coverage. Whole-app compilation is not asserted.

| Metric | Prior final | Source-closure final |
|---|---:|---:|
| Strict explicit bodies | 81,828 | 89,026 |
| Semantic explicit bodies | 97,199 | 104,161 |
| Published units | 26,270 | 29,398 |
| Full uncapped javac errors | 51,302 | 57,245 |

Definitions: **205,497**. Global direct missing-method callers: **29,225**. Field-blocked callers: **51,051**; initialization-blocked callers: **50,568**. All exact identities and diagnostic messages are saved in the companion JSON files.

Tests: **{'passed': 465, 'failed': 0, 'ignored': 13}**. Syntax gate exit: **0**. Full javac exit: **1**; cap reached: **False**. Sources unchanged: **True**.

Pass30 export: **61.319 s / 2,734,608 KiB RSS**. Exact JDK17 java.base.jmod and existing Android35 jar. Source root: `/tmp/opencode/decomphify-java-corpus/run-1790912079467647626-1495733/com.samsung.android.samsungpass`.

## Exact identity deltas and diagnostic reconciliation

- Against **original**: strict **9,697 gained / 0 lost**; semantic **24,821 gained / 0 lost**.
- Against **prior**: strict **7,203 gained / 0 lost**; semantic **6,962 gained / 0 lost**.

The strict metric remains legacy same-class closure; global source availability is reported separately. AnalystAll retains every previously exported unit.

### original source-unit identity comparison

| Scope | Files | Before errors | After errors | After missing declarations | After code/type | After uncertain constructor selection |
|---|---:|---:|---:|---:|---:|---:|
| common | 24,749 | 38,158 | 50,000 | 42,412 | 2,719 | 4,869 |
| old_only | 0 | 0 | 0 | 0 | 0 | 0 |
| new_only | 4,649 | 0 | 7,245 | 6,770 | 234 | 241 |
### prior source-unit identity comparison

| Scope | Files | Before errors | After errors | After missing declarations | After code/type | After uncertain constructor selection |
|---|---:|---:|---:|---:|---:|---:|
| common | 26,270 | 51,302 | 53,555 | 45,787 | 2,770 | 4,998 |
| old_only | 0 | 0 | 0 | 0 | 0 | 0 |
| new_only | 3,128 | 0 | 3,690 | 3,395 | 183 | 112 |

These are complete compiler-attribution messages on matched units, not a claim that every classified error has the same root cause. Constructor-selection diagnostics remain explicitly ambiguous.

Original constructors newly recovered against prior: **206**. Initialization-unavailable source units: **20,646**. Descriptor arity is preserved even for unused parameters, without inventing SSA values. The complete constructor identity inventory and original rejection reasons are in `samsung-pass-sourceclosure-final-wholeclasses.json`.

## Unchanged real wholeclass evidence

Exact JDK17 compiler and existing Android35 jar; empty sourcepath; no source edits, source stubs or invented parent classes.

Previously failing retained units that now compile with the recorded genuine source closure:
- `androidx/constraintlayout/widget/l$a.java`
- `androidx/viewpager/widget/ViewPager$a.java`
- `com/google/android/material/appbar/CollapsingToolbarLayout$d.java`
- `em/f$type.java`
- `nx/a$type.java`
- `androidx/appcompat/widget/Toolbar$g.java`

Wholeclasses with newly recovered original constructors that compile, individually or with the recorded genuine source closure:
- `com/yessign/fido/jce/spec/KCDSAPrivateKeySpec.java`
- `i3/j0$type.java`
- `la/y$type.java`
- `com/google/android/material/appbar/CollapsingToolbarLayout$d.java`
- `androidx/appcompat/widget/Toolbar$g.java`

All selected probe failures, exact compiler commands, before/after source hashes and complete logs remain recorded; successful probes do not imply whole-app compilation. The final whole-source gate still fails.

## Production integration and remaining bounds

- `FieldPublication` is collected before alias freezing with the export’s one write inventory. Actual owner declarations precede method bodies, including field-only owner units. Proven encoded prefixes use frozen names and precede original clinit. Interface fields remain unavailable without a proven legal initializer. Malformed metadata is separately reported.
- `GlobalDeclarations` caches defining-pool locations, flags, exceptions, signatures and body status. The global worklist consumes actual cached semantic publications, visits each original identity once, and reports missing bodies/declarations without creating them. Class limits cannot attest undelivered worker output. Original native/abstract declarations require actual source publication; genuine external declarations require exact classpath metadata.
- Constructor and static branch-tree implementations remain the backend authority; atomic constructor family/hierarchy/super-chain and original-final-field proofs remain required. The family API accepts signature-preserving declaration bindings. `KCDSAPrivateKeySpec` retains its original six-argument constructor and unused `p4`, actual `super()` and actual field writes; the unchanged wholeclass compiles. `CollapsingToolbarLayout$d` retains all three original constructors against the genuine Android `FrameLayout.LayoutParams` parent and compiles with its real emitted `ub/m$type.java` resource-owner unit.
- Analyst output is the default and preserves all old source identities. Optional `StrictValidClasses` remains an explicit helper selection policy; it is not applied to the default export. Strict coverage in this benchmark means the historical same-class body closure, not wholeclass javac validity.
- Source closure is still partial: unavailable method bodies, superclass/interface obligations, unresolved dependencies and incomplete initialization remain visible. Declaration publication does not establish execution-equivalent initialization or full application correctness. The output retains SourceFallback provenance.
- Performance: one cached original-owner index avoids repeated full-input class scans for each field access. Owner field and write lookups are bounded ranges. Pass30 is unchanged; memory grows with the cached global catalog and buffered actual publications.
