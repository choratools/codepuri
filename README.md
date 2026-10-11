# Codepuri

![Rust 2024](https://img.shields.io/badge/Rust-2024-orange?logo=rust)
![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue)
![Status: Experimental](https://img.shields.io/badge/status-experimental-orange)

**Codepuri is a Rust-first Android APK/DEX → Java decompiler.** The development
CLI accepts conventional APKs with `AndroidManifest.xml` and `classes*.dex`;
the library also accepts parsed DEX inputs. It favors an
honest, verifiable source subset over fabricated stubs: methods that cannot be
supported with the available bytecode and metadata are reported as unavailable.

The project is library-first, with a development CLI for exporting APKs. It is
an actively evolving decompiler core—not yet a drop-in replacement for JADX or
a complete Android application reconstruction tool.

> [!IMPORTANT]
> Codepuri emits only the classes and method bodies it can justify. An export
> can be useful Java source without being a complete, compilable or runnable APK.

## Contents

- [Features](#features)
- [Requirements](#requirements)
- [Quick start](#quick-start)
- [Android and runtime metadata](#android-and-runtime-metadata)
- [Compact source output](#compact-source-output)
- [Rust library](#rust-library)
- [Performance](#performance)
- [Architecture](#architecture)
- [Limitations](#limitations)
- [Development](#development)
- [License](#license)

## Features

- Parse conventional APKs containing one or more `classes*.dex` files.
- Decode Dalvik instructions and lift them through CFG, typed IR and SSA stages.
- Reconstruct supported Java control flow, expressions, constructors and
  initializers with explicit proof checks.
- Resolve declarations from the APK, optional Android/runtime DEX metadata and
  explicit JVM classpaths.
- Emit through a streaming sink or the built-in directory sink.
- Optionally move generated diagnostic comments out of Java files into a
  lossless `diagnostics.tsv` sidecar.
- Reuse a successful in-memory export with `JavaSession` when the same inputs are
  emitted repeatedly in one process.

## Requirements

- Rust **1.95+** (edition 2024).
- A JDK is not required to run the CLI. **JDK 17** is required by the
  compile-and-run workspace tests; some optional Java 25 fixtures require JDK
  25.
- An Android SDK `android.jar` is optional, but improves resolution of Android
  API declarations.

## Quick start

Build the CLI:

```bash
cargo build --release -p decomp
```

Export an APK to a directory:

```bash
./target/release/decomp app.apk out/ --workers 30
```

Show all CLI options:

```bash
./target/release/decomp --help
```

The output directory contains Java compilation units plus a `JavaReport` on
stdout. The report records emitted classes, recovered bodies and unsupported
or unavailable methods.

### CLI options

| Option | Description |
|---|---|
| `--workers N` | Maximum parallel class workers; must be positive. |
| `--class FILTER` | Select descriptors containing this substring. |
| `--source-prefix PREFIX` | Repeatable original DEX owner prefix to render, e.g. `Lcom/example/app/`; declarations remain available and contracts follow required dependencies. |
| `--library-prefix PREFIX` | Retain matching input owners as metadata while excluding their Java output. |
| `--library-evidence PATH` | Read trusted artifact/reference evidence and verify exact original class implementations before separating source. Raw LibScout JSON is not accepted. |
| `--library-min-score SCORE` | Candidate score threshold in `0..1`; default `0.95`. Requires evidence; lowering it never bypasses implementation or distinctive-anchor checks. |
| `--library-out PATH` | Route qualified library Java in the selected scope to a separate, non-overlapping directory. Without it, qualified library owners are not rendered. |
| `--limit N` | Limit the number of emitted compilation units. |
| `--source-release 17\|25` | Select generated Java source level; default is 17. |
| `--classpath PATH` | Repeatable JVM `.class`, `.jar`, `.zip` or `.jmod` declaration input. |
| `--metadata-dex PATH` | Repeatable runtime DEX or ZIP/APK containing root `classes*.dex` entries. |
| `--compact-source` | Remove standalone diagnostic comments from `.java` and write them to `diagnostics.tsv`. |

## Android and runtime metadata

Evidence-qualified library separation can be configured explicitly:

```bash
./target/release/decomp app.apk out/app-or-unknown \
  --library-evidence /path/to/evidence.json --library-min-score 0.95 \
  --library-out out/libraries --workers 30 \
  --classpath /path/to/android.jar
```

`library-selection.json` records the applied threshold, exact original owners,
artifact/reference hashes, and rejection reasons. Remaining source is
**app-or-unknown**, not automatically first-party. Original APK declarations
remain available. See [library separation](bench/LIBRARY_SEPARATION.md) for the
evidence generator, verification boundaries, and actual Absher results.

Without external metadata, calls to APIs absent from the input APK may remain
unresolved and the corresponding method body may be omitted. Pass the Android
SDK platform JAR to supply genuine API declarations:

```bash
ANDROID_SDK_ROOT=/path/to/Android/Sdk
./target/release/decomp app.apk out/ --workers 30 \
  --classpath "$ANDROID_SDK_ROOT/platforms/android-35/android.jar"
```

For device/framework declarations, a DEX archive can be provided separately:

```bash
./target/release/decomp app.apk out/ --workers 30 \
  --metadata-dex framework.jar \
  --classpath "$ANDROID_SDK_ROOT/platforms/android-35/android.jar"
```

Metadata is explicit and ordered: runtime DEX providers precede the JVM
classpath, and APK declarations retain priority. A runtime DEX is **declaration
metadata, not a `javac` classpath**. `android.jar` supplies API declarations;
it does not supply Android framework implementation source. Missing or
ambiguous declarations remain unavailable rather than being guessed.

See [`docs/runtime-dex-metadata.md`](docs/runtime-dex-metadata.md) for provider
precedence and metadata limitations.

## Compact source output

By default, Java files include comments explaining subset status and rejected
members. For cleaner Java files, use:

```bash
./target/release/decomp app.apk out/ --compact-source \
  --classpath "$ANDROID_SDK_ROOT/platforms/android-35/android.jar"
```

Standalone diagnostic comments are preserved in `diagnostics.tsv`, keyed by
source path and original line number. This mode improves readability, but the
sidecar can be large and writing it can increase export time; it is an output
presentation mode, not a speed mode. The default export format is unchanged.

## Rust library

The `decomp` crate exposes the exporter and `SourceSink` interface. Callers that
need to parse APKs directly also depend on `dex-format`:

```rust,ignore
use decomp::dex_java::{JavaOptions, emit_java};
use decomp::output::DirectorySink;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = dex_format::apk::parse_apk_path("app.apk")?;
    let options = JavaOptions {
        workers: 30,
        ..JavaOptions::default()
    };
    let mut sink = DirectorySink::new("out")?;
    let report = emit_java(&files, &options, &mut sink)?;
    println!("{} classes emitted", report.classes_emitted);
    Ok(())
}
```

For explicit metadata, use `emit_with_metadata` with `DexMetadata`,
`Classpath` and `MetadataChain`. `JavaSession` can retain one successful source
snapshot for repeated exports in the same process; this saves rerendering but
does not accelerate the first export and retains generated source text in
memory.

## Performance

Paired 30-worker measurements on one Samsung Pass APK, using JADX 1.5.1 and
Codepuri's ordinary unprofiled `emit_java` path:

| Mode | Codepuri median | JADX median |
|---|---:|---:|
| Default | **22.94 s** | 24.80 s |
| Opt-in `fast_proofs` | **22.52 s** | 24.65 s |

Four runs alternated which tool ran first to reduce filesystem page-cache order
bias. In this sample Codepuri emitted **28,575 files / ~200 MB**; JADX emitted
**17,546 files / ~86 MB**. Output grouping and completeness differ, so timings
are sample-specific and are not a universal speed guarantee. `fast_proofs` is
experimental: on other APKs it may change unpublished-method diagnostic
reasons, so it is not enabled by default.

Reproduce with a local APK:

```bash
cargo build --release -p decomp --example java_export_bench
python3 bench/jadx_vs_codepuri.py app.apk \
  --out /tmp/codepuri-bench --runs 4 --workers 30
```

Use `--fast-proofs-codepuri` for the opt-in path. For
`--profiled-codepuri`, first build `cargo build --release -p decomp --example
java_profile`. The benchmark writes a JSON report and per-run
`/usr/bin/time -v` logs. Use fresh output directories for every run.

### Ten-app APK spot check (2026-10-08)

One paired run was made for each of ten installed, single-base-APK apps.
Both tools used 30 workers and fresh output directories; tool order alternated
between apps. JADX 1.5.1 ran with `--no-res -j 30`; Codepuri ran with
`--workers 30 --classpath android-35/android.jar`. This is a one-run spot check,
not a repeated-run statistical benchmark.

| App | DEX input | Codepuri time | JADX time | Peak RSS (Codepuri / JADX) | JADX log errors |
|---|---:|---:|---:|---:|---:|
| Google Chrome | 7.7 MiB | 13.3 s | 16.4 s | 1.9 / 5.8 GiB | 4 |
| Samsung Notes | 89.5 MiB | 118.7 s | 61.4 s | 14.6 / 40.5 GiB | 94 |
| Samsung SmartThings | 106.9 MiB | 122.7 s | 102.5 s | 18.5 / 66.7 GiB | 374 |
| Samsung Watch Manager | 6.4 MiB | 9.7 s | 10.0 s | 1.4 / 3.5 GiB | 6 |
| Samsung Wallet | 123.3 MiB | 125.3 s | 82.8 s | 19.4 / 60.2 GiB | 107 |
| Microsoft OneDrive | 36.8 MiB | 42.7 s | 37.1 s | 7.0 / 24.3 GiB | 58 |
| Spotify | 69.3 MiB | 92.7 s | 134.9 s | 13.2 / 46.2 GiB | 154 |
| Facebook | 128.0 MiB | 131.1 s | 126.0 s | 21.3 / 69.3 GiB | 364 |
| Amazon (`com.amazon.ziggy.android`) | 15.9 MiB | 18.7 s | 15.8 s | 3.3 / 8.2 GiB | 54 |
| Netflix | 2.5 MiB | 3.4 s | 5.9 s | 0.7 / 2.5 GiB | 7 |

Across the ten runs, Codepuri used **678.4 s** total versus **592.8 s** for
JADX; Codepuri was faster on Chrome, Spotify and Netflix, while Watch Manager
was nearly even. Median peak RSS across the apps was **10.08 GiB** for Codepuri
and **32.42 GiB** for JADX. Codepuri emitted **614,283 Java files**;
JADX emitted **509,337**. These file counts and output sizes are not direct
quality measures because the tools emit different source subsets and
diagnostics.

All Codepuri runs exited successfully and reported **1,891,138 complete
ordinary method bodies** and **469,287 unsupported ordinary methods**. JADX
wrote source output for all ten APKs, but exited with errors for every app; its
logs reported **1,222 errors** in total. The JADX output contained 51,203 raw
`Method not decompiled` and 50,444 `Code decompiled incorrectly` marker
occurrences. These diagnostics are not directly comparable with Codepuri's
method counters. Facebook's JADX run also reported a `resources.arsc` parsing
error while continuing to emit Java sources. This comparison measures export
behavior only; it does not establish that either output is a complete,
compilable or runnable reconstruction.

## Architecture

```text
APK / DEX
   │
   ▼
dex-format ──► dex-decompiler ──► managed-ir / decomp-core
                                      │
                                      ▼
                                decomp-structure
                                      │
                                      ▼
                                  backend-java
                                      │
                                      ▼
                           Java source + diagnostics
```

| Crate | Responsibility |
|---|---|
| `dex-format` | DEX model/parser and APK ZIP container extraction. |
| `dex-decompiler` | Dalvik decoding, lifting, type inference and SSA passes. |
| `managed-ir` | Typed managed IR, instructions, SSA forms and verification. |
| `decomp-core` | Generic CFG, graph analyses, pass registry and diagnostics. |
| `decomp-structure` | Language-independent branch, loop and switch structuring. |
| `decomp-emit` | Language-neutral tokens, precedence, naming and source maps. |
| `backend-java` | Java declarations, expressions, control-flow emission and proofs. |
| `decomp` | Public facade, metadata/session APIs, output sinks and development CLI. |

## Limitations

- Exported Java is a **supported subset**, not a reconstructed Android project.
- Binary `AndroidManifest.xml`, `resources.arsc`, assets and native libraries are
  not decoded by the DEX-to-Java CLI.
- Unsupported methods and unavailable initialization remain explicitly
  reported; Codepuri does not fabricate successful bodies to fill gaps.
- Android/JVM metadata improves declaration resolution but does not prove that
  every generated source compiles together or that an APK can be rebuilt/run.
- JVM `.class`, .NET IL/C#, ELF/native decompilation and a GUI are not currently
  implemented.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

export JAVA_HOME=/path/to/jdk-17
export PATH="$JAVA_HOME/bin:$PATH"
cargo test --workspace
```

`JAVA_HOME` is needed for Java compile-and-run tests. Rust `unsafe` is forbidden
by workspace lint; keep library errors typed and preserve conservative failure
behavior. The regression APK corpus in `tests/corpus/` is local and is not
included in this repository checkout.

## License

Apache-2.0, as declared by the Cargo workspace. See the
[Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
