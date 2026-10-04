# Samsung Pass five blockers — final production bounds

**Superseded:** latest post-edit production evidence is
[`samsung-pass-five-final.md`](samsung-pass-five-final.md) and
`samsung-pass-five-final*.json`. Measurements below are historical.

**Partial integration. Actual-output error reduction was not achieved.**
The immutable denominator remains **205,497 original method definitions**.

| Metric | Latest baseline | Final |
|---|---:|---:|
| Legacy strict explicit bodies | 79,340 | **80,092** |
| Proven enum equivalents | 257 | **263** |
| Genuine bodyless declarations | 11,140 | **11,140** |
| Legacy strict rejects | 114,760 | **114,002** |
| Written Java files | 24,749 | **26,209** |
| Full uncapped javac diagnostics | 38,158 | **50,966** |

Strict body/equivalence identities: **759 gained, 1 lost**. Separately,
**94,819 genuine semantic explicit bodies** are emitted: **15,480 gained, 1 lost**
against the original baseline explicit set. **14,727 ordinary semantic bodies**
remain outside legacy closure; **13,158** have directly missing same-class method
declarations. Direct same-class declaration closure is not global compilation
closure. No old coverage definition was silently changed.

## Exact five integration categories

1. **ctor/writeinv initializer:** atomic production inventory retained; exact
   direct-parent constructor-body invoke-super proof wired through genuine JVM
   declarations, original opcodes, aliases and existing access checks. Guideline3,
   Crypto2 and Biometric2 are present in actual production output. Unproved finals,
   broader super/constructor delegation and unknown write effects remain bounded.
2. **contextual_locals:** use-site/full-consumer constant typing active, with
   materialization required before literal replacement. **Atomic checked-phi
   family integration is deferred.** Captured provenance/seeds/cache remain active.
3. **dependency publication:** real ordinary callers emitted independently of old
   closure counters; actual `method_closure` and exact unresolved identities wired.
   Constructor/clinit publication stays atomic. Cross-class/global closure and
   the standalone publication adapter are not complete production paths.
4. **externalfields:** genuine exact public classpath field reads/writes active,
   original access/staticness checked, no inlining or synthetic metadata.
   **Inherited actual-owner and cross-DEX field publication are deferred.**
5. **structure_flow:** facade and backend use canonical Shared acyclic regions,
   availability intersections, original edge-specific parallel copies and typed
   returns. Forward labelled breaks preserve one placement per real block.
   Unsupported cyclic/exception/Throw paths still fail closed.

One lost original identity, **`Ld1/g;->a(I)J`**, is reproduced using original seeded
SSA and the production resolver: `contextual constant constraints:
Indeterminate(UnknownValue)` for an unmodeled numeric Cast consumer.

## Final validation

- **440 workspace tests passed; 0 failed; 12 ignored.** Workspace/all-target
  clippy `-D warnings`, full fmt check and release build pass.
- **All 26,209 exact sources parse; 0 syntax errors.** Full JDK17/Android35 javac
  fails with **50,966 diagnostics**; cap **2,147,483,647** was not reached.
- Whole unchanged Guideline compiles with all three original constructors.
  JDK17 alone compiles a real three-class emitted subset: CryptoLibException,
  BiometricTokenExpiredException and InvalidBiometricException. Exact hashes saved.
- Final Pass30: **53.705 s wall / 1,477,888 KiB RSS**, **0 errors, 0 skips**.
  Parse: **7.232 s / 1,934,248 KiB**. Javac:
  **25.527 s / 3,990,136 KiB**. Workspace tests:
  **200.127 s / 1,079,920 KiB**.
- Metadata: `/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod`, then
  `/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar`.

Top ordinary first rejections: Zero **20,673**, legacy closure **14,727**, source
field closure **13,404**, exact super target **4,058**. Constructors: unknown final
writes **19,150**, unavailable initialization **3,136**, unused parameter **1,720**.
Ordinary overlapping-region first rejections fall **10,949 -> 3,393**; newly
reachable failures move into other categories. Full attribution grows
**+12,808 diagnostics** across an expanded source/body subset; no error-reduction
claim is made.

Evidence: `bench/samsung-pass-five-blockers-{final,validation,summary,diagnostics,checks}.json`;
full details: `docs/samsung-pass-goal.md`. Final root:
`/tmp/opencode/decomphify-java-corpus/run-1790894446651331481-3421921/com.samsung.android.samsungpass`.
Logs: `/tmp/opencode/samsung-pass-five-blockers/`.
