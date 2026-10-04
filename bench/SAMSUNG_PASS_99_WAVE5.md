# Samsung Pass wave5: frozen coverage and compiler checkpoint

The 99% goal remains incomplete. This export measured the original APK with
30 workers, Java source release25 and the same four explicit genuine classpath
archives as wave4. It used a frozen release executable. Subsequent cache/index
work is separate from this checkpoint.

| Original-method accounting | Count |
|---|---:|
| Original definitions | 205,497 |
| Code-bearing definitions | 194,286 |
| Explicit complete bodies | 176,045 |
| Separately proved enum equivalents | 333 |
| Genuine retained bodyless declarations | 11,177 |
| Rejected definitions | 17,942 |

Bodies plus enum equivalents total **176,378**, or **90.782661% of code-bearing
definitions** and **85.829963% of all definitions**. Original-form retention
including genuine bodyless declarations is **91.268972%**. Another **15,966**
code-bearing recoveries are needed for the99% code-bearing target.

Wave4 comparison: **1,877 gains, 753 losses**, net **+1,124**. Losses are retained
in the original-identity ledger:420 callee checked-contract failures,207 body
closure failures,31 checked throw failures,29 checked invocation failures and
other bounded contract/initialization failures. A source-only abstract throws
constraint is not permission to violate an inherited external contract.

The actual frozen export accepts CharacterReader31531, SpassFingerprint34682,
Glide24570, GMS25181, WhirlpoolDigest10292 and wu/k42632. Genuine abstract
declarations24516/25179 remain declarations, not bodies. Focused witnesses are
recorded in `SAMSUNG_PASS_99_WAVE5_PROGRESS.md`.

All **31,188 exact sources parse** under Java25. Full uncapped javac still fails:
**53,544 → 52,427 diagnostics**, with no files excluded or rewritten and every
source hash unchanged. This is an aggregate reduction, not whole-app compilation
or a controlled normalized diagnostic-rate comparison.

Export wall time was **317.581015 seconds**, peak RSS **13,072,748KiB**
(12.47GiB), source text308,669,987bytes. The process reported zero swaps and
exit0. Concurrent workspace tests and shared-host activity mean this is not a
controlled performance A/B. Source-throws preparation took33.555658s; the earlier
suspicion of270s in that stage was disproved by the terminal profiler. File
delivery took1.581622s. Analysis/source caching remains enabled.

Frozen workspace checks: **826 passed, zero failed, 77 ignored**,141 suites;
strict workspace/all-target Clippy, fmt and release build passed.

## Evidence

- `samsung-pass-99-wave5-{environment,inventory,coverage,summary}.json`
- `samsung-pass-99-wave5-{run,time,workspace-tests,clippy,fmt,validation}.txt`
- Frozen output: `/tmp/opencode/samsung-pass-99-wave5/java25`
- Exact method ledger: output-root `method-coverage.jsonl`
- Uncapped compiler logs and identity delta:
  `/tmp/opencode/samsung-pass-99-wave5/validation25`

`report-debug.txt` is the exact `report=` value captured from the profiler's
summary, not a new run. The inventory identifies its origin as
`java_profile_summary`. Whole-app compilation, native recovery and Android
execution remain unverified.
