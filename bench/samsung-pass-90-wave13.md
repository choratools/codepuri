# Samsung Pass Wave13 — measured partial progress

Immutable denominator: 205,497 original defined methods. Accepted identities:
149,010 complete bodies + 300 proven enum equivalents = 149,310 (72.657995%).
90% requires 184,948; remaining 35,638. No bodyless declarations or unresolved
semantic-only bodies are counted as accepted.

Against Wave12: 1,992 gained and 740 withdrawn. Withdrawals are under exact
source-throws contract audit. This is not a zero-regression claim.

Full30: 226.208 seconds, all 34,579 classes visited, 30,344 source units,
zero export errors/skips. All exact units pass syntax parsing. Uncapped javac
still fails with 63,287 errors (previous 64,637). No source exclusions or stubs.
Workspace tests: 551 pass. Strict workspace/all-target Clippy passes.

Implemented native UTF-16 String literal/LDC emission, bounded descriptor-typed
constructor null/array/static-field expressions, and source-required checked
exception derivation with original metadata preserved. Focused JDK runtime and
negative tests pass. Six actual constructor identities gained in filtered probes.

Evidence: samsung-pass-90-wave13.json, -inventory.json, -delta.json,
-validation.json, -run.txt, -time.txt, -tests-final.txt, -clippy-final2.txt.
Output: /tmp/opencode/decomphify-java-corpus/run-1790940291461257430-654148/com.samsung.android.samsungpass

Goal remains active. Coverage is not whole-class compilation or APK execution.
