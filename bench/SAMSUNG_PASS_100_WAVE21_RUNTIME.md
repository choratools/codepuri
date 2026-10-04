# Samsung Pass wave21: separate runtime metadata experiment

Frozen wave20 code and binary, unchanged APK and four compiler metadata paths; only the explicit retained device framework DEX metadata was added. Live read-only paired hashes are in `samsung-platform-live-pair-wave21.json`.

| Metric | SDK wave20 | Runtime variant |
|---|---:|---:|
| Accepted executable originals | 180914 | 181036 |
| Executable denominator | 194286 | 194286 |
| Identity gains / losses | — | 331 / 209 |
| Uncapped javac diagnostics | 52165 | 52243 |
| Export seconds | 350.57 | 518.85 |
| Export peak RSS KiB | 15613336 | 27704828 |

All 32385 generated files pass syntax parsing. Whole javac still fails. This variant is **not promoted into the SDK baseline**: metadata changes exception contracts and dependencies, and inherited runtime declarations require further resolver work. Timing is not an isolated cache comparison.

Runtime DEX declaration availability is distinct from JVM classpath availability. Current installed APK/framework pairing does not establish the historical build environment, Android execution, native recovery, or source equivalence.

Full immutable evidence: `/tmp/opencode/samsung-pass-100-wave21-runtime/`.
