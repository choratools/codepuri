# APK → Java development slice

```sh
cargo run --release -p decomp -- <apk> <outdir> [--limit N] [--class FILTER]
```

`FILTER` is a substring of the original DEX descriptor. `N` limits emitted
compilation units. The output directory's parent must exist. Existing Java
files are never overwritten. Library callers use `dex_java::emit_java` with a
`SourceSink`, including a callback accepting `&GeneratedSource`.

Pipeline: APK parsing → descriptor-aware DEX lifting/merging/SSA/type inference
→ incoming parameter bindings → `decomp_structure::structure` validation →
`backend_java::emit_method` → class grouping → descriptor-derived output paths.
The structure result is a validation gate; Java emission still supports linear
control flow only. Receiver and parameter values are identified by original
register histories and synthetic initial definitions (`0x8001`). Long/double
consume two incoming words with one value identity. Missing/unused initial
values currently reject the method rather than using a later assignment or
inventing an argument identity.

Each accepted body is complete, but **each class is a subset** (`Fallback`
provenance). Constructors, fields, inheritance and unsupported methods are
omitted, never replaced by bodies. Java's implicit constructor is not a
recovered original constructor. Names, packages, parameter/return types and
staticness remain original; `$` is literal, not guessed nesting. The original
method visibility is restored after backend emission. Member-dependent bodies,
interfaces/enums/annotations, initializers and special method flags are
conservatively rejected. Signature types may still require external app types;
no whole-app compilation/completeness is implied.

Reports distinguish complete ordinary bodies, unsupported non-constructor
methods, unavailable constructors, selected subset classes and skipped classes.
Rejection counts apply to selected classes only. APK extraction does not expose
entry-name provenance, so `origin` is absent rather than guessed.

## Calculator evidence (2026-09-30)

Input: `tests/corpus/apks_device/com.sec.android.app.popupcalculator.apk`

Command:

```sh
cargo run --release -p decomp -- tests/corpus/apks_device/com.sec.android.app.popupcalculator.apk /tmp/opencode/java-calculator
```

Observed: 3,068 classes examined, 26 subset classes emitted, **73 complete
ordinary methods**, **15,198 unsupported non-constructor methods**, **2,316
unavailable constructors**, zero skipped classes. No complete-app claim.

Closed arithmetic representative: `/tmp/opencode/java-calculator/v5/a.java`,
original `public static int d(int p0, int p1)`, returns
`p0 << (((p1 % 10) * 3) + 1)` via ordered SSA temporaries. It has only primitive
types and arithmetic, no app dependencies. It is a library method from the
calculator APK, not a reconstruction of the calculator UI or evaluator.

Compile attempt:

```sh
/usr/lib/jvm/java-21-openjdk-amd64/bin/javac /tmp/opencode/java-calculator/v5/a.java
```

Failed to start: `no such file or directory`. `command -v java javac` finds only
`/usr/bin/java`; the JVM bin directory contains `java`, `jpackage`, `keytool`,
`rmiregistry`, and no `javac`. No compiler was installed. Source compilation is
therefore **unverified**, not passing. `cargo test -p decomp` covers real-pool
metadata, exact descriptors, UTF-16 surrogates, this/wide/narrow histories,
overwritten argument rejection and end-to-end SSA arithmetic emission.
