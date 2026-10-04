# Samsung Pass wave27 — measured regression, repair required

Frozen export: same original APK, four ordered SDK/JDK metadata inputs, 30 workers and Java25. Terminal receipts confirm unchanged sources, binary, APK and classpath.

| Metric | Wave26 | Wave27 |
|---|---:|---:|
| Accepted executable originals | 181282 | 180439 |
| Executable denominator | 194286 | 194286 |
| Explicit bodies | 180884 | 180014 |
| Proven enum equivalents | 398 | 425 |
| Original bodyless declarations, separate | 11183 | 11183 |
| Gained/lost executable identities | 186/0 | 197/1040 |
| Source files | 32396 | 32404 |
| Uncapped full javac diagnostics | 52127 | 54088 |

Executable coverage 92.872878%; 13847 executable originals remain unavailable. This is a regression, not an accepted improvement or completion. Workspace final gates: 1022 passed, 0 failed, 144 ignored; strict clippy, formatting and release passed. All 32404 sources parse; full javac fails. Export305.52seconds, peakRSS16390852KiB; not an isolated caching A/B.

Gains include actual SeslCheckedTextView constructor7112 plus three wrappers, Boolean int-word cast sites12446/6765, and private enum omitted primitive defaults. Exact SavedState and genuine Creator compile unchanged; SeslCheckedTextView full unit still has genuine absent dependency/member errors.

The selected-source checked-contract correctness fix exposed incomplete integration for Special calls: constructor/private emitted contracts must be proven without reverting to symbolic Super metadata. 410 lost identities are downstream closure removals; 133 directly report unresolved native protected source contract, with most remaining roots lacking Special cleanup contracts. These are queued for immediate repair. Preserve exact catch priority, prethrow state, allocation/init effects and original identities; do not relax contracts or hide losses.

Evidence: `/tmp/opencode/samsung-pass-100-wave27`. Full 100% goal remains active and incomplete; whole-source compilation, native libraries, APK rebuild and Android execution are not established.
