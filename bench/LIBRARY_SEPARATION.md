# Configurable, evidence-qualified library source separation

## CLI

```bash
cargo build -p decomp --release --bin decomp --example library_inventory

./target/release/decomp app.apk out/app-or-unknown \
  --library-evidence /path/to/evidence.json \
  --library-min-score 0.95 \
  --library-out out/libraries --workers 30 \
  --classpath /path/to/android.jar
```

- `--library-min-score`: finite `0..1`, default **0.95**. Values such as `0.90`
  or `0.99` are accepted. NaN, infinity, negative and >1 values are rejected.
- `--library-evidence` is required when the threshold or library output is supplied.
- `--library-out` routes qualified sources within the existing selected scope.
  Without it, verified exact owners are excluded from Java rendering, while all
  original input declarations and required dependency analysis remain available.
- Application and library output directories cannot be the same or nested.
- Explicit manual `--library-prefix` controls remain independent of the verified policy.
- A receipt is written to `app-or-unknown/library-selection.json`; it never
  silently overwrites an existing receipt.

## Mandatory checks at every threshold

1. The input APK hash matches the evidence file.
2. The trusted original artifact file and its reference DEX archive match their
   recorded hashes. Reference conversion/provenance is provided by the trusted
   evidence-generation workflow; hashes alone do not prove publisher identity.
3. Original class descriptors map exactly; broad package prefixes are not inferred.
4. Class declarations, fields/static values, all supported original bodies,
   ordered exception handlers and runtime/system annotations match the reference.
   Pool indexes are normalized to symbolic references; register allocation and
   instruction layout remain strict. Build-only annotations and debug/source
   metadata are omitted from origin evidence.
5. At least one non-constructor/non-clinit body with eight or more instructions
   has a fingerprint unique to that class across the **whole original APK**.

Unsupported references, changed declarations/implementations, constructor-only
profiles, shared generic bodies and unmatched/renamed classes stay in the
application-or-unknown output. The policy does not infer licenses or certify an
exact installed version. Native code and renamed semantic equivalents require
additional evidence; this implementation does not guess their origin.

The exact implementation algorithm is intentionally conservative. Compiler/R8
changes can prevent a match even for genuine library code.

## Evidence format

```json
{
  "format": "codepuri-library-evidence-v1",
  "apk_sha256": "<64 hex digits>",
  "references": [{
    "id": "group:artifact:version-candidate",
    "artifact_file": "original.aar",
    "artifact_sha256": "<64 hex digits>",
    "dex_file": "reference-dex.zip",
    "dex_sha256": "<64 hex digits>",
    "score": 0.98,
    "score_basis": "LibScout fresh artifact signature profile"
  }]
}
```

Relative paths are relative to the evidence file. Unknown schema fields,
duplicate identifiers and ambiguous class definitions fail closed. A reference
DEX ZIP contains root `classes*.dex`; it is not a JVM class JAR and does not
need an APK manifest. CLI receipt owners are checked locally, not accepted as
an external detector's assertion. A raw LibScout JSON file is insufficient.

## Generate fresh reference evidence

`bench/library_separation.py` accepts an explicit candidate registry and explicit
toolchain paths. Candidate metadata contains `id`, `name`, `version`, `category`,
and an HTTPS URL for the original JAR/AAR. The output directory must be fresh.

```bash
JAVA_HOME=/path/to/jdk17 PATH=/path/to/jdk17/bin:$PATH \
python3 bench/library_separation.py \
  --apk app.apk --candidates candidates.json --out /tmp/new-library-evidence \
  --libscout /path/to/LibScout.jar --libscout-cwd /path/to/LibScout \
  --java /path/to/jdk17/bin/java --d8 /path/to/build-tools/d8 \
  --android-jar /path/to/android.jar --min-score 0.95
```

The generator downloads original artifacts, records hashes, profiles them with
LibScout, builds reference DEX using D8/min-api24, runs the fresh detector and
Codepuri implementation inventories, and produces `evidence.json` plus a preview.
The CLI independently recomputes the decisive implementation checks. Logs and
artifacts remain beside the evidence. Missing detector JSON is an analysis
failure even if LibScout returns process exit 0.

