# Samsung Pass wave16: ordered checked catches and monitor cleanup

The monitor release/rethrow path now subtracts earlier original catch types only when the source uses native ordered catch clauses. Guarded instanceof dispatch receives no such exclusions. Catch parameters remain immutable; original lock identity and release/rethrow proof remain required.

Actual original classes2.dex method22115, SamsungPassApplicationContext.getApplicationContext(), now has complete_body status in the full production ledger. Object.wait(long) InterruptedException is handled by the earlier original handler, rather than wrongly treated as escaping the subsequent catch-all cleanup.

|Metric|Wave15|Wave16|
|---|---:|---:|
|Executable accepted (of194286)|180533|180566|
|Executable percentage|92.921260|92.938246|
|Original forms (of205497)|191716|191749|
|Original-form percentage|93.293819|93.309878|
|Sources|32386|32386|
|Uncapped javac diagnostics|53664|52958|

33 gained executable identities,0lost versuswave15;641gained/0lost versuswave13. Remaining13720code-bearing methods and28genuine bodyless forms. All32386source units pass syntax; full uncapped javac exits1 with52958diagnostics,706fewer thanwave15. No compiled APK or complete Android execution claim.

Focused native ordered wait cleanup tests passed on JDK17 and25 (2each), including real wait interruption, original Error identity and monitor release; changed handler/object/lock cases reject. Foreign undeclared checked Throwable identity is not established by that fixture. Backend suite372passed/0failed/12ignored; strict backend all-target Clippy, full formatting and release build pass. Prior full workspace972passed/0failed/129ignored is pre-wave16 and does not substitute for a post-wave16 full workspace rerun.

Frozen full run exit0; source/binary/APK/classpath unchanged. Wall338.81seconds;peakRSS15853816KiB;0swaps. This loaded-host result is not a cache acceleration benchmark. Root:/tmp/opencode/samsung-pass-100-wave16. Exact ledgers, snapshots and uncapped compiler logs retained.100% goal remains unmet.
