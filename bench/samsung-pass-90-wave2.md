# Samsung Pass recovery wave 2

Immutable denominator: 205,497 original definitions. Accepted statuses remain
complete bodies and independently proven enum equivalents only.

| Metric | Original source-closure baseline | Wave 1 | Wave 2 retry |
| --- | ---: | ---: | ---: |
| Explicit complete bodies | 89,026 | 104,219 | 121,271 |
| Proven enum equivalents | 268 | 274 | 288 |
| Accepted identities | 89,294 | 104,493 | 121,559 |
| Strict coverage | 43.45% | 50.85% | 59.1537% |
| Generated Java units | 29,398 | 29,493 | 29,576 |
| Uncapped javac diagnostics | 57,245 | 56,677 | 62,702 |

32,265 accepted identities gained relative to baseline, zero lost. The 90%
target requires another 63,389. All exact files parse; full javac fails. Body
coverage does not certify class initialization or global compilation closure.

This wave adds definition-proven Null/Zero local declarations and split uses,
per-path instance-final constructor-family body proofs, no hoisting of
uninitialized allocations, and actual Throw emission with exact type and
checked-exception evidence. Whole-input native/reflection uncertainty remains.
Real Samsung constructor source subsets compile/run; missing/double assignments
reject. Actual application and protobuf methods now reach body publication.

The first attempt panicked in the old static-initializer traversal after Throw
support exposed a zero-successor terminal. It is excluded. A focused regression
reproduces a valid ordinary throw body and confirms that unsupported linear
initializer completion returns a diagnostic, without panicking. The retry
visits all 34,579 classes with report=true, zero errors and zero skips.

Validation: backend library and focused branch/Null/constructor tests pass;
strict workspace all-target Clippy, formatting and release build pass. Exact
ledger hashes and identity deltas: `samsung-pass-90-wave2.json`; exact-source
compiler commands/hashes/outcomes: `samsung-pass-90-wave2-validation.json`.
Export wall time: 72.56 seconds; peak RSS: 3,095,056 KiB.

Successful artifact:
`/tmp/opencode/decomphify-java-corpus/run-1790928515677799756-3345608/com.samsung.android.samsungpass`

The active goal is incomplete. Next experiments preserve allocation-before-arg
effect ordering and genuine ordinary super dispatch; no gate is waived merely
to reduce the rejection count.
