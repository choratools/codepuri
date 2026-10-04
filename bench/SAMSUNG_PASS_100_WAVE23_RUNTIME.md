# Samsung Pass wave23 — runtime class-instance inheritance

This is a separate runtime-metadata variant, not a promoted SDK baseline. Frozen APK/four classpaths/runtime framework hashes match runtime wave21. New source additionally includes verified wave22 enum/allocation repairs.

| Metric | Runtime wave21 | Runtime wave23 |
|---|---:|---:|
| Accepted executable originals | 181036 | 181051 |
| Explicit bodies | 180642 | 180653 |
| Enum equivalents | 394 | 398 |
| Executable denominator | 194286 | 194286 |
| Identity gains / losses | — | 55 / 40 |
| Full uncapped javac diagnostics | 52243 | 52208 |
| Java sources | 32385 | 32392 |

Executable coverage **93.187878%**, 13235 executable originals remain. Bodyless declarations 11183 remain separate. All sources parse; whole javac still fails.

Runtime DEX class-instance proof retains real Signature/body presence/flags/throws and follows selected owner class hierarchy. It does not fabricate JVM Code or merge AndroidSDK member definitions. Nearest invalid/static/private/protected/generic/ambiguous declaration fails closed. Three genuine inherited View APIs resolved. Existing Static/Special/Super dispatch gates were reviewed and remain separate.

Newly rejected identities: 33 callee-exception-contract rejections plus 7 dependency closures. Exact records retained in adjacent losses.json; source-contract diagnosis is in progress. Do not blindly restore these or claim every new rejection correct without the original trace. Both new metadata capability and verified wave22 changes contribute to the 55 gains.

Workspace **994 passed, 0 failed, 137 ignored**, strict all-target Clippy and fmt passed. Export **523.49 seconds**, peakRSS **25964736 KiB**. This is not an isolated caching A/B.

Runtime declarations do not prove javac classpath availability or APK/platform historical compatibility. Native libraries, complete APK rebuild and Android execution remain unverified.

Immutable artifact root: `/tmp/opencode/samsung-pass-100-wave23-runtime`.
