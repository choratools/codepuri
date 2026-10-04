# Java integration: actual local APK exports

## Integrated output path

`decomp::dex_java::emit_java` calls `java_integration::emit`. That path builds one
frozen ASCII alias inventory over **all input DEX pools**, including external
references, and uses it for class headers, signatures, fields, body references,
and source paths. Original descriptors remain in source provenance. The old
first-slice emitter was removed; its baseline report was preserved.

Supported body output includes linear instructions and acyclic nested branches,
with typed method-scope locals and snapshot-before-assignment Phi edge copies.
Unused incoming parameters retain their descriptor positions without fabricated
SSA identities. The original strict `ParameterBindings` API remains available;
the facade uses the backend's new `DeclarationBindings` contract.

The facade validates incoming physical word layout, method declaration flags,
class/interface headers and inheritance, then prunes same-class callers to a
fixed-point closure over actual complete bodies and genuine field declarations.
Abstract/native declarations are counted separately from bodies. Checked
exceptions require exact declared-name containment; no exception hierarchy is
guessed.

Supported static initializers are linear and write declared non-final same-class
static fields. Supported constructors explicitly invoke the known no-argument
`Object` initializer and preserve supported subsequent statements. The facade
does not claim general constructor-chain or class-initialization recovery.

The extended structurer is registered and tested through its public API. Its
acyclic switch grammar is not yet lowered by the Java backend.

## Final six-app run

Actual artifact root:

```
/tmp/opencode/decomphify-java-corpus/run-1790811305913942573-1251883
```

Each row's sources are at `<root>/<app>/sources/`, and its full rejection counts
are at `<root>/<app>/report.txt`. The combined report is `<root>/summary.txt`.
All six reports were available; each app had zero reported app/output errors and
zero filtered/skipped classes. Generated and successfully written class counts
were equal for every app.

| App / artifact subdirectory | APK bytes | Classes seen | Classes written | Ordinary complete bodies | Complete constructors | Complete static blocks | Declaration-only methods | Fields emitted | Unsupported methods | Unavailable constructors | Seconds |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `com.skmc.okcashbag.home_google` | 76,686,842 | 96,744 | 36,076 | 49,630 | 16,975 | 1,865 | 33,173 | 26,395 | 411,214 | 75,824 | 631.128 |
| `com.android.settings` | 77,072,596 | 33,659 | 8,692 | 27,880 | 1,323 | 37 | 6,438 | 3,926 | 117,202 | 16,590 | 225.220 |
| `com.finnq.f1` | 94,053,232 | 94,701 | 18,356 | 34,517 | 0 | 0 | 32,572 | 20,087 | 435,895 | 90,408 | 621.898 |
| `com.samsung.android.spay` | 181,243,125 | 114,794 | 44,367 | 71,034 | 23,262 | 28 | 34,668 | 33,300 | 508,725 | 85,586 | 898.693 |
| `com.ubercab` | 197,377,083 | 286,515 | 93,582 | 32,628 | 60,369 | 2,430 | 110,232 | 62,476 | 1,306,165 | 290,134 | 2108.223 |
| `com.samsung.android.oneconnect` | 246,753,595 | 117,272 | 38,925 | 32,523 | 25,680 | 5 | 22,363 | 15,285 | 397,762 | 100,562 | 938.361 |
| **Total** | | **743,685** | **239,998** | **248,212** | **127,609** | **4,365** | **239,446** | **161,469** | **3,176,963** | **659,104** | **2730.206 wall** |

Run command (inputs opened read-only, no APK copying):

```sh
cargo build --release -p decomp --example java_corpus
target/release/examples/java_corpus --workers 3 \
  --app tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk \
  --app tests/corpus/apks_bulk/com.android.settings.apk \
  --app tests/corpus/apks_device/com.finnq.f1.apk \
  --app tests/corpus/apks_bulk/com.samsung.android.spay.apk \
  --app tests/corpus/apks_bulk/com.ubercab.apk \
  --app tests/corpus/apks_bulk/com.samsung.android.oneconnect.apk
```

The runner sorts inputs by file size and uses the explicit app list. Reports and
sources go to a fresh `/tmp/opencode` run directory. It no longer overwrites
`bench/java_corpus_first_slice.txt`.

### OKcashbag versus the preserved baseline

| Metric | Baseline | Final integrated run | Difference |
|---|---:|---:|---:|
| Classes seen | 96,744 | 96,744 | 0 |
| Classes generated | 985 | 36,076 | +35,091 |
| Classes written | 982 | 36,076 | +35,094 |
| Ordinary complete bodies | 2,051 | 49,630 | +47,579 |
| Unsupported methods | 493,831 | 411,214 | -82,617 |
| Unavailable constructors | 92,799 | 75,824 | -16,975 |

