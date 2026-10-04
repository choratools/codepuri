# Samsung Pass 99-percent goal: verified wave4 checkpoint

The full frozen export and original-identity inventory finished. The active 99%
goal remains incomplete; no placeholder or denominator change is counted.

| Original-method accounting | Count |
|---|---:|
| Original definitions | 205,497 |
| Code-bearing definitions | 194,286 |
| Explicit complete bodies | 174,929 |
| Separately proved enum equivalents | 325 |
| Genuine retained bodyless declarations | 11,177 |
| Rejected definitions | 19,066 |

Bodies plus enum equivalents total **175,254**, or **90.204132% of code-bearing
definitions** and 85.282997% of all original definitions. Original-form retention
including genuine bodyless declarations is **90.722006%**. The code-bearing 99%
target still requires 17,090 additional recoveries. Bodyless declarations are
not recovered bodies.

Exact wave3 comparison: **1,583 gained, 461 lost**, net **+1,122**. The loss list
is retained, not omitted: 332 callee checked-contract failures, 100 dependency
closure failures, 12 initializer checked contracts and 17 other checked/backend
boundaries. Original metadata and source-contract correctness must be examined
before restoring any withdrawn acceptance.

All **31,184 exact source files parse** under Java 25. Whole-source uncapped
javac still fails: **53,291 → 53,544 diagnostics**. No files were excluded or
rewritten, and source hashes remained unchanged throughout validation. Increased
coverage is not whole-class compilation or runtime-equivalence proof.

Export: **300.582954 seconds**, peak RSS **13,639,132 KiB**, 304,301,179 source
text bytes. Concurrent shared-host work means this is not controlled speedup
evidence. Frozen workspace checks: **808 passed, zero failed, 70 ignored**;
strict all-target Clippy, formatting and release build passed. Subsequent wave5
edits are not included in these measurements.

## Evidence

- `samsung-pass-99-wave4-{environment,inventory,coverage,summary}.json`
- `samsung-pass-99-wave4-{run,time,workspace-tests,clippy,fmt,validation}.txt`
- Frozen export: `/tmp/opencode/samsung-pass-99-wave4/java25`
- Exact all-definition ledger: export-root `method-coverage.jsonl`
- Full uncapped compiler logs and identity delta:
  `/tmp/opencode/samsung-pass-99-wave4/validation25`

Whole-app compilation, Android execution and native decompilation remain
unverified. The 99% goal remains active.
