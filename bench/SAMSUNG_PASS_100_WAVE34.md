# Samsung Pass wave34 — accepted method-coverage progress, goal incomplete

Frozen root: `/tmp/opencode/samsung-pass-100-wave34`. Same original APK, four ordered classpaths, Java25, 30 workers. Export/input/binary/source/classpath hashes stayed unchanged. Driver and workspace are terminal exit0.

| Metric | Accepted wave29 | Wave34 |
|---|---:|---:|
| Executable original definitions | 194,286 | 194,286 |
| Accepted bodies and proven enum equivalents | 181,849 | 183,646 |
| Executable recovery | 93.598612% | 94.523537% |
| Explicit original bodies | 181,420 | 183,217 |
| Separate enum equivalents | 429 | 429 |
| Gained / lost against wave29 | — | 1,797 / 0 |

Both wave29 and stronger wave26 no-loss gates pass. All nine wave33 losses are restored; see [exact original identities](samsung-pass-wave34-restored-nine.json). Original `classes.dex #54616` constructor also changes from rejected to complete_body. Its genuine dependency compile still fails225 diagnostics; direct target diagnostics0 does not prove compilation or Android execution.

All32,420 exact sources parse. Full uncapped javac still fails **50,711 diagnostics**, cap not reached, sources unchanged. Whole APK compilation/runtime remain unproved. **1,091 workspace tests pass, zero fail,152 ignored,186 suites**; fmt, strict workspace/all-target Clippy and release pass.

Export takes **276.59s**, peak RSS **17,234,664KiB**. Previous wave33 took380.63s/17,477,060KiB. Admission window240→960; effective active workers13.712→22.347. The candidate also contains fidelity changes and both exports overlapped tests, so this is a measured combined result, not an isolated full-export attribution. The [scaled scheduler A/B](samsung-pass-wave34-scheduler-bounded.json) separately preserves34,579 ordered results and checksum.

Whole-body checked-source bridges retain original public throws clauses, exact selected target contracts and unknown runtime facts. They wrap existing statements/expressions without splitting allocation order. Synthetic helpers remain separately attributed and add reflection/live-stack observability. The six real roots emit; JDK17/25 boundary fixtures test identity/order, not complete Samsung application behavior.

Remaining: **10,640 executable originals** and **28 bodyless forms**; enum payload reconstruction, checked exception scopes, constructor/initialization proofs, source dependency closure and complete compilation remain. Native libraries/resources/assets are outside current Java recovery. Objective remains active.

Evidence: [terminal summary](samsung-pass-wave34-final.json), frozen coverage/validation/terminal receipts in the root above, `/tmp/opencode/wave34-root-workspace-final.log`.
