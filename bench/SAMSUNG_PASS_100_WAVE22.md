# Samsung Pass wave22 — enum identity and conditional allocation

Frozen code/binary/APK/four ordered compiler metadata inputs. Comparison is against the verified SDK-only wave20, not the separate runtime metadata experiment.

| Metric | Wave20 | Wave22 |
|---|---:|---:|
| Accepted executable originals | 180914 | 180928 |
| Executable denominator | 194286 | 194286 |
| Explicit complete bodies | 180520 | 180530 |
| Proven enum equivalents | 394 | 398 |
| Genuine bodyless declarations | 11183 | 11183 |
| Identity gains / losses | — | 14 / 0 |
| Java sources | 32388 | 32389 |
| Uncapped javac diagnostics | 52165 | 52147 |

Executable coverage: **93.124569%**. Remaining executable originals: **13358**. All sources pass syntax parsing; whole javac still fails. No APK rebuild/native/Android execution success claim.

CommandEnum hidden name/ordinal use is proved by original SSA identity rather than token spelling, recovering nine original identities. Its unchanged whole emitted unit compiled and ran on JDK17; D8/JVM differential retains payloads, order, clone behavior and private/final metadata.

Generic conditional allocation preserves one same-class allocation with a pure already-materialized null selector and two original constructor arms. It recovered MediaBrowserCompat$d #2803 and four additional original identities. JVM/JDK25 comparisons exercised constructor failure, class initialization failure identity and genuine OOM; effectful selectors, escape and handler shapes still reject.

Workspace: 990 passed, 0 failed, 136 ignored, 164 suites. Full strict workspace/all-target Clippy and fmt passed. Export wall **394.44 seconds**, peak RSS **15579984 KiB**. Concurrent test load and changed code make this timing unsuitable as an isolated cache performance comparison.

Evidence root: `/tmp/opencode/samsung-pass-100-wave22`. Exact coverage, terminal immutability, validation and test receipts are adjacent JSON files.
