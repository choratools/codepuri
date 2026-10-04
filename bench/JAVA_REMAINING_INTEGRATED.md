# Remaining-module production integration — final measured artifact

This supersedes the helper-only routing statements in `JAVA_PRODUCTION_INTEGRATION.md`.
The bounded worker grammars now run through the production facade. Their proof
restrictions remain enforced; this is not arbitrary-CFG recovery or a runnable APK.

## Production routes

* **Explicit local classpath:** `JavaOptions.classpath: Vec<PathBuf>` is empty by
  default and preserves `Clone`. `emit_java` and `emit_java_profiled` load actual
  class/JAR/JMOD declarations and pass the provider into whole-input hierarchy and
  member resolution. `decomp`, `java_corpus`, and `java_profile` accept repeatable
  `--classpath PATH`. Invalid/duplicate declarations return errors. There is no
  ambient JDK lookup, download, or fabricated Android metadata. Explicit-provider
  APIs remain available.
* **Exceptions:** the facade now builds `java_exception_regions::Metadata` from
  actual DEX ranges, handler addresses and SSA offsets, using the same frozen-name
  hierarchy as ordinary assignments. It calls `exceptions::emit_method_region`.
  Protected branching, pretry operations, success commits, typed catches, proved
  finally and proved monitor regions use the existing transactional typed emitter.
  Dependency exception coverage is checked again at each original instruction.
  The old linear bridge remains only for its regression tests.
* **Loop control:** both loop modules are exported. When the ordinary structurer
  rejects a non-switch method, the facade permits the canonical loop-control
  structurer, and the backend calls `branching::plan_control`. Its Planner hooks
  reuse definition, branch, read-availability, Boolean and exact Phi validation.
  Original edge copies execute once before labeled transfers; copy sources are
  snapshotted together. Labels represent actual loop headers. Posttest tests stay
  inside the original iteration body. Loop exit availability comes from proved
  breaks, not definitions from a possibly zero-iteration body. Switch-containing
  cycles still require a different grammar.
* **Enums:** before generic enum-header rejection, the facade lifts the actual
  constructor and initializer, calls `plan_enum_initialization`, verifies original
  `values`, `valueOf`, and any array helper with `plan_enum_methods`, validates
  surviving member IR against absorbed implementation identities, and requires
  complete remaining-member coverage. The assembler emits actual constant order.
  Frozen-name mismatch and unavailable remaining-member closure reject. Enum
  rejection reasons now identify the attempted initialization/method/member route.
* **Atomic initialization and constructors:** a reusable raw-bytecode
  `WriteInventory` scans every input pool before filtering, including failed,
  dropped and unreachable bodies. The facade supplies complete constructor SSA,
  bindings, clinit and genuine class/access/overload/throws declarations to
  `plan_class_initialization_with_inventory`. Encoded prefix, blank-final proof,
  complete this/super family and original clinit are approved together. A proposed
  initialization family must survive fixed-point member closure before publication.
  Successful fields have no duplicate declaration initializer; the encoded block
  precedes original clinit. Initializer proof failures are reported explicitly.
  Hierarchy declarations are indexed once, and SSA uses shared DEX metadata.

### Important scope of the initialization result

This APK produces **9,739 unknown whole-input write observations**. The inventory
intentionally treats unavailable/native/opaque or unresolved effects as unknown,
so it cannot prove final-write absence or exclusivity for this complete input.
**50,397 rejected initializer records** now expose that actual indeterminate proof.
No blanket final-write success is inferred from successfully emitted methods.
The atomic production route is integrated; general blank-final APK recovery is
not established. Likewise, this artifact has **zero explicit `this(...)` sites**
and **zero synchronized sites**. Constructor-family/monitor helper runtime tests
do not change those corpus facts.

## Final gates and runtime evidence

After the last production changes:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
env PATH=/home/jtjisgod/tools/jdk-17.0.13+11/bin:$PATH cargo test --workspace --quiet
cargo build --release -p decomp --examples
```

Passed: **376 tests, 0 failures, 9 opt-in/ignored tests**, zero test warnings.
Then explicitly ran all three ignored decomp library corpus tests with the real
APK and JDK17: **3 passed, 0 failed**. The initial opt-in attempt lacked the
required `EXCEPTION_APK`; a relative-path retry resolved from Cargo's crate cwd
and failed to locate it. The final absolute-path invocation passed.

Logs:

* `/tmp/opencode/decomphify-remaining-tests.log`
* `/tmp/opencode/decomphify-remaining-real-tests.log`

The final release build took **10.22 s**. JDK **17.0.13+11** compiled unchanged
units from the final artifact with its real `sources/` as sourcepath, without
source edits or dependency stubs. Classes are in
`/tmp/opencode/decomphify-remaining-final-realunits`.

`bench/JavaRemainingApkSmoke.java` executes:

* actual `CameraState$Type` and `Timebase` names/order/ordinals/valueOf and fresh
  `values()` arrays;
* actual commons-lang3 `math/c` parse methods, including null pretry returns,
  invalid input catches, numeric limits and successful values;
* actual `IntStack.indexOf`, including selected Phi transfers, labeled continue,
  early return, exhaustion break and zero iterations. Field state is supplied
  explicitly; this setup does not claim recovery of a missing constructor.

The existing production/array/while APK harnesses also passed against final units:

```text
PASS unchanged production enums, clone/valueOf/ordinals, pretry and typed catches
PASS unchanged APK grouped/default switch, nested loops, annotation defaults
PASS unchanged OKcashbag array/object source units
APK sequential while loops: 3 exact alphabet checks passed
```

Whole-APK Java compilation and Android execution remain unverified.

## Final full OKcashbag export

Final command, after the last production changes and gates:

```sh
/usr/bin/time -v -o /tmp/opencode/decomphify-remaining-final-time.txt \
  target/release/examples/java_corpus --workers 30 \
  --classpath /home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod \
  --app tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk \
  > /tmp/opencode/decomphify-remaining-final-stdout.txt \
  2> /tmp/opencode/decomphify-remaining-final-stderr.txt
