# Samsung Pass speed optimization

The final measured Java export took **184.282465 seconds**, versus **256.204694 seconds** before optimization: **28.07% less wall time**, or **1.3903x throughput**. These are individual local measurements, not a statistical benchmark distribution. The first optimized measurement was 179.892124 seconds; the additional lookup changes did not establish a further whole-run improvement.

## Fixed measurement conditions

- APK: `tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk`.
- SHA-256: `d49a8bbf7aac35ec4d2e842535fd85ac105c49291d9efe23e041ae8afee12c41`.
- 205,497 original defined methods; 34,579 original classes; four DEX inputs.
- Release `java_profile`, Java17 source profile, 30 class workers, actual DirectorySink writes and ordered diagnostic checksums.
- Explicit classpath: JDK17 `java.base.jmod`, Android35 `android.jar`, JDK17 `jdk.unsupported.jmod`, JDK17 `java.naming.jmod`.
- Benchmark process was not overlapped with build/test/whole-input inventory jobs. Kernel perf sampling was unavailable; stage timers were used without changing kernel settings.

| Measurement | Before | First optimized run | Final optimized run |
|---|---:|---:|---:|
| Export wall seconds | 256.204694 | 179.892124 | 184.282465 |
| Whole process wall seconds | 257.73 | 181.37 | 185.78 |
| Peak RSS KiB | 8,286,996 | 7,909,776 | 8,029,208 |
| Generated Java files | 30,525 | 30,525 | 30,525 |
| Strict accepted original identities | 160,737 | 160,737 | 160,737 |

## Production changes

1. **Parallel source-throws prepass.** Existing ordered bounded scheduling prepares compact method requirements concurrently. It does not retain whole SSA IRs for all methods. Legacy serial and selected-root behavior remains available; exact-plan differential tests cover worker counts, cross-DEX propagation, and missing metadata.
2. **Indexed contextual locals.** One immutable full-arena index records Phi/copy consumers and definitions per method, instead of repeated instruction-arena scans. Unmodeled uses and insufficient type evidence still reject.
3. **Indexed member definitions.** Per-DEX member lookup and indexed closure avoid repeated scans through all class declarations. Legacy metadata validation and duplicate/owner checks remain enforced.
4. **Cached object construction plans.** Per-emitter CFG lookup, successful/failed allocation plans, and argument-gap ownership are reused. Dynamic pending-allocation predicates are reevaluated; scratch emitters share the same immutable planning cache.
5. **Avoid discarded inventory copies.** Field publication requests the encoded-store plan without cloning capability evidence that the caller immediately discarded. Public evidence-returning contracts remain intact.
6. **Cheaper initialization checks.** Exact original-class membership uses a binary-search fast path plus the old exhaustive fallback. A maintained structural-unknown predicate avoids filtering all unknown observations for each field when no structural observation exists. Typed diagnostics are retained whenever they apply.

These changes are in `crates/decomp/src/java_source_throws_integration.rs`, `java_integration.rs`, `java_members.rs`, `java_declaration_initializers.rs`, `java_field_publication.rs`, `write_inventory.rs`, and `crates/backend-java/src/contextual_locals.rs`, `producer_reference_locals.rs`, `object_plan.rs`, `object_arguments.rs`, with emitter construction sites and focused regressions updated.

## Bottleneck evidence

- Before, initialization-family work represented 42.03% and structure/backend work 32.14% of the summed class-task elapsed durations. These sums overlap across workers and are not whole-run wall seconds.
- Final source-throws prepass wall time is 26.452481 seconds. It is now explicitly instrumented; no exact baseline prepass time is asserted because that stage was not separately timed before the change.
- Output scheduler wall sum decreased from 178.543720 to 133.515272 seconds; effective active task workers increased from about 12.87 to 17.14.
- The formerly slow `FalconSmallPrimeList` class decreased from 40.117134 seconds to 0.829275 seconds in the final run (0.523436 in the first optimized run).
- Final initialization-family task sum is 1,010.848054 seconds, 44.17% of summed class durations. The final run does not establish a reduction in aggregate initialization work despite the improved tail and cheaper repeated lookups.
- Total user CPU seconds changed from 2,313.52 to 2,254.10. Most wall improvement comes from reducing serial/tail work and using the existing worker budget more effectively, rather than a comparable reduction in total CPU work.

## Preservation and limits

Original-identity inventories independently enumerate the input DEX definitions and map original identities to generated sources. The final ledger retains exactly the same accepted identity set: **zero gained, zero lost**. The immutable denominator remains 205,497. Accepted identities comprise 160,431 explicit complete bodies and 306 proven enum equivalents; 11,140 bodyless declarations are excluded. All prior Java file paths remain present; no new or removed paths.

**Exact source-byte parity did not pass.** SHA-256 comparison found **4,072 changed files**, with zero missing or added files, and unchanged report counters. Inspected differences include dispatcher state/label renumbering with corresponding case placement changes. This is not an exhaustive proof that every changed source is semantically equivalent. The byte comparator deliberately exits 1 and retains the changed-file inventory rather than normalizing away differences. No whole-app javac/Android execution claim is made by this performance work.

## Validation

- Main optimization set: workspace tests **659 passed, 0 failed, 37 ignored**.
- Final class-membership change: one exhaustive-vs-fast-path regression passed.
- Final inventory/initializer changes: four focused targets, **20 passed, 0 failed, 3 ignored**.
- Final strict workspace/all-target Clippy, fmt check, release profile build, and `git diff --check` passed.
- Final full-input profile exited 0. Original-identity comparison exited 0. Exact-byte comparison exited 1 for the disclosed source differences.

## Reproduction and artifacts

The complete executable command and process resource measurements are in `samsung-pass-speed-final-time.txt`. Input configuration is in `samsung-pass-speed-environment.json`.

- Before output: `/tmp/opencode/samsung-pass-speed-before`.
- First optimized output: `/tmp/opencode/samsung-pass-speed-after`.
- Final output: `/tmp/opencode/samsung-pass-speed-final`.
- Exact byte comparison: `samsung-pass-speed-final-comparison.json`.
- Exact identity comparison: `samsung-pass-speed-final-identity-comparison.json`.
- Final original-method inventory: `samsung-pass-speed-final-inventory.json` and output-root `method-coverage.jsonl`.
- Test evidence: `samsung-pass-speed-tests.txt`, `samsung-pass-speed-final-focused.txt`, `samsung-pass-speed-final-inventory-tests.txt`.
- Final checks: `samsung-pass-speed-final-{clippy,fmt,build}.txt`.

The translated output-root `report.txt` files explicitly identify their origin as `java_profile_summary`; they translate actual counter names for the existing inventory tool and do not fabricate a production export report.
