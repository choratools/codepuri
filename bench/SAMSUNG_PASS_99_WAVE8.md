# Samsung Pass wave8: entry-flow correctness and measured recovery

The active 99% goal remains incomplete. This is the frozen wave8 full APK
checkpoint, before the subsequent wave9 edits and external DEX metadata adapter.

| Original accounting | Count |
|---|---:|
| Original defined methods | 205,497 |
| Code-bearing definitions | 194,286 |
| Explicit complete emitted bodies | 178,044 |
| Separately proven enum equivalents | 333 |
| Genuine retained bodyless declarations | 11,177 |
| Rejected definitions | 15,943 |

Bodies/equivalents total **178,377**, **91.811556%** of code-bearing originals.
Bodies/all definitions:86.802727%; original-form retention:92.241736%.
Another **13,967** code-bearing recoveries are needed for99%. Genuine bodyless
declarations are never counted as recovered bodies.

Against the exact wave7 ledger: **166 gained,29 lost,net+137**. The unchanged
205,497 identities and every loss are retained. Fifteen losses are dependency
closure failures; direct type/Phi,checked-contract and access failures are also
recorded. These withdrawals are not all assumed justified: bounded original
SSA witnesses are being investigated separately in wave9.

## Integrated fixes and proof limits

- CFG merging preserves the original entry chain and its first payload. An
  invocation preheader gives entry loops separate initial and backedge Phi
  inputs. ViewPager21016 and ArrayEncoder5433 original witnesses pass; portable
  Java executes original parent traversal and prethrow exception identities.
- Retained unconditional entry instructions require exact agreement with their
  genuine CFG edge before linear rendering. A valid graph with a disagreeing
  encoded branch still rejects.
- Contextual family declarations respect raw consumer roles. Original39055's
  offset533 remains an int array index. Its unchanged88,158-byte body matches
  the frozen previous source across28 JVM boundary modes. This is not Android
  execution of the original DEX.
- Shared typed catches retain per-edge catch types and ordering, without a
  fabricated union nominal type. ReflectUtil23703 now proves its source contract.
  Missing Samsung framework declarations still block dependent source plans.
- Original in-range copies in protected allocation chains now carry the original
  value. Auth20398 proves nine chains, but its wave8 body remains unavailable
  because of checked-catch emission. A local chain proof is not a recovered body.

## Whole export validation

All **31,192** exact source files pass Java25 parsing, with zero syntax errors.
No files were added or removed versus wave7;5,316 changed. Full uncapped javac
**still fails:56,705→56,715 diagnostics**. Five unchanged representative units
compile. Source hashes remain unchanged through compiler validation. No files
were excluded or rewritten to reduce errors.

The frozen source passed **880 tests,zero failed,99 ignored**,149 suites; strict
workspace/all-target Clippy,fmt and release build passed. These checks precede
wave9 edits and do not attest that newer edits pass all the same gates.

The30-worker export took **344.152252s**,346.29s whole process, peak15,965,848KiB
(~15.23GiB),zero swaps,exit0. Concurrent tests/host activity make this an
uncontrolled cold timing, not a measured speedup. SSA269,358hits/194,286misses;
member resolution5,624,381hits/403,703misses. The separately measured same-session
source replay remains1.51s; it is not this cold export's time.

## Evidence and continuing work

- `samsung-pass-99-wave8-{environment,inventory,coverage,summary}.json`
- `samsung-pass-99-wave8-{workspace-counts,lost-identities,focused}.json`
- `samsung-pass-99-wave8-{compiler-summary,diagnostic-delta,source-delta}.json`
- `samsung-pass-99-wave8-{workspace-tests,fmt,clippy,build,run,time,stderr}.txt`
- Frozen output:`/tmp/opencode/samsung-pass-99-wave8/java25`
- Full compiler/hash/identity evidence:`/tmp/opencode/samsung-pass-99-wave8/validation25`

Wave9 separately works on guarded unchecked-throw dispatch, actual external DEX
declarations, original lost type/SSA roots and private enum effect-ordered payloads.
World-readable Samsung framework metadata has genuine declaration evidence, but
its Android16 build is not attested to match the APK's original platform. Runtime
DEX declarations are not javac classfiles. Publicized enum constructors remain
an explicit source-modifier incompatibility; their flags are not silently changed.
No wave9 aggregate coverage, whole APK compilation, native-library recovery or
Android runtime completion is claimed here.
