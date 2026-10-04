# Samsung Pass wave37 verified result

Exact interface invoke-super now emits the proved direct-interface qualified call, preserving original selected target, receiver identity, access, overload selection and checked exceptions. Competing, redundant, incomplete or static interface targets remain denied. Source selection and emitted-method execution fixtures pass on JDK17 and JDK25.

Accepted executable definitions: 183,850 / 194,286 (94.628537%): 183,421 explicit bodies plus 429 proven enum equivalents. Compared with wave36, 118 ordinary identities gained (89 direct super sites and 29 dependency callers), zero lost. No-loss gates against wave36 and wave26 pass. Including 11,183 genuine no-body declarations, 195,033 / 205,497 original forms are retained (94.907955%). Remaining 10,436 executable definitions and 28 genuine no-body forms.

All 32,420 exact sources parse. Full uncapped javac still fails: 50,662 diagnostics versus 50,658 previously; the four additional diagnostics are cannot-find-symbol. No whole-APK compilation or Android execution claim. Resources and native libraries are still incomplete.

Workspace: 1,099 passed, zero failed, 157 ignored, 184 suites. Strict scoped Clippy, full formatting and release build pass. Export 287.15 seconds, peak RSS 16,435,948 KiB with concurrent tests/probes; this is not an isolated performance comparison.

Frozen evidence: `/tmp/opencode/samsung-pass-100-wave37`. Machine receipt: `bench/samsung-pass-wave37-final.json`. Goal remains active and incomplete.