The new declaration-only and static-block counters are separate, so these are
not silently included in the ordinary-body comparison. Different concurrency
and machine load make the elapsed times unsuitable as a speed benchmark.

### Concrete final output evidence

Under the final root's `com.skmc.okcashbag.home_google/sources/`:

* `android/support/customtabs/ICustomTabsCallback$Stub$Proxy.java` contains
  `private android.os.IBinder mRemote`, an explicit constructor assigning it,
  and `asBinder()` reading that same field. Unsupported original methods have
  their original signatures and rejection reasons retained as comments.
* `androidx/core/content/res/GrowingArrayUtils.java` contains `growSize(int)`
  with typed hoisted locals, a real conditional, and separate edge snapshots
  assigning its Phi local before the return. Its other unsupported original
  methods remain explicitly identified.
* 45 OKcashbag source files contain emitted Phi snapshot assignments, counted
  with `rg -l ' = edge[0-9]+;' <sources> | wc -l`. This is an artifact-text count,
  not a Java compilation or execution claim.

## Earlier runs, including failures

All roots below have prefix `/tmp/opencode/decomphify-java-corpus/`.

| Run directory | Scope | Actual outcome |
|---|---|---|
| `run-1790809941165811760-1106104` | OKcashbag, `IAdvertisingIdService`, limit 4 | Report unavailable, 0 written; inventory rejected an array-owned method reference (`[F`). Fixed array-owner inventory handling before retry. |
| `run-1790809979711678789-1109385` | Same filter and limit | 96,744 seen, 3 written, 3 ordinary bodies, 7 unsupported methods, 1 unavailable constructor, 96,740 skipped; 9.369 seconds, no output errors. This predates the extra counters. |
| `run-1790810060793205858-1118009` | OKcashbag, `android/support/customtabs/ICustomTabsCallback`, limit 3 | 96,744 seen, 3 written, 11 ordinary bodies, 2 constructors, 0 static blocks, 11 declaration-only methods, 1 field, 15 unsupported methods, 0 unavailable constructors, 96,741 skipped; 9.308 seconds, no errors. |
| `run-1790810075801004472-1120054` | Full OKcashbag | Shell timeout at 200 seconds. Partial directory only; no final report or trustworthy final metrics. Not included in successful totals. |
| `run-1790810384041538374-1151489` | Full OKcashbag, one worker | 96,744 seen, 36,038 written, 49,433 ordinary bodies, 16,982 constructors, 1,865 static blocks, 33,173 declaration-only methods, 26,313 fields, 411,411 unsupported methods, 75,817 unavailable constructors; 580.158 seconds, zero errors. Later CFG-initial binding correction and stricter constructor flags explain the final-run differences. |

Filtered commands used the same runner with `--workers 1 --app
tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk`, adding `--class
IAdvertisingIdService --limit 4` or `--class
android/support/customtabs/ICustomTabsCallback --limit 3`. Full smallest-only runs
used that command without the filter/limit.

## Validation and remaining limitations

Commands executed:

```sh
cargo test --workspace --no-run
cargo test --workspace --quiet
cargo test -p backend-java -p decomp -p decomp-structure --quiet
cargo build --release -p decomp --example java_corpus
```

The first compile exposed ambiguous `MethodIr::from_parts` calls in the
constructor/static-initializer tests. Those were fixed with explicit `SsaForm`;
assertions were not weakened. Workspace tests passed, including existing SSA
regressions. After the final structurer registration and facade clean-up, scoped
tests passed again, including real facade field/alias/closure tests and integrated
branch rendering tests. Planner execution-model tests cover selected-arm effects,
empty/nested arms, and simultaneous copies; they are not generated-Java execution.

`javac` is absent. **No generated Java compilation, whole dependency closure, or
whole-app correctness is claimed.** Every generated unit remains `Fallback`.
In particular, concrete subset classes retaining interfaces can omit required
implementations, and retained superclasses may require unavailable constructors.

Still explicit unsupported/deferred: loops, exception reconstruction, Java switch
lowering, enums, annotation emission through the facade, general external member
closure, inheritance-based value compatibility, checked-exception hierarchy
reasoning, final/encoded field initialization, blank-final static writes,
managed synchronized scaffolds, arbitrary constructor chains, constructor
varargs/unused-parameter lowering, arrays/allocation/casts and other instructions
outside the existing typed emitter. No unsupported body is replaced with a stub.

No agents were delegated, and no Git commit or push was performed.
