# Static initializer object-plan reuse

## Finding

Samsung Notes' `SpenInputEmojiList` was the longest class task: roughly 41 s
in the initial full profile. Static initialization first validates the IR with
the ordinary method emitter, then renders it with blank-final assignment rules.
Both passes independently built object allocation/argument plans. Focused
instrumentation measured about 17 s per pass building argument-owner evidence
for 1,305 allocation plans. Original static-final path proof took only 0.005 s.

## Change

Reuse the ordinary emitter's immutable object-plan evidence for the static
renderer, including block lookup, successful/rejected allocation plans,
conditional/region plans, sequence membership and argument owners. The static
renderer retains its own value/materialization state and its original
pure-expression policy. Validation, final-field legality checks and source
rendering remain in place.

## Measurements

Same installed APKs, Android 35 `android.jar`, 30 workers, fresh output roots.
These are individual runs, not statistical medians.

| Measurement | Before | After |
|---|---:|---:|
| Notes focused class export (whole-input preparation included), no I/O | 85.69 s | 67.86 s |
| Notes full profiled export, no I/O | 112.33 s | 96.48 s |
| Notes full CLI, file output | 118.69 s | 107.44 s |
| Wallet full profiled export, no I/O | 119.04 s | 118.67 s |

The focused class's ordered checksum remained `e8d3dc6151968364`, with
722,866 output bytes. Notes' 66,345 classes, 351,587 complete ordinary bodies,
45,397 constructors, 12,660 static initializers and 36,758 unsupported ordinary
methods were unchanged. Wallet's corresponding counters were also unchanged.

Comparing all 66,345 Notes CLI Java files with the previous paired baseline
found two files with comment-only differences and no non-comment source
differences. Full-output checksums differ because of these diagnostic comments;
the output is not claimed to be byte-identical across all files.

## Validation

- Backend library tests with explicit JDK 17: 367 passed, 14 ignored.
- New regression checks cache reuse without sharing materialized value state or
  ordinary pure-expression policy.
- `cargo clippy -p backend-java -p decomp --all-targets -- -D warnings` passed.
- Release CLI and profiling example builds passed.

Local measurement artifacts: `/tmp/opencode/adb-major10/profile-investigation/`.
The APK corpus is local and is not committed.