```

One app worker, **30 class workers**, no filter/limit. Exit **0**, report available
**true**, errors **0**, stderr empty. Final artifact root:

`/tmp/opencode/decomphify-java-corpus/run-1790857072952458608-3163583`

| Counter | Most recent baseline | Final | Delta |
|---|---:|---:|---:|
| Classes seen | 96,744 | 96,744 | 0 |
| Java files generated/written | 38,956 | 42,877 | +3,921 |
| Ordinary complete methods | 64,205 | 70,175 | +5,970 |
| Complete constructors | 17,313 | 21,316 | +4,003 |
| Complete static initializers | 2,017 | 2,424 | +407 |
| Unsupported methods | 395,886 | 389,493 | −6,393 |
| Unavailable constructors | 75,486 | 71,483 | −4,003 |
| Declaration-only methods | 33,774 | 33,790 | +16 |
| Fields emitted | 29,537 | 32,705 | +3,168 |
| Skipped classes | 0 | 0 | 0 |
| Wall seconds | 36.22 | 208.16 | +171.94 |
| Peak RSS KiB | 1,735,328 | 3,209,500 | +1,474,172 |

App time **207.749 s**, runner **207.756 s**, user CPU **1914.37 s**, system
**18.01 s**, swaps **0**. This run is substantially slower than the unsupplied-
classpath baseline; no speed improvement or isolated causal attribution is claimed.
An earlier pre-cache attempt timed out at 200 seconds without a final report;
its partial artifact is excluded. An intermediate completed run is also excluded
from the final table.

Counter accounting includes proved compiler-equivalent enum absorption: 336 enum
constructors and initializers, their original values/valueOf methods, and 210
verified array helpers. These are not all literal Java method declarations.
The provenance audit finds **8,826 added explicit retained bodies**, **16 added
declarations**, and **zero removed previously retained method identities**.
Ordinary + unsupported + declarations + static initializers remains **495,882**;
constructors + unavailable constructors remains **92,799**.

Actual syntax in the final sources: **336 enums**, **48 labeled loops**, **69
labeled breaks**, **49 labeled continues**, **262 try sites**, **11 finally sites**,
**0 synchronized sites**, **20,980 explicit super sites**, **0 explicit this sites**.
These are syntax-site counts, not a classification of every original CFG.

## No-classpath default fixture

Using the same final binary, exported only `Landroidx/biometric/ErrorUtils;`, one
worker, limit one, with **no classpath argument**. Report: one class written, one
ordinary method, one constructor, two unsupported methods, zero unavailable
constructors, zero emitted fields, zero errors; **20.615 s**. Root:

`/tmp/opencode/decomphify-java-corpus/run-1790857289029850147-3186417`

Compared with the preserved most-recent baseline unit. The sources differ only
in ordering of grouped switch labels; the existing switch grouping uses
unordered iteration. The audit asserts equality after sorting each contiguous
group of labels. This is **semantic fixture parity, not byte-identical output**.
The default fixture compiled and passed the 36-input production switch harness.

## Exact remaining first-failing proof rules

Counts below come from this final artifact, with only arena IDs normalized.
They are recorded rejection frequencies, not counts of all latent features.

| Rule | Records |
|---|---:|
| Whole-input final-write proof indeterminate | 50,397 |
| Required genuine field declaration unavailable | 70,431 |
| External invocation accessibility unavailable | 37,041 |
| Missing/ambiguous method declaration metadata | 32,064 |
| External field source closure unavailable | 28,100 |
| Overlapping structured region | 25,335 |
| Cyclic control outside selected grammar | 2,035 |
| Branch/Phi exact type mismatch | 2,100 |
| Unsupported branch-plan instruction | 2,898 |
| Enum initialization proof unavailable | 10,555 |
| Unused constructor input requires declaration-aware family lowering | 4,494 |
| Atomic initialization/family proof unavailable | 4,333 |
| Managed synchronized method scaffold unproved | 3,649 |
| Multiple/nested exception source ranges | 7,066 |
| Multiple normal exits from exact protected range | 5,385 |
| Overlapping/cyclic exception region | 3,007 |
| Protected region not single-entry | 2,604 |
| Cyclic exception region | 876 |
| Noncontiguous protected region | 40 |
| Unsupported normal exception dispatch | 34 |
| Synthetic handler provenance unproved | 145 |
| Catch type not proven Throwable | 429 |
| Exception throwing-site metadata mismatch | 324 |
| Exception Phi type mismatch | 78 |
| Unproved monitor balance | 23 |

Specific selected-method result: `Operator.isAllowedOperatorChar` now reaches
loop lowering but still fails `TypeMismatch(ValueId(12v1))`; it is not counted as
recovered. Annotation enum-default closure still rejects **77** records. Unknown,
Null/Zero/uninitialized types, missing dependencies, protected/member access,
irreducible/shared-return flow, incomplete initializer families and general enum
payloads remain actual boundaries. No arbitrary catch/finally, hierarchy edge,
final assignment, or monitor scaffold is invented to pass them.

Reproduction/audit: `bench/java_remaining_inventory.py` and
`bench/java_production_inventory.py`; outputs:
`/tmp/opencode/decomphify-remaining-inventory.json` and
`/tmp/opencode/decomphify-remaining-comparison.json`.

No Git commit/push, download, or full 101-app run was performed.
