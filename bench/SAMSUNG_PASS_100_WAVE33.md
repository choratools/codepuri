# Samsung Pass wave33: terminal, candidate not accepted

The original APK, ordered classpath and executable denominator are unchanged. The accepted baseline remains wave29. Both export and workspace handles are terminal; source freeze was released afterward.

| Metric | Frozen wave33 |
|---|---:|
| Original definitions | 205,497 |
| Original executable definitions | 194,286 |
| Explicit bodies | 182,649 |
| Separately proved enum equivalents | 429 |
| Executable recovery | 183,078 / 194,286 = 94.231185% |
| Remaining executable definitions | 11,208 |
| Retained genuine bodyless declarations | 11,183; 28 pending |
| Gained / lost versus wave29 | 1,238 / 9 |
| Written source units | 32,417 |
| Syntax errors | 0 |
| Uncapped javac diagnostics | 50,727 (full compile fails) |
| Workspace | 1,085 passed, 0 failed, 151 ignored; 185 suites |
| Export wall / peak RSS | 380.63 seconds / 17,477,060 KiB |

Release, strict workspace/all-target Clippy and formatting passed. Export exited 0 with unchanged source, binary, input and classpath receipts. Both no-loss guards exited 1: nine old original identities remain unavailable. No 100%, whole-app compilation or Android runtime claim is made. Benchmark ran concurrently with workspace tests and is not an isolated performance comparison.

## Confirmed repairs

Numeric comparison inference preserves exact integral producers instead of narrowing array length and copies to Byte. The exact original a4.a$c.k method #122 and its h/i/j callers now have complete bodies.

Deferred allocation cleanup uses a private exact-site token only after typed empty-fragment emission and the existing allocation/initializer proof with identical original scope and ordered handlers. Constructor effects and throws remain at the actual initializer. The original Lottie #23985 and four callers are restored; SeMobileService #31994 is restored.

The requirements gate now uses the same proved receiver-static checked target as backend emission, while retaining original symbolic access and dispatch checks. Four original AEAD methods #10608/#10609/#10620/#10621 are restored against genuine metadata. No synthetic native bodies or relaxed exceptions were used.

## Remaining regression queue

Six root classes3 methods remain checked-contract failures: #43824, #43830, #45524, #46352, #46603 and #60310. Their three dependency callers #43829, #45521 and #46595 also remain unavailable. The exact identities are preserved in `samsung-pass-wave33-final.json`. Next work proves source/runtime contract and allocation ownership for these methods; the baseline is not advanced around the losses.

## Evidence

Frozen root: `/tmp/opencode/samsung-pass-100-wave33`. Coverage, ordered metadata hashes, snapshot, all-method ledger, whole-source compiler diagnostics, both no-loss guard receipts and workspace log hashes are retained. `bench/samsung-pass-wave33-final.json` is the authoritative compact report.
