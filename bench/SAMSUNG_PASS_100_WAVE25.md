# Samsung Pass wave25 — nested allocation and constructor shared tails

Frozen SDK-only APK, code/binary and same four ordered metadata inputs; terminal receipt verifies no changes during export. Comparison is against verified SDK wave24.

| Metric | Wave24 | Wave25 |
|---|---:|---:|
| Accepted executable originals | 180955 | 181096 |
| Executable denominator | 194286 | 194286 |
| Explicit bodies | 180557 | 180698 |
| Proven enum equivalents | 398 | 398 |
| Genuine bodyless declarations | 11183 | 11183 |
| Identity gains / losses | — | 222 / 81 |
| Source files | 32390 | 32396 |
| Uncapped javac diagnostics | 52163 | 52151 |

Executable coverage **93.211039%**; 13190 executable definitions remain. Original forms including genuine bodyless declarations: 192279 / 205497 (93.567789%). This is not whole-source compilation, APK rebuild/native decompilation or Android execution completion. All sources parse; javac still fails.

Generic nested allocation statement sequences preserve outer/inner NEW, constructor/append/call/string-resolution order, with exact uninitialized-use and handler gates. Genuine discarded nonvoid source-target proofs are required; return-only overload ambiguity is denied, private virtual/interface dispatch rejects. Actual La/b method4 is recovered. JVM/Java25 differential tests retain class-init, exception and genuine OOM ordering.

Constructor shared-terminal-tail expansion is bounded and maps temporary topology back to original blocks/edges before loop lowering. Actual Lag/b constructor14 recovers without weakening once-per-normal-path final proof, including lazy guard loads and the trimming loop. Its constructor runtime fixture uses an explicitly labelled ag.a boundary, not whole original-class execution.

**81 prior executable identities are now rejected.** 62 are dependent closure failures, 19 are independent allocation/initializer paths. The discarded-call proof and nested-plan changes need per-identity diagnosis; no loss has been silently restored or omitted. Exact transitions/reasons are in the adjacent loss receipt. Further fixes are required before calling this regression-free.

Workspace: **1005 passed, 0 failed, 140 ignored, 169 suites**. Full strict workspace/all-target Clippy, fmt and release build passed. Export **305.97 seconds**, peak RSS **16146704 KiB**. Previous321.69s had different code/load; not isolated caching A/B.

Evidence: `/tmp/opencode/samsung-pass-100-wave25`. Goal remains incomplete and active. Next evidence-backed capability: checked generic Signature and raw-ancestry proof for exact super target selection, after investigating losses.
