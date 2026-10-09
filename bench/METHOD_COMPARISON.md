# Original-method comparison research dataset

## Build and run

```bash
cargo build --release -p decomp --example method_inventory --example java_profile
python3 bench/method_comparison.py \
  --manifest /tmp/opencode/adb-major10/manifest.json \
  --manifest /tmp/opencode/adb-extra7/manifest.json \
  --classpath /home/jtjisgod/Android/Sdk/platforms/android-35/android.jar \
  --out /tmp/opencode/method-comparison-17apps
```

Use `--package PACKAGE` (repeatable) for a subset. An output root must be fresh,
or use `--resume` to reuse committed inventory/export observations with matching
APK hashes, tool binaries, classpath and worker count. Failed/incomplete tool
runs are not silently treated as successful exports.

Generated APK/source/SQLite artifacts are local, not committed. The completed
17-app dataset is at `/tmp/opencode/method-comparison-17apps/comparison.sqlite`.
Its `configuration.json` records tool binary hashes, revision, classpath hash and
manifest hashes. `summary.json` and `SUMMARY.md` contain the scope matrices.

## What is recorded

- APK and DEX SHA-256, DEX entry, logical container index and Codepuri root-pool
  membership; non-root asset parse failures remain explicit.
- Defined original owner/name/parameter/return descriptors and pool method index,
  flags, physical code offset, register/code-unit/instruction/try counts and
  decoded operation-family counts.
- Raw instruction SHA-256 and a conservative symbolic-pool code fingerprint.
  Fingerprints preserve register/branch/payload layout and exception metadata;
  debug info is excluded. Unsupported symbolic references produce no fingerprint.
  Equal fingerprints are duplicate candidates, not semantic-equivalence proofs.
- Codepuri source identity/availability comments and whole-unit unpublished
  receipts. Body-emitted dependency diagnostics are distinct from absent bodies.
- JADX JSON method observations attributed by exact DEX/code-item offset plus
  raw signature, or exact descriptor for declarations. Aliases alone are never
  sufficient. Missing/ambiguous records are kept in `unmatched`.
- Per-class Codepuri elapsed profiles. They are not divided among methods.
- A `validations` table reserved for actual compiler/runtime checks. It is empty
  in this initial dataset; no correctness claim is manufactured from markers.

JADX can inline, merge, rename or reorganize original methods. An unobserved
identity is therefore not automatically a failure. A JSON method without an
error marker is not automatically correct Java or semantically equivalent.
Codepuri's identity strings follow the DEX parser's Unicode scalar view; raw DEX
hashes remain authoritative for names containing isolated UTF-16 surrogates.

`method_comparison` has one row per original definition. Multiple observations
are preserved separately, avoiding count inflation from joined duplicates.
Raw observation counts are not unique method counts.

## Completed corpus observations

17 APKs contain **4,894,560** parsed original definitions, of which **4,894,344**
are in Codepuri's root-DEX scope. **4,620,319** definitions have code; the strict
instruction decoder reported zero errors in those parsed inputs. SQLite
`quick_check` returned `ok`; no observation refers to a missing method row.

| App | Parsed original definitions | Codepuri root scope |
|---|---:|---:|
| Chrome | 48,064 | 48,013 |
| Samsung Notes | 517,598 | 517,598 |
| SmartThings | 578,895 | 578,895 |
| Watch Manager | 34,871 | 34,871 |
| Samsung Wallet | 682,721 | 682,721 |
| OneDrive | 223,655 | 223,655 |
| Spotify | 404,775 | 404,775 |
| Facebook | 612,783 | 612,783 |
| Amazon Ziggy | 114,336 | 114,336 |
| Netflix | 15,256 | 15,256 |
| Audible | 658,003 | 658,003 |
| Samsung Internet | 233,691 | 233,526 |
| Newegg | 210,354 | 210,354 |
| eero | 302,981 | 302,981 |
| Amazon Vendor | 62,724 | 62,724 |
| Acronis | 59,194 | 59,194 |
| Bitmoji | 134,659 | 134,659 |

There are **132,188** cross-app symbolic code fingerprint candidate groups with
at least eight code units. The total observations include 4,885,387 Codepuri
descriptor/pool-index records and 142,429 unpublished receipts; JADX has
3,832,687 offset/signature records and 236,024 descriptor records. JADX also has
74,554 unattributed method records, 173 invalid JSON artifacts and 17 non-class
JSON documents, all retained as explicit unmatched evidence.

Input exceptions: Chrome `assets/webapk8.dex` has 51 definitions; Samsung
Internet `assets/webapk-1.dex` has 165. Neither is in Codepuri root scope.
Wallet's 5,283,943-byte asset ending in `.dex` fails DEX file-size validation;
its definition count is unknown, not zero.

In root scope, Codepuri explicitly unavailable/unpublished identities paired
with marker-free JADX body observations total **591,312** (494,450 + 96,862).
Codepuri body observations paired with explicit JADX diagnostics total
**26,672** (25,038 + 1,634 with Codepuri dependency diagnostics). These are
investigation queues, not correctness scores.

## Queries and next evidence

```bash
sqlite3 /tmp/opencode/method-comparison-17apps/comparison.sqlite \
  < bench/method_comparison_queries.sql
python3 -m unittest discover -s bench -p test_method_comparison.py -v
```

The supplied queries cover scope differences, asymmetric body observations,
cross-app fingerprints, class-level hotspots and attribution quality.
Follow up with bounded original-method compiler/runtime oracles, pool-independent
library deduplication, and actual method-pass repetition/timing instrumentation.
Per-method timings and repetition counts are currently unmeasured. This export
uses JADX JSON and Codepuri profiling/checksumming, so its process timings are
not substitutes for ordinary paired throughput measurements.
