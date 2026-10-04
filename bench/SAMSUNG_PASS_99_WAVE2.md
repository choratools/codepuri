# Samsung Pass 99-percent goal: measured wave2 checkpoint

The goal remains active and incomplete. This frozen release export audits the
same 205,497 original defined methods, including 194,286 code-bearing and 11,211
genuinely bodyless definitions. No placeholders or removed inputs count as recovery.

| Metric | Prior corrected wave1 | Frozen wave2 |
|---|---:|---:|
| Explicit bodies + proven enum equivalents | 168,926 | 172,587 |
| Explicit bodies | 168,628 | 172,289 |
| Proven enum equivalents | 298 | 298 |
| Genuine retained bodyless declarations | 11,177 | 11,177 |
| Rejected original definitions | 25,394 | 21,733 |
| Code-bearing body coverage | 86.947078% | 88.831413% |
| Bodies/all original definitions | 82.203633% | 83.985168% |
| Original-form retention, declarations separate | 87.642642% | 89.424177% |
| Java files | 30,596 | 31,076 |
| Full uncapped Java 25 diagnostics | 56,146 | 53,639 |

The identity comparison reports **3,745 gains and 84 losses**, net +3,661.
The losses are retained in `samsung-pass-99-wave2-delta.json`; they are being
audited for genuine correctness withdrawals versus regressions. No zero-loss
claim is made. The accepted exact super-binding witnesses include original
RecyclerView.exceptionLabel, ContentFrameLayout.onAttachedToWindow and
ActionBarContainer.drawableStateChanged. Their complete class compilation is
not established by accepting these individual bodies.

All 31,076 unchanged output files pass Java 25 syntax parsing. Whole-source
uncapped javac still fails with 53,639 diagnostics, including 43,180
cannot-find-symbol diagnostics. No source exclusions or rewrites were used;
source hashes remain unchanged across validation. This is not a compiled APK
and does not prove Android runtime equivalence.

The frozen source checkpoint passed 734 workspace tests, 0 failed, 50 ignored,
strict all-target Clippy, formatting and the release build. Those checks predate
the later null-throw, metadata inventory, enum interface and constructor suffix
changes; the current mixed worktree requires separate checks and a new export.

Export timing: 307.762656 s; wrapper process 309.805596 s; max RSS 13,058,100 KiB.
More backend support and concurrent tests differ from the previous checkpoint;
this is not a controlled cache speedup comparison. Source/analysis caches remain
enabled, while cold/warm session measurements are documented separately.

Evidence:
- `samsung-pass-99-wave2-{environment,inventory,coverage,summary,delta}.json`
- `/tmp/opencode/samsung-pass-99-wave2/java25/method-coverage.jsonl`
- `/tmp/opencode/samsung-pass-99-wave2/validation25/validation.json`
- Exact source root: `/tmp/opencode/samsung-pass-99-wave2/java25/sources`

The next measured run will integrate verified null-throw handling into every
exception renderer, genuine external metadata into symbolic field-write scoping,
and the original private enum interface and constructor/object-capture cases.
None of those later changes are attributed to this frozen export.

## Parallel speed research from this frozen run

Caches recorded 269,791 SSA hits / 194,286 misses (zero frontend failures),
and 5,380,762 member-resolution hits / 402,778 misses (122 cached unavailable
lookups). Worker frontend time was 1.291407 summed seconds because the serial
source-throws prepass already materializes the SSA cache; this is not total cold
frontend time. The prepass took 23.213865 wall seconds. Worker initialization
planning accounted for 1,164.572086 summed seconds (34.813% of class task time),
while structure/backend accounted for 1,937.222480 (57.910%). These summed worker
durations are not additive wall times. Alias building took 15.124880 serial
seconds. This ranks repeated backend/initialization work above further frontend
cache expansion; no new controlled performance gain is claimed.

Compact profile: `samsung-pass-99-wave2-profile-summary.txt`.