Fresh XML descriptions include a nonempty comment because the upstream exporter
can throw a NullPointerException when an optional comment is absent. The detector
algorithm itself is unchanged.

## Actual Absher result

Input: `/home/jtjisgod/absher/base.apk`, SHA-256
`172575d755606326dd5fd1d8c21dff7d3f5390ee782a0584746195952ac936e4`.

Original APK metadata supplies version hints for Google Maps **19.2.0**, phone
auth API **18.1.0**, and Firebase measurement connector **20.0.1**. These fresh
reference versions replace the old public-profile candidates for the experiment;
metadata hints are not used as implementation proof.

| Reference artifact | Fresh detector score | Distinctive exact implementation classes | Selected at 0.95 |
|---|---:|---:|---:|
| Play services maps 19.2.0 | 0.9156 | 23 | 0 |
| Play services auth API phone 18.1.0 | 1.0000 | 8 | 8 |
| Firebase measurement connector 20.0.1 | 1.0000 | 0 | 0 |
| Retrofit 3.0.0 candidate | 0.9824 | 0 | 0 |
| Guava 33.4.8-android candidate | 0.8993 | 2 | 0 |

At **0.95**, the CLI independently qualified **8 original classes**. Existing
Java subset publication rules delivered **4 library Java files**, with **86,865
application-or-unknown files**. Their disjoint union is **86,869 published files**,
matching the export report. Four qualified classes do not have published Java;
their original DEX remains available.

At **0.90**, a narrow-source CLI validation independently qualified **31 classes**
(8 phone + 23 maps). This is a classification check, not a second full export.
Scores below 0.95 did not bypass the same exact-body and anchor checks.

The 0.95 full export took **199.08s**, max RSS **24,195,476KiB** (single run,
including whole-APK evidence verification). The old export directory currently
contains only 59,021 of its originally reported paths, so it was not used as a
complete paired byte-output baseline. All available old paths remain in the new
source union; source routing was additionally checked with a controlled D8 fixture.

Outputs:

- Evidence: `/tmp/opencode/absher-library-detection/fresh-v2/evidence.json`
- Fresh scores and reference decisions: same directory's `summary.json`
- Java: `/tmp/opencode/absher-library-split-095/app-or-unknown/`
- Library Java: `/tmp/opencode/absher-library-split-095/libraries/`
- Output/DEX verification: `/tmp/opencode/absher-library-split-095/validation.json`
- 0.90 receipt: `/tmp/opencode/absher-library-score-090/app/library-selection.json`
- Updated LLM bundle: `/tmp/opencode/absher-llm-library-split-final/`

The LLM pack retains the threshold/selection receipt and all **9 original DEX
entries byte-for-byte** in `runtime-dex.jar`. It contains the existing app-focused
retrieval scope (5,607 compact Java files, 39,322 method identities, 50,297-character
initial context), not the whole application-or-unknown directory as one prompt.
The qualified SDK classes are outside that app-focused source prefix; no app
context size reduction is attributed to their separation.

## Verification

`crates/decomp/tests/library_evidence.rs` uses real javac/D8 fixtures for:

- 0.93 candidate retained at the default threshold and separated at 0.90;
- weak trivial, changed and globally ambiguous bodies retained even at threshold 0;
- APK/reference hash mismatch and invalid numeric score rejection;
- original-owner routing, explicit exclusion, metadata-preserved caller emission,
  default-threshold behavior and non-overlapping output directories.

Exact-owner neighbor preservation is also tested in `dex_java.rs`.

Final validation: **1,300 workspace tests passed / 175 ignored / 0 failed**;
workspace/all-target Clippy with `-D warnings`, formatting, diff checks and release
build passed. Python policy tests 3 and LLM-pack tests 8 also passed. A controlled
D8 fixture confirms routed Java bytes equal the unsplit output for all three
fixture classes. The full original runtime archive's nine DEX entries were
compared byte-for-byte against the input APK.
