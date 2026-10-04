# Samsung Pass wave29 verified aggregate

Frozen artifact: `/tmp/opencode/samsung-pass-100-wave29`. Same APK, four ordered metadata inputs, 30 workers and Java25 source validation. Export exit0; changed sources empty; APK, classpath and frozen binary unchanged.

- Executable originals: 181,849/194,286 = 93.598612%.
- Explicit complete bodies: 181,420; separately proven enum equivalents:429.
- Genuine bodyless declarations:11,183; excluded from executable denominator.
- Executable originals remaining:12,437; genuine bodyless originals remaining:28.
- Versus wave28:263 gained bodies,0 lost. Both wave28 and stronger wave26 no-loss guards exit0.
- All32,411 emitted source files pass Java25 syntax parsing. Whole-source compilation still fails (exit1); uncapped diagnostic total:51,457. This is not whole-app compilation or Android execution.
- Workspace verification:1,037 passed,0 failed,146 ignored,179 suites. Strict workspace/all-target Clippy, formatting and release build passed.

## Original identities now recovered

- classes3.dex #36597 ScreenOnController.startCheckingScreenOn: genuine classpath ConstantValue absence retains GETSTATIC SDK_INT.
- classes4.dex #31326 Entities.load: original protected range and cleanup identity retained.
- classes.dex #5892 AppCompat y.N and classes3.dex #25969 TestDataApiServiceKt.toAddress: nested child allocations execute in original order.

## Bounds and next work

100% remains unachieved. Full javac still fails; native libraries/resources/rebuilt Android execution remain unverified. Next bounded repairs are original nested suppression cleanup, reference/array Phi producer typing and constructor-prefix contextual null comparisons. JADX same-method evidence is used alongside original DEX, not as a semantic oracle.
