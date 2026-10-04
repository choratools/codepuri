# Samsung Pass 99% goal: Java release wave 1

The corrected Java 25 export recovered **168,926 original code-bearing
definitions**: 168,628 explicit bodies and 298 proven enum equivalents. This is
82.203633% of all 205,497 original definitions, or 86.947078% of the 194,286
original code-bearing definitions. The active 99% goal remains incomplete.

Samsung Pass 5.2.10.10, version code 521010900, has unchanged APK SHA-256
`d49a8bbf7aac35ec4d2e842535fd85ac105c49291d9efe23e041ae8afee12c41`.
All runs used four DEX files, 30 workers and the recorded JDK17/Android35 metadata
classpath. Java 17 remains the default; the additional constructor recovery
requires explicit `--source-release 25` and Java 25 source validation.

| Completed export | Explicit bodies | Enum equivalents | Genuine declarations | Rejected | Source files | Export seconds |
|---|---:|---:|---:|---:|---:|---:|
| Java 17 | 162,923 | 298 | 11,177 | 31,099 | 30,558 | 289.780865 |
| Initial Java 25 | 168,615 | 298 | 11,177 | 25,407 | 30,596 | 288.115248 |
| Corrected Java 25 | 168,628 | 298 | 11,177 | 25,394 | 30,596 | 276.465616 |

The first release pair used the same frozen executable. The corrected run used
a new executable with an independent static-initializer fallback after full
family closure failed; its complete subprocess took 278.359199 seconds and
exited successfully. These are single cold export measurements with differing
behavior, not a controlled cold speedup claim.

Compared with the prior completed session baseline, corrected Java 25 gained
7,498 accepted original identities and lost none: 4,693 constructors, 2,777
ordinary methods and 28 static initializers. It retained 37 additional genuine
bodyless declarations. Another 121 existing enum equivalents changed to explicit
bodies; they are not counted again as new accepted identities.

The initial Java 25 run lost `Ltu/j;-><clinit>()V`. The corrected ledger restores
it as `complete_body` in `classes4.dex`, method index 41993, source
`tu/j$type.java`. Corrected versus initial Java 25 gained 13 static initializers
and lost none: this restoration plus 12 additional baseline gains.

Original identity, DEX slot, method index, access flags and code presence remain
unchanged across the ledgers. All 11,177 retained declarations were originally
code-less abstract/native definitions. They are not complete bodies. Including
these genuine original forms gives 180,103 /205,497 =87.642642%; this separate
accounting view does not redefine the goal. At 99%, the remaining counts are
34,517 for the historical strict/all-definition view, 23,418 for the code-bearing
view and 23,340 for genuine original forms. Bodies alone have a 94.544446%
ceiling against all definitions because 11,211 originals have no code.

All three source validations have finished. Every exact source file passed the
syntax gate; full uncapped javac compilation failed in each run:

| Validation | Parsed source files | Syntax exit | Full javac exit | Diagnostics |
|---|---:|---:|---:|---:|
| Java 17 | 30,558 | 0 | 1 | 59,784 |
| Initial Java 25 | 30,596 | 0 | 1 | 56,138 |
| Corrected Java 25 | 30,596 | 0 | 1 | 56,146 |

The diagnostic cap of 2,147,483,647 was not reached, and source hashes remained
unchanged during validation. No whole-app compilation, compiled APK, device
execution or runtime-equivalence result is established.

The pre-static-fix workspace run passed 705 tests. Subsequent focused checks
passed two static regressions, two actual Java 25 constructor witnesses and six
fresh release session-cache tests, including Samsung Fragment initialization
and replay. Parent-reported decomp all-target strict Clippy also passed; the
705-test run is not presented as a post-fix whole-workspace rerun.

The earlier 189.117 →1.436 second cache result measured immutable source replay
within one process. It does not measure cold acceleration of this recovery
wave. See [the session cache report](SAMSUNG_PASS_SOURCE_SESSION.md).

Machine-readable evidence and exact artifact paths are in
[the wave summary](samsung-pass-99-wave1-summary.json). Coverage and inventory
are retained in `samsung-pass-99-wave1-{java17,java25,corrected}-*.json`, with
commands, hashes, build and timing logs under the same prefix. Terminal source
validation JSONs are under
`/tmp/opencode/samsung-pass-99-java-release-wave1/{validation17,validation25,validation25-corrected}/validation.json`.
