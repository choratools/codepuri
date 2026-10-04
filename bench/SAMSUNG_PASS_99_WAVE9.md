# Samsung Pass wave9: measured partial recovery

Goal remains active and incomplete. Both runs use the same frozen release binary:
`f57feea6f5a7be216164678aa96a6aa9236d0de2b23e2be2e35db7cf7ee7a67c`. Both frozen source/binary terminal checks passed.
Subsequent wave10 edits are not represented by these measurements.

| Metric | Wave8 | Wave9 SDK baseline | Wave9 runtime metadata experiment |
|---|---:|---:|---:|
| Explicit bodies | 178044 | 178279 | 178550 |
| Proven enum equivalents, separate | 333 | 342 | 342 |
| Accepted code-bearing originals | 178377 | 178621 | 178892 |
| Code-bearing denominator | 194286 | 194286 | 194286 |
| Code-bearing coverage | 91.811556% | 91.937144% | 92.076629% |
| Original-form retention | 92.241736% | 92.360472% | 92.492348% |
| Java files | 31192 | 31196 | 31198 |
| Full uncapped javac diagnostics | 56715 | 55943 | 55976 |
| Export seconds | 344.152252 | 329.685170 | 550.359717 |

All 205497 original definitions remain in each ledger. Genuine retained bodyless
declarations (11177) are not bodies. SDK baseline gained 244 exact original
identities and lost zero versus wave8. Another 13723 code-bearing recoveries
are needed for 99%. Runtime metadata gained 277 and lost 6 against the same-code
SDK baseline, net +271; versus wave8 it gained 521 and lost 6. Its remaining
code-bearing gap is 13452. Every lost identity is preserved in the loss artifact.

## Source changes and bounded original evidence

- Concrete reference Phi/copy/producer proofs restore ViewComponentManager.d
  (#43723), DERBoolean.getInstance (#40557), and s$a.a (#5765). The first original
  body compiled unchanged and matched an independent opcode-order JVM reference
  across 12 modes, with explicit Android boundary fixtures.
- Effect-ordered private enum payloads recover all five WireFormat$JavaType
  originals (#13379–13383), preserving names, flags, boxed payloads, identity,
  static reads, constructor order and values-array independence.
- Exact unchecked explicit throws preserve ordered catch dispatch. Original
  Auth #20398 generates a direct complete body but stays rejected by same-class
  closure: simpleLogInPostRequest #20448 still has Unknown catch-Phi lineage.
  Direct backend success is not counted as complete published recovery.

## Verification

895 workspace tests passed, zero failed, 105 ignored. Strict workspace/all-target
Clippy, formatting and release build passed before the freeze. All 31196 SDK
files and all 31198 runtime-experiment files parse under genuine Java 25.
Full uncapped javac still fails in both configurations. No sources were excluded
or rewritten; compiler-phase hashes remained unchanged. No APK execution,
native recovery or whole-app compilation claim is made.

Baseline peak RSS: 15417468 KiB (14.70 GiB), zero exporter swaps. Runtime peak
RSS is recorded in the exact runtime time log. Timings are loaded-host single
observations, not controlled cold speedup claims. Backend task-time sum was
2069.14 seconds with SDK metadata and 4261.77 seconds with runtime metadata;
effective workers were 13.883 and 13.161. Runtime metadata loading itself took
2.832815 seconds. Further work cost cannot be attributed to loading alone.

## Runtime experiment boundaries

Explicit Android 16 / SDK 36 framework.jar uses whole-owner precedence over
compiler providers, while original APK declarations remain authoritative. APK
original-build match is unattested. DEX metadata does not provide javac class
files: compiler inputs remain genuine JDK17 / Android35. This configuration is
opt-in, not a replacement for the baseline. The six withdrawals and their exact
contracts are being audited.

## Evidence

- `/tmp/opencode/samsung-pass-99-wave9/java25` and `baseline-coverage.json`
- `/tmp/opencode/samsung-pass-99-wave9/validation25/validation.json` and `full-javac.log`
- `/tmp/opencode/samsung-pass-99-wave9/java25-runtime` and `runtime-coverage.json`
- `/tmp/opencode/samsung-pass-99-wave9/validation25-runtime/validation.json` and `full-javac.log`
- Bench copies: samsung-pass-99-wave9-coverage.json, -inventory.json,
  -validation.json, -runtime-coverage.json, -runtime-losses.json,
  -runtime-validation.json, -environment.json and terminal/time receipts.

Next original roots: captured Exception Phi lineage; Future interrupt-cleanup
precise rethrow; initialized allocation aliases retained as pending after
argument-expression capture. Scratch proposals are not counted as recovery.
Speed research is separately retained in WAVE9_SPEED_RESEARCH.md.
