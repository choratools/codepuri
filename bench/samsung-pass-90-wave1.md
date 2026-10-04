# Samsung Pass recovery wave 1

The unchanged denominator is 205,497 original defined method identities.
Only ledger `complete_body` and independently `proven_enum_equivalence`
statuses count toward the active 90 percent goal. Declarations and unresolved
semantic-only publications are excluded.

| Metric | Previous source-closure baseline | Wave 1 |
| --- | ---: | ---: |
| Complete bodies | 89,026 | 104,219 |
| Proven enum equivalents | 268 | 274 |
| Accepted identities | 89,294 | 104,493 |
| Strict coverage | 43.45% | 50.8489% |
| Gained / lost identities | — | 15,199 / 0 |
| Written Java units | 29,398 | 29,493 |
| Uncapped javac diagnostics | 57,245 | 56,677 |

Every exact generated source passed the JDK17 parse gate. Full javac still
fails. Strict body coverage does not establish whole-class initialization,
globally complete dependencies, compiled APK output, or runtime parity of the
whole application.

Changes: contextual comparison typing; class literals retaining resolution
effects; typed three-way float/double/long comparisons preserving NaN polarity
and signed-zero equality; corrected exception throw classification; complete
original constructor-family body recovery for classes without instance finals.
Constructor subset recovery never attests whole-class initialization.

Focused positive/negative tests and generated-fixture runtime checks passed;
the backend, DEX and facade suites passed with explicit JDK17 environment.
Formatting, strict workspace all-target Clippy and diff checks passed.

The 30-worker export completed with zero output errors in 64.43 seconds,
maximum RSS 2,907,312 KiB, using the same explicit JDK17 java.base.jmod and
Android35 android.jar metadata. Exact identity sets and ledger hash are in
`samsung-pass-90-wave1.json`; compiler commands, source hashes and outcomes are
in `samsung-pass-90-wave1-validation.json`.

Artifact root:
`/tmp/opencode/decomphify-java-corpus/run-1790926485292369707-3124777/com.samsung.android.samsungpass`

The 90 percent target is not achieved: another 80,455 accepted original
identities are required. Next experiments address definition-proven Null local
hoisting and per-family instance-final constructor-body proofs. No stub bodies,
source exclusions, native/reflection uncertainty deletion, or whole-app success
claims were used.
