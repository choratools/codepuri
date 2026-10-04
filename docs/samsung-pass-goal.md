# Latest verified aggregate: wave29

Executable recovery **93.598612% (181,849/194,286)**; +263 /0 lost versus wave28. All prior wave26 accepted originals retained. Whole javac remains failed; goal100% is active. Evidence: `bench/SAMSUNG_PASS_100_WAVE29.md`.

## Latest verified wave28: 93.463245%, one regression remains

181586/194286 executable originals;305gains/1loss versus strong wave26 baseline. Previous wave27 losses restored1039/1040. All32408sources parse; full javac51688errors. No-regression gate correctly fails. See `bench/SAMSUNG_PASS_100_WAVE28.md`. Full100 goal is active and incomplete.

## Latest verified wave27: regression under repair

Frozen wave27 executable coverage **92.872878%** (180439/194286), 197 gains and 1040 losses versus wave26. Full100 goal remains incomplete; do not present this wave as improvement. See `bench/SAMSUNG_PASS_100_WAVE27.md`. Source-contract Special integration repair is underway; previous wave26 remains the stronger measured baseline.

## Latest verified SDK baseline: wave26

Executable originals **181282 / 194286 (93.306775%)**, **186 gained / 0 lost** vs wave25.
All81 prior wave25 losses restored. Explicit bodies180884; enum equivalents398;
genuine bodyless declarations11183 remain separate. Remaining13004 executable+28
bodyless. All32396 sources parse; full uncapped javac stillfails52127diagnostics.
See `bench/SAMSUNG_PASS_100_WAVE26.md` and exact frozen receipts. Actual7112
constructor remains unavailable; zero-prefix and field/super cleanup work follows.
Whole100% remains active and incomplete.

## Latest verified SDK baseline: wave25

Executable originals **181096 / 194286 (93.211039%)**, 222 gained and 81 lost vs wave24; net141. Explicit bodies180698; enum equivalents398; genuine bodyless declarations11183 separate. Remaining13190 executable +28 bodyless. All32396sources parse; whole javac stillfails52151diagnostics. Losses are under investigation, not hidden. See `bench/SAMSUNG_PASS_100_WAVE25.md` and exact loss receipts. Whole100% remains incomplete.

## Latest verified SDK baseline: wave24

Executable originals **180955 / 194286 (93.138466%)**, 27 gained and zero lost vs wave22. Explicit bodies 180557; enum equivalents 398; genuine bodyless declarations 11183 remain separate. Remaining executable originals: 13331. All 32390 sources parse; whole javac fails with 52163 diagnostics. See `bench/SAMSUNG_PASS_100_WAVE24.md`. Runtime wave23 and selected-module compiler experiments are separate, unpromoted contexts.

## Latest verified SDK baseline: wave22

Executable originals: **180928 / 194286 (93.124569%)**, 14 gained and zero lost vs wave20. Explicit bodies 180530; enum equivalents 398; bodyless declarations 11183 are counted separately. Remaining executable originals: 13358. Whole javac still fails. See `bench/SAMSUNG_PASS_100_WAVE22.md` and adjacent immutable receipts. Runtime metadata wave21 is a separate unpromoted experiment.

## Latest verified aggregate: Samsung Pass wave20 (2026-10-04)

Executable originals180914/194286 (**93.117363%**), originalforms192097/205497 (**93.479224%**). Noidentitygainsorlossesversusverifiedwave18; failedwave19generalizationrecovered.24earlierwave16identitiesrequirecorrectcheckedcallercontracts andarenotblindlyrestored. All32388sourcesparse; whole-sourcejavacstillfailswith52165diagnostics. Remaining13372executable+28bodyless.100% remainsactive,incomplete. See [wave20 evidence](../bench/SAMSUNG_PASS_100_WAVE20.md).

## Latest verified aggregate: Samsung Pass wave18 (2026-10-04)

Executable originals180914/194286 (**93.117363%**), original forms192097/205497 (**93.479224%**).5gained/0lost versuswave17;24 identities lost inwave17 remain unresolved versuswave16. All32388sources parse; whole-source compiler fails with52165diagnostics. Remaining13372executable +28bodyless.100% remains active and incomplete. See [wave18 evidence](../bench/SAMSUNG_PASS_100_WAVE18.md).

## Latest verified aggregate: Samsung Pass wave16 (2026-10-03)

Executable originals180566/194286 (**92.938246%**); original forms191749/205497 (**93.309878%**).33gained/0lost versuswave15;641gained/0lost versuswave13.13720executable+28bodyless remain. Actual getApplicationContext() now survives production output. All32386files parse; whole-source javac stillfails with52958diagnostics (706fewer).100% remains unmet. See [wave16 evidence](../bench/SAMSUNG_PASS_100_WAVE16.md).

## Latest verified aggregate: Samsung Pass wave15 (2026-10-03)

Executable originals180533/194286(**92.921260%**); original forms191716/205497(**93.293819%**).106gained/0lost versuswave14;608gained/0lost versuswave13.13753executable+28bodyless remain. All32386source files parse. Uncapped whole-source Java25 compiler stillfails with53664diagnostics;100% is not achieved. Original six nullable-mask constructors now survive production output. See [wave15 evidence](../bench/SAMSUNG_PASS_100_WAVE15.md).

## Latest verified checkpoint: Samsung Pass wave14 (2026-10-03)

Executable originals:180427/194286 (**92.866702%**). Original forms including genuine bodyless declarations:191610/205497 (**93.242237%**). Exactly502gained/0lost versuswave13.13859executable +28bodyless remain. All32386 sources parse; full Java25 compilation stillfails with53559diagnostics.100% goal remains incomplete. See [wave14 report](../bench/SAMSUNG_PASS_100_WAVE14.md) and exact receipts. Focused mask constructor proofs do not yet satisfy production delegation metadata and receive no aggregate credit.

# Samsung Pass static decompiler goal — measured partial progress

## Active user target: 100% (raised from 99%)

The user raised the target to 100% on 2026-10-03. Earlier 99% checkpoints below
are historical milestones, not the final completion gate. Keep the original
205,497 definitions and 194,286 code-bearing methods unchanged; genuine original
abstract/native declarations remain separate from recovered executable bodies.
No placeholder, fallback, dropped original identity, invented member or relaxed
proof is accepted as a recovery. Missing dependencies and semantic/runtime scope
remain explicit rather than being hidden by successful export or syntax checks.

The latest frozen full checkpoint is wave13:179925 accepted code-bearing originals
(92.608320%), including179580 explicit bodies and345 proven enum equivalents.
14361 executable originals remain to100%.11183 of11211 genuine bodyless
original declarations are retained;28 remain missing. Original-form retention is
191108/205497 (92.997951%). All31234 output files parse; full uncapped Java25
javac still fails with55037 diagnostics (wave12:56434). Whole APK/Android/native
completion remains unverified. Versus wave12:13 gained,3 lost, net10 executable
identities;6 genuine declarations additionally retained. The three lost wu/f
identities fail checked-catch reachability and remain an explicit next repair.

Evidence: `bench/SAMSUNG_PASS_100_WAVE13.md`, machine-readable wave13 artifacts,
and `/tmp/opencode/samsung-pass-100-wave13`. Export/binary/source/input/classpath
freeze receipts are terminal and unchanged. Workspace952 tests passed,0failed,
124ignored. Focused original enum and original-method JVM boundary tests passed
separately; they do not establish whole original APK execution.

## Latest measured wave10 SDK baseline: 91.965453%

Frozen full export:178334 explicit bodies +342 separately proved enum equivalents =178676 accepted code-bearing originals out of194286. Exactly55 gained and0 lost versus wave9;13668 more code-bearing recoveries are needed for99%. All205497 definitions remain in the ledger. Genuine retained bodyless declarations11177 are separate; original-form retention is92.387237%.

All31196 sources parse; full uncapped javac still fails with55942 diagnostics. Export321.103s; peak RSS14.36GiB;0 swaps. Workspace913 tests passed; final backend193 tests passed, overlapping counts. Source/binary/APK/classpath freeze receipts passed. No whole-APK/native/Android equivalence claim.

Evidence: [wave10 report](../bench/SAMSUNG_PASS_99_WAVE10.md), `bench/samsung-pass-99-wave10-summary.json`, `/tmp/opencode/samsung-pass-99-wave10`. The99% goal remains active and incomplete.


## Latest verified 99-percent checkpoint: wave9 baseline

Full frozen APK:178279 explicit bodies +342 separately proven enum equivalents
=178621 accepted code-bearing originals, **91.937144%** of 194286. Original
definitions 205497 and genuine retained bodyless declarations 11177 remain
separate. Original-form retention 92.360472%. Another 13723 code-bearing
recoveries are needed for 99%; the active goal remains incomplete.

Wave 8 delta:244 gained,zero lost. All 31196 exact sources parse underJava 25.
Full uncapped javac still fails but diagnostics decreased 56715→55943.
895 tests passed,zero failed,105 ignored; strictClippy/fmt/release pass.
Export 329.685170 s,peak 14.70 GiB; no cold speedup claimed.
Explicit Samsung runtime-DEX experiment recovered 178892 code-bearing originals
(92.076629%), net+271 versus same-code SDK baseline (277 gains,6 losses).
Its Android 16/SDK 36 original-build match is unattested, compiler sources remain
Android 35, and uncapped javac still fails 55976 diagnostics. All 31198 files parse.
It is opt-in and not substituted for the SDK baseline;99% remains incomplete. See [wave9 evidence](../bench/SAMSUNG_PASS_99_WAVE9.md).

## Historical 99-percent checkpoint: wave8

Frozen full APK:178,044 explicit bodies +333 proven enum equivalents =178,377
accepted code-bearing originals, **91.811556%** of194,286 code-bearing methods.
The unchanged all-definition denominator is205,497. Genuine retained bodyless
declarations11,177 are separate. Another **13,967** code-bearing recoveries are
needed for99%; the active goal remains incomplete.

Exact wave7 delta:166 gained,29 lost,net+137. All losses are recorded and being
investigated, rather than assumed safe. All31,192 exact sources parse underJava25;
no paths added/removed. Full uncapped javac still fails:56,705→56,715 diagnostics.
Frozen checks:880 passed,zero failed,99 ignored; full strictClippy/fmt/release pass.
Export344.152252s,peak15.23GiB. Subsequent wave9 changes are not this aggregate.
See [wave8 evidence](../bench/SAMSUNG_PASS_99_WAVE8.md).

## Historical 99-percent checkpoint: wave7

Frozen full APK:177,907 explicit bodies +333 proven enum equivalents =178,240
accepted code-bearing originals, **91.741042%** of194,286 code-bearing methods.
The unchanged all-definition denominator is205,497. Bodies/all86.736059%;
11,177 genuine bodyless declarations separately bring original-form retention to
92.175068%. **14,104** additional code-bearing recoveries are needed for99%.
The active goal remains incomplete.

Exact baseline delta:1,942 gained,80 lost, net+1,862; losses are retained and
being investigated, including one concrete Boolean declaration regression.
31,192 output files parse underJava25. Five added files and one removed file
are reported explicitly. Full uncapped javac still fails:52,427→56,705 diagnostics,
with no source exclusion/rewrite and output hashes unchanged. Frozen checks:
856 passed, zero failed,90 ignored; full strictClippy/fmt/release passed.
Export313.853576s, peak14.97GiB. Current wave8 source edits are NOT this aggregate
measurement. See [wave7 evidence](../bench/SAMSUNG_PASS_99_WAVE7.md).

## Historical 99-percent checkpoint: wave5

Frozen wave5 recovered **176,045 explicit bodies + 333 proven enum equivalents**
out of unchanged 205,497 original definitions: **85.829963% body/all** and
**90.782661% code-bearing body coverage**. The separate 11,177 genuine original
bodyless declarations bring original-form retention to **91.268972%**.
The 99% goal remains active and incomplete; bodyless declarations are not bodies.

Exact comparison to wave4: **1,877 gains, 753 losses**, net +1,124. All 31,188
unchanged source files parse under Java 25; whole-source uncapped javac still
fails: **53,544 → 52,427 diagnostics**. No source exclusion or rewrite was used.
Frozen-baseline workspace checks:826 passed, zero failed,77 ignored. Subsequent
performance cache/index edits are not part of this aggregate measurement.

Details: [wave5 evidence](../bench/SAMSUNG_PASS_99_WAVE5.md).

Latest cache-only wave6 preserves every one of205,497 original method statuses:
zero body gains/losses. Its full same-session benchmark measured350.646405s for
first render and1.507423s for cached replay, with all31,188 file hashes and reports
identical. Peak process RSS12.26GiB;829 workspace tests passed. The new original
input inheritance index's isolated linkage stage measured3.108213→0.316701s with
exact all-abstract vector parity. This does not establish faster whole cold export
or advance the recovery percentage. See
[current cache evidence](../bench/SAMSUNG_PASS_CACHE_WAVE6.md).

## Active objective and cache checkpoint

The active user goal is now **at least 99% recovery**, with speed research
continuing alongside coverage work. It remains incomplete. The older 90% targets
below are historical checkpoints, not the current completion criterion.

The matched memory-cache exports audited all 205,497 original definitions:
160,431 explicit bodies, 332 proven enum equivalents, 11,140 genuine bodyless
declarations and 33,594 rejects. Inventory cache ON/OFF changed no method status
or accepted identity. Both exported 30,530 units. The frozen benchmark executable
predates the newly integrated attested static-initializer path; its counts must
not be presented as a full export of subsequent production edits.

Original DEX has 194,286 code-bearing definitions and 11,211 genuinely bodyless
abstract/native declarations. Consequently, bodies divided by all definitions
have a 94.54445% ceiling. Body recovery for code-bearing definitions and retention
of genuine original declarations are separate accounting views; changing a
denominator or emitting placeholders does not establish goal completion.

Caching is enabled by default for seeded SSA, immutable member resolution and
shared inventory evidence. One controlled inventory-only pair measured
183.848050 → 179.400434 seconds, about 9.32 GiB peak exporter RSS. This is a single
pair, not a statistically established speedup. Full source-byte parity did not
pass; exact original identity/status preservation did.
Evidence: [memory-cache report](../bench/SAMSUNG_PASS_MEMORY_CACHES.md).

Attested static proof is now wired to the actual cached SSA. Two original
initializers compiled and ran in focused tests. Constructor-prelude, exception
dispatch and enum-family helpers have focused validation, but their new facade
routes and a fresh complete coverage export remain pending. Public or differently
named enum shapes remain explicit residuals instead of silently changing access
or runtime Enum.name behavior. Whole-app compilation and Android execution are
not established.

## Historical measured wave23: 78.13253 percent; validation complete

160,254 strict bodies +306 enum equivalents =160,560 /205,497.
24,388 more accepted original identities needed for90 percent.
Wave22 delta3,628gained,13static-initializer withdrawals under independent-proof repair.
All30,524sourcesparse;uncappedjavacstillfails with60,854diagnostics.
636workspace tests,strictClippy/fmt/releasebuildpass. Export272.304sec,errors/skipszero.
ActualPassOperation.process isretained;wholeunitcompilation/runtimeunproved.
Coverage isnotwhole-appcompilation/runtime. Goalactive.
Evidence: bench/samsung-pass-90-wave23.md/.json.

## Historical measured wave22: 76.37338 percent; validation complete

156,639 strict bodies +306 enum equivalents =156,945 /205,497.
28,003 more accepted original identities needed for90 percent.
Wave21 delta418gained,zero lost.30,507filesparse;uncappedjavacstillfails
with60,809diagnostics. Export270.726sec,errors/skipszero.
PassOperation.process advances fromallocationbarrier toObject/String typeproof,
not yet recovered. Mixed-source late doctest requiresrerun aftercurrentfreeze.
Coverage isnotwhole-appcompilation/runtime. Goalactive.
Evidence: bench/samsung-pass-90-wave22.md/.json.

## Historical measured wave21: 76.16997 percent; compile validation complete

156,221 strict bodies +306 proven enum equivalents =156,527 /205,497.
28,421 more accepted original identities required for90 percent.
Wave20 delta:75 gained,zero lost. Both corrected g9 constructor Phi paths
and their downstream caller are recovered without restoring the incorrect source.
All34,579classes visited;30,500files,export errors/skips zero.
Full30 export249.994seconds; all30,500filesparse. Uncappedjavacfails
with60,755diagnostics.604testcasespass; concurrent next-wave edits caused
a late doctest source/rlib mismatch,rerun pending after freeze.
Coverage is not a whole-app compilation/runtime claim.
Evidence: bench/samsung-pass-90-wave21*.json/.txt.

## Historical measured wave20: 76.13347 percent; validation complete

156,146 strict bodies +306 proven enum equivalents =156,452 /205,497.
28,496 more accepted original identities required for90 percent.
Wave19 delta:880gained,threecorrectness withdrawals. g9/g andg9/j constructors
previously lost a conditional null input; correctedSSA adds the requiredPhi,
nowpending suffixlowering. Downstreamg9/g.m isclosurewithdrawal.
599workspace tests, strictClippy/fmt/buildpass. All34,579classes visited;
30,500syntax-validsources,exporterrors/skipszero. Uncappedjavacstillfails
with60719diagnostics. Full30:243.315seconds,peakRSS7,844,716KiB;
sharedhost/concurrentvalidation,no controlledspeedupclaim.
Coverage isnotwhole-appcompilation/runtime. Goalactive/incomplete.
Evidence: bench/samsung-pass-90-wave20*.json/.md.

## Historical measured wave19: 75.70670 percent; validation complete

155,269 strict bodies + 306 proven enum equivalents = 155,575 / 205,497.
29,373 more accepted original identities are required for 90 percent.
Wave18 identity delta: 544 gained, zero lost; all six earlier withdrawals restored.
589 workspace tests, strict all-target Clippy/fmt/releasebuild passed.
All 34,579 classes visited; 30,498 files, all syntax-valid; exporterrors/skipszero.
Uncapped javac fails with 60816 diagnostics. Full30:249.257seconds,
peakRSS8,078,044KiB; concurrent validation/sharedhost, no controlled speedup claim.
Original HijrahDate examples and p9/a constructor are retained. PassOperation
still rejects on allocation barrier after shared-handler recovery.
Goal active/incomplete. Evidence: bench/samsung-pass-90-wave19*.json/.md.

## Historical measured wave18: 75.44198 percent; validation complete

154,725 strict bodies + 306 proven enum equivalents = 155,031 / 205,497.
29,917 more accepted original identities are required for 90 percent.
Wave17 identity delta: 747 gained, six lost; withdrawals under audit. Class-literal allocation arguments
and exact static pre-constructor arguments are connected. 579 workspace tests,
strict Clippy and fmt passed for the frozen release. All 34,579 classes visited,
30,496 files, export errors/skips zero. Execution was under GDB with brief profiling
pauses: program-reported 379.952 seconds is not a comparable unattended benchmark.
An initial wrongly scoped debugger run was explicitly aborted and excluded.
All 30,496 sources parse; uncapped javac still fails with 60,972 diagnostics. Coverage is not whole-app compilation.
Goal active/incomplete. Evidence: bench/samsung-pass-90-wave18*.json.

## Historical measured wave17: 75.08139 percent; validation complete

153,984 strict bodies + 306 proven enum equivalents = 154,290 / 205,497.
30,658 more accepted original identities are required for 90 percent.
Wave16 identity delta: 2,613 gained, zero lost. Original Fragment.requireContext,
Duration.create(JI), LongArray constructor and shared void-terminal exception
routes are connected. All 34,579 classes visited; 30,371 source files; export
errors/skips zero. 574 workspace tests, strict all-target Clippy and fmt passed.
All 30,371 exact sources pass syntax parsing. Uncapped javac still fails with 61,661 diagnostics. Full30 took 419.187 seconds,
peak RSS 8,189,560 KiB. RSS remains within the recent 7,979,304–8,189,380KiB range. The wall-time increase
is unresolved and is not attributed without profiling. Coverage does not establish whole-app compilation/runtime.
Artifacts: bench/samsung-pass-90-wave17.json, -inventory.json, -delta.json.
Goal active/incomplete.

## Historical measured wave16: 73.80984 percent; verification complete

151,377 strict bodies + 300 proven enum equivalents = 151,677 / 205,497.
33,271 more accepted original identities are required for 90 percent.
Wave15 exact identity delta: 1,345 gained, zero lost.
Full30: 249.689 seconds, all 34,579 classes visited, 30,359 source files,
zero export errors/skips. Workspace tests and strict all-target Clippy passed.
All 30,359 sources pass syntax parsing. Uncapped javac still fails with 64,082 errors.
Workspace tests: 566 passed. Coverage is not whole-class compilation or APK
execution. Goal active/incomplete.
Artifacts: bench/samsung-pass-90-wave16.json, -inventory.json, -delta.json.

## Measured wave15: 73.15533 percent

150,032 strict bodies + 300 proven enum equivalents = 150,332 / 205,497.
Wave14 exact identity delta: 555 gained, zero lost. The previously withdrawn
bu/f1.f1 body is restored after deterministic loop structuring.
All 30,345 sources parse; uncapped javac fails with 63,374 diagnostics.
Workspace tests: 565 passed. Full30: 244.311 seconds. Goal remains incomplete.

## Historical measured wave14: 72.88525 percent; one withdrawal under audit

149,477 strict bodies + 300 proven enum equivalents = 149,777 / 205,497.
35,171 more accepted original identities are required for 90 percent.
Wave13 delta: 468 gained, one lost (bu/f1.f1, cyclic-control rejection).
Full30: 249.627 seconds, all 34,579 classes visited, 30,344 source files,
zero export errors/skips. Workspace tests: 559 passed. Strict workspace/all-target
Clippy passed. All 30,344 exact sources pass syntax parsing. Uncapped javac fails with
63,317 errors (Wave13: 63,287); no whole-class or APK compilation claim. Original definition denominator and accepted statuses
remain unchanged. Goal active/incomplete.
Artifacts: bench/samsung-pass-90-wave14.json, -inventory.json, -delta.json.

## Latest measured wave13: 72.6580 percent; withdrawal audit active

149,010 strict bodies + 300 proven enum equivalents = 149,310 / 205,497.
35,638 more accepted original identities are required for 90 percent.
Wave12 exact identity delta: 1,992 gained, 740 lost. New withdrawals are
under source-throws contract audit; no loss is hidden or counted as recovered.
Full30 export: 226.208 seconds, all 34,579 classes visited, 30,344 source files,
zero export errors and skips. Workspace tests: 551 passed. Strict workspace
all-target Clippy passed. All 30,344 exact sources pass syntax parsing. Uncapped javac still fails
with 63,287 errors, down from Wave12 64,637. Whole-app compilation is not established.
Artifacts: bench/samsung-pass-90-wave13.json, -inventory.json, -delta.json.
Goal remains active and incomplete.

## Latest measured wave12: 72.0487 percent; identity loss audit active

147,758 strict bodies +300 proven enum equivalents =148,058/205,497.
36,890 more accepted identities required for90%. Wave11 delta+546/-169;
newlosses under raw/source audit, never hidden or counted as recovered.
Full30:191.334sec,34,579classesvisited,29,904files,error0,skips0.
ActualFAB31028 nowcomplete; filteredclass36->81supportedbodies.
All29,904exact sources parse; fulluncappedjavac still fails64,637diagnostics. Workspace tests534passed,0failed;
strictworkspace/alltargetClippy passed. DerivedsourceThrows pass underwork;
not included in measuredbinary. Goalactive/incomplete.

## Latest measured wave11: 71.8653 percent; regression audit active

147,381 strict bodies + 300 proven enum equivalents = 147,681 / 205,497.
37,267 more accepted original identities are required for 90 percent.
Compared with wave10: 2,072 gained, 138 lost (constructor regression under repair).
Compared with original baseline: 58,387 gained, zero lost. Full30 visited all
34,579 classes; 29,902 source files; zero export errors/skips; 157.724 seconds.
Workspace test log records 529 passing tests. All 29,902 exact sources pass Java syntax parsing. Uncapped full javac still
fails with 64,430 diagnostics; no sources were excluded or rewritten.
The completed exception terminal-return adapter is built but not yet measured
in a fresh full export. Goal remains active and incomplete.

## Latest measured wave10: 70.9241 percent; attested this-delegation fix live

145,447 strict bodies +300 enum equivalents =145,747accepted out of205,497
original definitions. Wave9 gain918/loss0; baseline gain56,453/loss0.
All30,036exact files parse; fulluncappedjavac still fails62,988errors.
Full30:157.427sec,all34,579classes visited,noexporterrors/skips.
39,201more accepted identities requiredfor90%. Narrowregister Byte/Short/Char
widening andprechain allocation arguments are validated.

Wave11 fixes a production resolver bug: delegates_to was alwaysNone. Newcached
receiver-identity/linear-CFG proof derives exactthis targets fromoriginalSSA;
argumentobject initializers are excluded. ActualPersister now20/20constructor
bodies versus1previously. This count is filteredsource evidence, not fullclass
compilation or measuredwholeAPK gain. StrictClippy/focusedidentitytests/build
pass; full30/workspace tests running. Goal active/incomplete.

## Latest measured wave9: 70.4774 percent

144,529 strict bodies +300 enum equivalents =144,829 accepted out of205,497
original definitions. Wave8 gain3,897/loss0; originalbaseline gain55,535/loss0.
All29,714 exact sources parse; fulluncappedjavac still fails63,213errors.
Workspace521tests passed,0failed,23ignored. Full30:150.872seconds,all34,579
classes visited,zero export errors/skips.40,119 moreaccepted originals required
for90%. New pre-chain allocated arguments and narrowregister scalar widening
are undergoing focused source/runtime validation. Goal active/incomplete.

## Latest measured wave 8: 68.5811 percent; next recovery integration underway

140,632 strict bodies +300 enum equivalents =140,932 accepted out of205,497
original definitions. Baseline delta +51,638/-0. All113 wave7 regression
identities restored. All29,672 exact files parse; uncapped javac still fails
62,851errors, compared with69,735 wave6. Full30:146.049seconds,34,579classes
visited,zero export errors/skips.44,016 accepted originals remain for90%.

New numeric allocation arguments, branch-final static initialization, closed
reachable constructor subsets and inherited class-super dispatch are integrated
or under focused actual-method validation. No unmeasured coverage is counted;
class initialization unknowns remain separately reported. Goal active/incomplete.

## Latest measured wave 7: 68.5261 percent; copy regression fixed in head

140,519 strict bodies + 300 enum equivalents = 140,819 accepted out of 205,497
original definitions. Compared with the original baseline: 51,525 gained,
zero lost. Compared with wave 6: 113 lost; 54 direct pending-allocation
rejections and 59 dependent closure rejections. The initialized-copy marker
regression is corrected in head with a premature-read, successful-copy, and
repeated-initializer regression; full wave 8 remeasurement is running.

All 29,671 exact sources parse. Full uncapped javac still fails, but diagnostics
fell from 69,735 to 62,688. Workspace tests: 515 passed, zero failed, 20 ignored.
Full30 visited all 34,579 classes, zero export errors/skips, 176.456 seconds.
44,129 accepted identities remain for 90 percent. Method coverage does not
establish whole-class initialization or whole-app compilation. Goal active.
See `bench/samsung-pass-90-wave7.json` and `-validation.json`.

## Historical wave 6: 68.5811 percent

140,632 strict bodies + 300 enum equivalents = 140,932 accepted. Full javac
failed with 69,735 diagnostics. Its allocation lexical-scope error was fixed
in wave 7; initialized-copy recovery is now under wave 8 validation.

## Previous verified wave 5: 67.5441 percent

138,501 strict complete bodies +300 enum equivalents =138,801 accepted original
identities against205,497. 49,507 baseline gains,zero losses;46,147 remain for90%.
29,660 exact files parse; whole-source javac still fails70,133uncapped errors.
All34,579classes visited,no export errors/skips.513tests passed/0failed/20ignored.
See `bench/samsung-pass-90-wave5.md`. Later exception-local/common-ancestor repairs
are undergoing fresh measurement; no unmeasured coverage gain is counted.

## Previous verified wave 4: 63.1664 percent, zero baseline identity losses

Against 205,497 original definitions, **129,517 strict explicit bodies + 288
proven enum equivalents = 129,805 accepted**. 40,511 gained and zero lost relative
to the source-closure baseline. Another 55,143 accepted identities are needed for
90 percent. All 29,628 exact sources parse; full uncapped javac still fails with
64,521 diagnostics. Full30 export: 76.463 seconds, peak RSS3,181,756 KiB, zero
export errors/skips. This is body coverage, not compiled-app or initialization
proof. Details: `bench/samsung-pass-90-wave4.md`. Rejected identities and reasons:
`bench/samsung-pass-90-wave4-rejected-methods.csv`. Sequential try-scope work is
underway; the 90 percent goal remains active and incomplete.

## Historical wave 3 regression checkpoint

126,092 strict complete bodies plus 288 proven enum equivalents against 205,497
original definitions: **61.4997 percent**. Another 58,568 accepted identities are
needed for 90 percent. Compared with the original source-closure baseline:
38,656 gained, **1,570 lost**. The new argument folding skip-selection regression
is identified and fixed with a focused fixture; full remeasurement is pending.
All 29,612 exact sources parse; full javac fails with 62,784 uncapped diagnostics.
See `bench/samsung-pass-90-wave3.md`. The goal is active and incomplete.

## Previous wave 2 measurement

The active target remains incomplete. Against the unchanged 205,497 original
definitions, the fresh second wave retains 121,271 explicit complete bodies and
288 independently proven enum equivalents: **121,559 accepted identities,
59.1537 percent**. Relative to the source-closure baseline, **32,265 gained,
zero lost**. Bodyless declarations and unresolved semantic-only bodies are not
counted. Another **63,389 accepted original identities** are required for 90%.

All 29,576 exact sources parse; full uncapped javac still fails with **62,702
diagnostics**. This is method body/equivalence coverage under historical strict
closure rules, not globally complete class initialization or compiled-app
coverage. The full retry visited 34,579 classes with zero export errors; wall
time was 72.56 seconds and peak RSS 3,095,056 KiB. An earlier failed wave is
excluded and separately retained, with its static-initializer panic regression
fixed. Actual instance-final ctor subsets and throw/branch fixtures were
compiled/executed; missing/double assignments and unchecked proof gaps reject.

Evidence: `bench/samsung-pass-90-wave2.json`,
`bench/samsung-pass-90-wave2-validation.json`, and
[`90 percent execution plan`](samsung-pass-90-plan.md).
Output:
`/tmp/opencode/decomphify-java-corpus/run-1790928515677799756-3345608/com.samsung.android.samsungpass`.

## Historical production source-closure integration

Final evidence: [`bench/samsung-pass-sourceclosure-final.md`](../bench/samsung-pass-sourceclosure-final.md)
and `bench/samsung-pass-sourceclosure-final*.json`. The actual final export,
uncapped compiler gate and unchanged real wholeclass probes were run after the
last production changes. **Production progress is verified; whole-app compilation
and the aggregate error-reduction goal remain unmet.**

| Metric | Original baseline | Prior five-path final | Source-closure final |
|---|---:|---:|---:|
| Strict explicit bodies | 79,340 | 81,828 | **89,026** |
| Proven enum equivalents | 257 | 263 | **268** |
| Genuine bodyless declarations | 11,140 | 11,140 | **11,140** |
| Strict rejected definitions | 114,760 | 112,266 | **105,063** |
| Semantic explicit bodies, separately | 79,340 | 97,199 | **104,161** |
| Published Java units | 24,749 | 26,270 | **29,398** |
| All-source uncapped javac diagnostics | 38,158 | 51,302 | **57,245** |

The original denominator remains **205,497**. Against the prior final:
**7,203 strict body/equivalence identities gained, 0 lost**;
**6,962 semantic explicit identities gained, 0 lost**. Against the original:
**9,697 strict identities gained, 0 lost** and **24,821 semantic identities gained,
0 lost**. Every original and prior published source-unit identity remains present.
The strict measure retains its historical same-class closure meaning; it is not
redefined as global declaration closure or wholeclass compiler validity.

### Integrated production behavior

- `FieldPublication` is collected before alias freezing, sharing the single
  whole-input write inventory. Actual-owner declarations are serialized before
  body references, including genuine field-only owner units. Frozen-name encoded
  prefixes precede original clinit when atomic initialization is proven. Blank
  interface fields remain unavailable; malformed field metadata is reported
  separately rather than aborting analyst export or inventing declarations.
  The backend consumes the globally proven receiver strategy, including protected
  access through multiple parent levels. Private/package-private inherited
  references use actual declaring-owner casts; caller access alone is not proof
  of Java inheritance into the receiver type.
- `GlobalDeclarations` caches original defining-DEX locations, flags, exceptions,
  signatures and body status. The worklist consumes actual cached method
  publications once per identity; it does not reparse a class for every callee.
  Unavailable bodies remain unavailable. Native/abstract declarations require
  actual source publication; external declarations require exact classpath
  metadata. Worker results excluded by a publication limit cannot attest source
  availability.
- Constructor families accept `DeclarationBindings`, preserving original arity
  and unused descriptor positions without fabricated SSA values. Constructor and
  static branch-tree lowering retains the existing hierarchy/super-chain and
  original-final-field proofs. No replacement concrete bodies or forced abstract
  classes are introduced. AnalystAll remains the default; strict-valid-class
  filtering is not used to improve these measurements.
- Cached input-owner lookup eliminates per-field whole-input class scans. Owner
  field/write lookups use bounded ranges. Final Pass30: **61.319 s / 2,734,608 KiB
  RSS**, **0 export errors, 0 skipped classes**, using the existing JDK17
  `java.base.jmod` and Android35 jar. Memory includes cached global metadata and
  buffered actual publications.

### Genuine compilation improvements and remaining evidence

**Six previously failing retained units now compile with their unchanged genuine
source closures**: `androidx/constraintlayout/widget/l$a.java`,
`androidx/viewpager/widget/ViewPager$a.java`,
`com/google/android/material/appbar/CollapsingToolbarLayout$d.java`,
`em/f$type.java`, `nx/a$type.java`, and `androidx/appcompat/widget/Toolbar$g.java`.
Before/after commands, source hashes, every actual dependency unit and full logs
are recorded in `samsung-pass-sourceclosure-final-wholeclasses.json`.

**206 original constructors** are newly recovered against the prior final.
Unchanged `KCDSAPrivateKeySpec.java` compiles with its actual six-argument
constructor, unused `p4`, original `super()` and original field writes.
`CollapsingToolbarLayout$d` compiles with all three actual constructors against
the genuine Android `FrameLayout.LayoutParams` parent and its real emitted
resource-owner source. The existing three-class JDK17 subset and Android35
Guideline wholeclass also compile unchanged.

All **29,398** sources parse, with **0 syntax errors**. Full javac still fails:
**57,245** diagnostics, cap **2,147,483,647**, not reached. On the **26,270** matched
prior units, total attribution errors rise **51,302 -> 53,555**; new-only units
add **3,690**. Matched-unit code/type diagnostics fall **2,936 -> 2,770**, while
missing-declaration diagnostics rise **44,102 -> 45,787** and ambiguous
constructor-selection/missing-overload diagnostics rise **4,264 -> 4,998**.
These classifications are compiler evidence, not complete per-error causal proof.

Global direct missing-method callers: **29,225**. Field-blocked callers:
**51,051**, including **50,568** initialization-blocked callers.
Initialization-unavailable source units: **20,646**. These global measures are
separate from historical same-class dependency counts. Output remains explicitly
SourceFallback where class/initialization closure is incomplete.

Final verification: **465 tests passed, 0 failed, 13 ignored**; workspace/all-target
strict clippy, fmt and release build pass under JDK17. Final source root:
`/tmp/opencode/decomphify-java-corpus/run-1790912079467647626-1495733/com.samsung.android.samsungpass`.
Complete exact identity deltas and per-file diagnostic messages are saved without
truncation or published-source exclusions.

## Fresh five-path final — supersedes the historical measurements below

Latest evidence: [`bench/samsung-pass-five-final.md`](../bench/samsung-pass-five-final.md),
with exact identities, validation, diagnostic categories and per-file old/common
error counts in `bench/samsung-pass-five-final*.json`.

| Metric | Original baseline | Fresh final |
|---|---:|---:|
| Strict explicit bodies | 79,340 | **81,828** |
| Proven enum equivalents | 257 | **263** |
| Bodyless declarations | 11,140 | **11,140** |
| Strict rejects | 114,760 | **112,266** |
| Semantic explicit bodies (separate) | 79,340 | **97,199** |
| Published Java files | 24,749 | **26,270** |
| Full uncapped javac diagnostics | 38,158 | **51,302** |

The immutable denominator remains **205,497** definitions. Strict
body/equivalence identities: **2,494 gained, 0 lost**. Semantic explicit
identities: **17,859 gained, 0 lost**. Of the **15,371** semantic bodies outside
strict closure, **13,459** have directly unresolved same-class declarations.
These are distinct metrics, not changes to the old coverage definition.

`Ld1/g;->a(I)J` is restored in the original-SSA regression and asserted complete
in the final ledger. Its actual Java locals are **int** for category-one zero
and shift distance, **long** for both numeric-cast results and long arithmetic;
the emitted method remains `long a(int p0)`. Numeric Cast consumers consult the
actual synthetic target through `JavaResolver::instruction_type`, separately
from their source numeric category. Missing metadata remains indeterminate.

| Five production paths | Latest status |
|---|---|
| ctor/writeinv initializer | Existing atomic initialization/write inventory and exact external direct-parent constructor-body super proof retained. |
| contextual_locals | **Atomic checked phi/copy-family declarations adopted** in ordinary structured, switch and shared-forward paths. Whole-arena consumers, concrete leaves, conflicts and original input coercions are checked. Original availability and simultaneous edge snapshots are retained. Loop/exception adoption is bounded. |
| dependency publication | Real semantic callers remain published independently of legacy strict closure. Exact directly unresolved declarations remain measured. Constructor/clinit publication is atomic. |
| externalfields | **Public inherited classpath actual-owner resolution adopted**, using descriptor-exact genuine metadata, original access/staticness and actual-owner aliases. Cross-DEX/source-field publication and globally attested explicit initialization remain deferred; protected inherited production access stays bounded. Java ConstantValue inlining/class-initialization preservation is an existing unresolved subset limitation. |
| structure_flow | Canonical shared acyclic placement, availability intersections, original edge copies and typed returns retained. |

Fresh post-edit verification: **442 tests passed, 0 failed, 12 ignored**;
workspace/all-target strict clippy, fmt and release build pass under JDK17.
New runtime fixtures compile and execute integer/boolean/reference zero joins,
downstream copies and selected branch effects. Genuine Android35 inherited-owner
output compiles against the exact jar. Existing protected receiver, hidden-field,
staticness and cross-DEX resolver tests pass; helper tests do not establish
cross-DEX production publication.

All **26,270** published sources parse, with **0 syntax errors**. Whole unchanged
source javac still fails: **51,302** diagnostics, uncapped limit **2,147,483,647**
not reached. All **24,749** original files remain present. Their attribution
errors rise **38,158 -> 48,091**; new-only files contribute **3,211** errors.
Per-file old/final messages are saved separately. No published bodies were
excluded to improve the error count; compilation/error reduction is not claimed.

Final Pass30 export: **72.096 s / 1,477,776 KiB RSS**, **0 export errors, 0 skips**,
using the same JDK17 `java.base.jmod` and Android35 jar. Source root:
`/tmp/opencode/decomphify-java-corpus/run-1790897304245626033-3778956/com.samsung.android.samsungpass`.
Logs: `/tmp/opencode/samsung-pass-five-final/`. This remains **partial integration**,
not completion of the deferred cross-DEX publication path.

## Historical five-blocker integration — superseded measured bounds

**Partial integration; the actual-output error-reduction goal is not met.**
This section supersedes the measurements below. The final production export has
more real bodies and source units, but full javac diagnostics rise from **38,158
to 50,966 (+12,808)**. Full compilation still fails. No diagnostic exclusions,
source rewriting, fabricated dependencies or replacement bodies are used.

### Exact five categories: integrated and deferred

| Category | Actual production integration | Remaining boundary |
|---|---|---|
| ctor/writeinv initializer | Existing whole-input write inventory and atomic initialization family remain active. `ConstructorResolver::constructor_super_target` now connects genuine external method lists, exact original direct-parent identity, original invoke-super opcodes and unaliased method names to constructor-body `super` emission. The existing invocation access/hierarchy checks still run. | Arbitrary unknown writes and unproved finals still fail. Broader super dispatch and constructor delegation metadata are incomplete. |
| contextual_locals | Full-consumer constant/use-site code is active; operands and filled-array literal replacements now require materialization. Captured incoming provenance, initial seeds and current metadata cache remain the production authority. | **Atomic `checked_phi_plan` declaration-family integration is deferred.** Nominal phi checking remains; no partial family rewrite is installed. Unmodeled Cast/monitor/reference operand roles fail closed. |
| dependency publication | Real ordinary caller bodies outside the legacy closure gate are emitted with separate semantic provenance. `MemberRequirements::method_closure` audits actual emitted bodies and genuine emitted native/abstract declarations and writes exact unresolved original-method identities. Old counters and original strict identities keep their meaning. | Constructor/clinit closure remains atomic. The standalone publication adapter and cross-class/global declaration closure are not complete production paths. |
| externalfields | Exact public classpath fields are resolved from genuine field/class metadata, original caller access and opcode staticness. Actual field reads/writes are emitted; neither `ConstantValue` nor guessed initializers are substituted. Input-owned classes take precedence. | **Inherited actual-owner and cross-DEX source-field publication integration is deferred.** Their resolver helpers/tests exist, but that does not prove end-to-end source closure. Protected/nonpublic and source-renamed external fields remain unsupported here. |
| structure_flow | Both facade prechecks and the backend connect canonical Shared acyclic regions. Production planning validates definition identities and intersects availability over every incoming path; original edges execute parallel copies. Real blocks are placed once using forward labelled breaks, with hoisted declarations and original in-place effects/typed returns. | Cycles/exceptions and unsupported instruction/type forms retain their existing rejection boundaries; explicit Throw is still rejected by the branch validator. No state-machine or fallback return is introduced. |

The earlier `externalfields:277` borrow error does not persist. Workspace check,
tests, clippy and fmt pass. General availability is not claimed as complete-path
support for any deferred row.

### Coverage continuity and BOTH original-identity comparisons

The same **205,497 original definitions** reconcile across all four DEX files.

| Original-identity metric | Latest baseline | Final |
|---|---:|---:|
| Legacy strict explicit bodies | 79,340 | **80,092** |
| Proven enum equivalents (separate) | 257 | **263** |
| Genuine no-body declarations | 11,140 | **11,140** |
| Legacy strict rejected definitions | 114,760 | **114,002** |
| Written Java units | 24,749 | **26,209** |
| Full javac diagnostics | 38,158 | **50,966** |

Final strict partition: **80,092 explicit + 263 equivalents + 11,140 declarations
+ 97,873 written-unit rejects + 16,129 unpublished-unit rejects = 205,497**.
Strict body/equivalence identity comparison: **759 gained, 1 lost, net +758**.

Semantic explicit bodies are **94,819**, not a redefinition of old coverage:
**14,727 ordinary bodies** are emitted outside legacy transitive same-class
closure. Of these, **13,158** have directly unavailable same-class method
declarations; **1,569** now have their direct declarations present in the expanded
subset. Semantic identity comparison against the original baseline explicit set:
**15,480 gained, 1 lost, net +15,479**. All identities are saved, not inferred from
aggregate counters. The direct same-class method-declaration-closed explicit count
is **81,661**; this does not establish cross-class, transitive whole-subset or full
JVM attribution closure.

The one lost identity in both comparisons is **`Ld1/g;->a(I)J`**. A targeted test
using original seeded SSA and the production resolver observes
`Type("contextual constant constraints: Indeterminate(UnknownValue)")` for its
unmodeled numeric Cast consumer. It stays rejected rather than weakening the
consumer proof to preserve a coverage number.

Exact explicit-body splits (enum equivalents excluded): ordinary
**69,504 -> 70,217**, constructors **9,066 -> 9,102**, static initializers
**770 -> 773**. Final semantic ordinary bodies: **84,944**; constructor and
static-initializer semantic counts remain **9,102 / 773**. Enum equivalents split
into **153 ordinary / 55 constructors / 55 static initializers**. Facade counters
include these equivalents: **70,370 / 9,157 / 828** respectively.

### Honest source validation and real original fixtures

All **26,209 exact final sources** parse: **0 syntax errors**, exit 0. All are then
passed to JDK17 javac with Android35, empty sourcepath, no exclusions and
`-Xmaxerrs 2147483647`: exit 1, **50,966 diagnostics**, cap not reached. Source
hashes remain unchanged. More emitted code exposes additional missing members and
declarations; the error increase is an observed result, not an error-reduction
claim or a complete causal attribution.

Exact unmodified **Guideline.java** now compiles against Android35, including all
three original constructors and genuine `super.setVisibility` calls. Its original
constructor identities are **10300, 10301, 10302**. A separate **JDK17-only actual
three-class emitted subset** compiles together: `CryptoLibException`,
`BiometricTokenExpiredException`, `InvalidBiometricException`. The first two
retain their exact original two-constructor families; their parent dependencies
come from the chosen real subset. These whole original emitted files and their
hashes are recorded separately from helper-generated runtime fixtures.

There are **96,888 required fields** and **17,669 units** explicitly marked as
initialization unavailable. Honest subset, missing initializer and implicit-Java-
constructor markers remain in source. Bodies do not imply complete classes.

### First recorded rejection histogram: bounds, not exhaustive causes

| Five pattern groups | Ordinary before -> after | Constructor before -> after | Static initializer before -> after |
|---|---:|---:|---:|
| Initializer/final-write/constructor patterns | 1,867 -> 1,867 | 24,682 -> 24,638 | 5,444 -> 5,441 |
| Zero/UnknownValue | 16,169 -> 20,674 | 1 -> 2 | 1 -> 2 |
| Required method/field closure | 14,765 -> 14,729 | 79 -> 80 | 4 -> 4 |
| External field closure/metadata | 14,757 -> 15,946 | 136 -> 142 | 78 -> 77 |
| OverlappingRegion | 10,949 -> 3,393 | 2 -> 1 | 2 -> 1 |

These are explicit first-rejection string-pattern summaries. They are not five
disjoint bug populations; newly reachable paths change which reason is first.
Legacy closure rejects include the separately emitted semantic callers.
Top ordinary reasons: Zero **20,673**, legacy method closure **14,727**, source
field closure **13,404**, exact super target **4,058**, observable preinitializer
operation **3,412**, unsupported branch instruction **3,163**, unresolved field
metadata **2,542**. Constructors: whole-input final-write indeterminacy **19,150**,
field initialization **3,136**, unused parameter **1,720**. Static initializers:
whole-input final-write indeterminacy **4,829**, enum constructors **434**.

Full attribution groups include missing/unresolved output members **25,071**,
missing/unmapped external classes **7,851**, missing overrides **5,707**, missing
input classes/aliases **4,392**, constructor/signature/implicit parent **4,261**,
other attribution **2,787**, access **87**, module conflict **37**. Mixed groups
are not relabeled as genuinely absent SDK APIs.

### Final tests, resource profiles and artifacts

- Full final workspace: **440 passed, 0 failed, 12 ignored**, **200.127 s wall**,
  **1,079,920 KiB peak RSS**. Workspace/all-target clippy `-D warnings`, full
  `cargo fmt --all --check` and release production example build pass.
- Production shared-tail test compiles and executes all four branch combinations
  with distinct typed return outcomes. Helper tests additionally cover effects,
  snapshots, canonical-CFG rejection and exception propagation boundaries.
- Fresh Pass30 after the last production edits: **53.506 s app-reported**,
  **53.705 s actual wall**, **1,477,888 KiB RSS**, **0 export errors, 0 skips**.
- Parse-all: **7.232 s / 1,934,248 KiB**. Full uncapped javac:
  **25.527 s / 3,990,136 KiB**. These are single-run observations.
- Explicit metadata paths: JDK17
  `/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod` first, Android35
  `/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar` second.
- Final root:
  `/tmp/opencode/decomphify-java-corpus/run-1790894446651331481-3421921/com.samsung.android.samsungpass`.
- Complete logs, argument files, individual time/RSS records and identity delta:
  `/tmp/opencode/samsung-pass-five-blockers/`.
- Saved inventory, validation, diagnostic groups, final checks and compact summary:
  `bench/samsung-pass-five-blockers*.json`; concise report:
  `bench/samsung-pass-five-blockers.md`.
- Reproduce with `python3 bench/samsung_pass_five_blockers.py`, then
  `python3 bench/samsung_pass_five_summary.py`. `--resume` only reuses a successful
  workspace test gate; `--checks-only` refreshes final tests without replacing the
  already measured production export. Final source changes precede that export;
  subsequent changes are tests/reporting only.

## Next wave — genuine external member-type source names (latest)

**The whole goal remains partial.** This section supersedes the protected-receiver
results below. The immutable APK and **205,497 original definitions** are unchanged.

### Confirmed source bug and general fix

The complete previous javac log contains **2,279 missing-symbol diagnostics for
classpath-present binary nested names**. For example, generated source used
`android.graphics.drawable.Drawable$ConstantState` instead of the genuine Java
member-type name `android.graphics.drawable.Drawable.ConstantState`. These are
emitter naming errors, not absent Android APIs.

- `java_classpath.rs` now reads bounded `InnerClasses` attributes from actual
  class files. `ExternalMetadata::member_source_names` derives source names
  recursively from exact inner/outer/name relationships, requiring loaded outer
  declarations, consistent binary identities, valid identifiers, and no cycles
  or ambiguous self entries. Local/anonymous classes are not member-name proofs.
- `java_aliases.rs` applies these attested names to frozen external reference
  aliases; `java_integration.rs` wires them once per input before building shared
  hierarchy/resolver metadata. Input classes retain flattened output names and
  paths. Input-defined outer prefixes block external remapping. Literal `$`
  top-level names and unknown/missing metadata retain their existing identities.
- Class headers, fields, parameters, return types, arrays and emitted references
  share the same lookup. Original DEX/JVM descriptor member keys are unchanged.
  No missing API, initializer, stub, empty replacement body or constructor was
  supplied to make javac succeed.

### Exact original identity accounting

| Gate/count | Protected final | Next wave |
|---|---:|---:|
| Original method definitions | 205,497 | **205,497** |
| Explicit complete bodies | 79,340 | **79,340** |
| Proven enum equivalents | 257 | **257** |
| No-body declarations | 11,140 | **11,140** |
| Rejected original methods | 114,760 | **114,760** |
| Accepted constructors / static initializers | 9,120 / 824 | **9,120 / 824** |
| Written units / emitted required fields | 24,749 / 95,385 | **24,749 / 95,385** |
| Initialization-unavailable units | 16,328 | **16,328** |

**Zero gained bodies and zero lost bodies; zero status transitions.** Exact
original identity sets reconcile across all four DEX files. The partition stays
**79,340 explicit + 257 equivalents + 11,140 declarations + 90,546 written-unit
rejects + 24,214 unpublished-unit rejects = 205,497**. Explicit coverage remains
**38.608836%**; body/equivalence coverage remains **38.733899%**. This wave improves
source correctness, not executable-body recovery volume.

### Complete diagnostic comparison and attribution limits

All **24,749 exact sources** were parsed and passed to full JDK17 javac with
Android35, empty sourcepath, no exclusions and `-Xmaxerrs 2147483647`. The cap
was not reached. Parsing: **0 syntax errors**, exit 0, **6.480 s**. Full
attribution: exit 1, **22.322 s**; the subset still does not compile as a whole.

| Observed diagnostic group | Before | After |
|---|---:|---:|
| Total | 40,060 | **38,158** |
| Cannot find symbol | 30,475 | **28,252** |
| Classpath-present binary nested source names | 2,279 | **0** |
| Missing original input class or alias (direct name match) | 5,244 | **5,244** |
| Missing external or unmapped/aliased class (mixed, unresolved) | 6,655 | **6,655** |
| Missing output member or unresolved symbol (mixed) | 16,297 | **16,353** |
| Missing original input package | 129 | **129** |
| Missing external package | 498 | **498** |
| Missing output override or signature alias | 5,166 | **5,465** |
| Constructor signature or implicit parent | 2,861 | **2,881** |
| Module conflict (`java.xml`) | 37 | **37** |
| Access diagnostics | 82 | **82** |
| Other source attribution | 812 | **814** |

The naming group disappears completely, while resolving types exposes previously
suppressed override/member/constructor checks. Net reduction: **1,902 diagnostics
(4.75%)**. Protected access remains **10**, private access **66**, incompatible
types **223**, cyclic inheritance **0**. These are observed diagnostics, not a
census of every latent defect. The mixed symbol groups require individual original
metadata/member-closure probes; they must not be relabeled as absent libraries.

Explicit classpath gaps still include Samsung fingerprint/iris declarations,
AndroidX window extension APIs, Conscrypt and legacy Apache HTTP. Android35 cannot
establish every API37 or Samsung proprietary declaration. Unpublished APK classes,
missing original overrides and alias/signature closure are separate blockers.

### Unmodified-source compilation and regressions

Five **whole exact regenerated units fail before and compile after**, using only
JDK17 + Android35 and an empty sourcepath:

- `androidx/core/widget/d0$f.java`
- `androidx/percentlayout/widget/a$c.java`
- `androidx/biometric/d$o.java`
- `androidx/biometric/d$n.java`
- `androidx/biometric/a$c.java`

All source hashes remain unchanged during validation. The five previously passing
Android-dependent Samsung representatives also still compile. This is static
compilation evidence; no APK method was executed.

The JDK17 regression compiles real class-file fixtures, derives two-level member
names, then compiles and runs a separate generated-type-name consumer covering
arrays, signatures, construction, class literals and a static return assertion.
It also checks literal `$`, unknown metadata, missing outer definitions,
input-owned outers, and local/anonymous nesting. These fixtures are separate from
the exact APK units. Full workspace: **413 passed, 0 failed, 11 ignored**;
**101.734 s**, max RSS **1,078,632 KiB**. Final expanded nesting regression,
backend-java/decomp all-target clippy `-D warnings`, and touched-file rustfmt
checks pass.

### Second probe: implicit no-argument parent construction

The exact Guideline unit was compiled before and after without modifications;
both still report **`no suitable constructor found for View(no arguments)`**.
Source SHA-256 is identical. Android35's actual `View` declarations were inspected
with JDK17 javap and contain no zero-argument constructor. All three original
Guideline constructor identities remain explicitly rejected with
`initialization proof: FieldInitializationUnavailable`. This proves the residual
publication problem, not constructor recovery or the lower-level cause of that
family-proof failure. Supplying a fictitious `super(Context)` or synthetic
constructor would hide it. Genuine constructor/final-field proof and declaration
closure remain the next recovery work; no workaround was added.

Recorded rejection frequencies are unchanged, including:

| First recorded method-rejection category | Count |
|---|---:|
| Ordinary unresolved/uninitialized Zero | 16,169 |
| Ordinary required complete body unavailable | 14,763 |
| Ordinary external field closure | 11,063 |
| Ordinary overlapping CFG region | 10,949 |
| Ordinary unresolved field metadata | 3,694 |
| Ordinary exact Java super target proof unavailable | 3,656 |
| Constructor whole-input final-write indeterminacy | 19,156 |
| Constructor field initialization unavailable | 3,173 |
| Constructor unused parameter | 1,720 |
| Static initializer whole-input final-write indeterminacy | 4,830 |

### Final artifacts, resource counters and reproduction

- Final root:
  `/tmp/opencode/decomphify-java-corpus/run-1790888814662654132-2680151/com.samsung.android.samsungpass`
- Inventory: `bench/samsung-pass-next-wave-final.json`.
- Identity delta, full category comparison, exact commands/hashes, constructor
  metadata/probe, tests and resource counters:
  `bench/samsung-pass-next-wave-validation.json`.
- Full uncapped logs: `/tmp/opencode/samsung-pass-next-wave/`.
- Full30 export after final production edits: **51.037 s reported**, **51.199 s
  process wall**, **1,459,444 KiB peak RSS**, **0 errors, 0 skips**. Classpath is
  explicitly JDK17 `java.base.jmod` first, Android35 `android.jar` second.
  Single-run timing/resource observations are not a performance improvement claim.
- `bench/samsung_pass_next_wave.py` runs workspace checks/build/export;
  `samsung_pass_inventory.py` independently enumerates DEX definitions and writes
  the identity ledger; run it before `samsung_pass_continue.py` (which needs that
  ledger). `samsung_pass_diagnostics.py` reads complete logs and groups them;
  `samsung_pass_next_wave_evidence.py` retains exact-unit and final-check evidence.

All units retain honest subset/equivalence provenance. Body acceptance does not
imply fully compiling classes. No compiled-APK, original-source 100%, native
decompilation, device/runtime-security, or protection-bypass claim is made.

## Protected-receiver continuation — latest production and final validation

**The goal remains incomplete.** This section supersedes the continuation below.
The immutable APK and **205,497 original-method denominator** are unchanged.

### Source-correctness fix and exact Guideline evidence

- `crates/backend-java/src/lib.rs` now permits an attested receiver's existing
  static subclass type or `this` to survive method/field emission. Genuine flags
  and independent receiver-selection proofs come from `JavaResolver`; default
  proofs fail closed. Descriptor argument casts still pin overload selection.
  An unproved cross-package protected receiver is rejected rather than widened
  to its declaring superclass. No parameter is downcast to manufacture access.
- `crates/decomp/src/java_integration.rs` supplies narrowly bounded proofs using
  original-name eligibility, existing `AccessContext`/Java access rules, hierarchy,
  and genuine declaration metadata. Currently the receiver must be a known input
  class directly extending the target owner. Competing same-name descriptors,
  private/static competing declarations, missing metadata, and aliased original
  identities fail closed. A same-descriptor ordinary override preserves virtual
  dispatch. Required field receiver retention additionally proves absence of a
  shadowing field using an input-wide field-name index built once and shared by
  workers. External field publication remains unsupported by genuine-field closure.
- Exact exported `androidx/constraintlayout/widget/Guideline.java:18` changes
  `((android.view.View) this).setMeasuredDimension(v0, v0)` to
  **`this.setMeasuredDimension(v0, v0)`**. JDK17 + Android35 compilation of each
  **unmodified whole unit** fails before with two diagnostics and after with one:
  the protected-access error disappears; `View(no arguments)` remains because
  all three original Guideline constructors are unavailable. No whole-unit pass
  is claimed. A separate fixture preserves the complete `onMeasure` declaration
  verbatim and supplies an explicit `super(Context)` fixture constructor:
  **before fails protected access; after compiles**. Exported source hashes are
  recorded and unchanged by validation. No Android/APK method was executed.
- JDK17 regressions cover emitted `this` and subclass-typed parameter receivers,
  descriptor-pinned overload calls, no-overload subclass calls, invalid
  superclass-typed receivers and unrelated other-package callers, protected field
  get/put and illegal parent casts. Actual resolver metadata tests cover valid
  access, original/alias rejection, competing overloads, exact overrides versus
  private/static lookalikes, and field shadowing. Access permissions are unchanged.

### Exact identity accounting versus continuation 79,351

| Gate/count | Previous continuation | Latest final |
|---|---:|---:|
| Original method definitions | 205,497 | **205,497** |
| Explicit complete bodies | 79,351 | **79,340** |
| Proven enum equivalents, separately counted | 257 | **257** |
| Body/equivalence total | 79,608 | **79,597** |
| Ordinary accepted bodies/equivalents | 69,664 | **69,653** |
| Accepted constructors | 9,120 | **9,120** |
| Accepted static initializers | 824 | **824** |
| Retained no-body declarations | 11,140 | **11,140** |
| Rejected original methods | 114,749 | **114,760** |
| Written units | 24,749 | **24,749** |
| Emitted required fields | 95,385 | **95,385** |
| Initialization-unavailable units | 16,328 | **16,328** |

**Zero gained bodies; 11 lost ordinary bodies**, each with recorded
`protected receiver selection proof unavailable`. These are conservative
publication corrections, not a recovery gain. Exact descriptor identities,
original indexes, source paths and reasons are saved in the new validation JSON
and `/tmp/opencode/samsung-pass-protected-final/identity-delta.json`.

The exact partition is **79,340 explicit bodies + 257 enum equivalents + 11,140
declarations + 90,546 rejects in written units + 24,214 rejects without a written
unit = 205,497**. Explicit-body coverage is **38.608836%**; body/equivalence
coverage is **38.733899%**. Declarations are never recovered executable bodies.

### Final gates and categorized diagnostics

- Final release full30 export: **46.812 s**, JDK17 `java.base.jmod` first and
  Android35 second, **0 errors, 0 skips**. Original four-DEX identities reconcile.
- JDK17 `cargo test --workspace`: **412 passed, 0 failed, 11 ignored**. A later
  subclass-parameter assertion was validated by the final targeted protected tests.
  Scoped backend-java/decomp all-target clippy with `-D warnings` passes; touched
  Rust files are formatted. Initial tests without JDK on PATH failed to launch
  javac; the recorded successful checks supply JDK17 explicitly.
- Parse API gate on **all 24,749 exact source files**: **0 syntax errors**, exit
  0, **6.329 s**. Parsing is separate from attribution.
- Full exact-source javac, empty sourcepath, **no exclusions**: exit **1**,
  **22.121 s**, **40,060 observed diagnostics**. The **2,147,483,647** error cap
  is not reached; logs are complete. Earlier attribution errors can suppress
  later checks, so this is not a census of every latent source defect.

| Observed javac category | Before | After |
|---|---:|---:|
| All diagnostics | 40,083 | **40,060** |
| Cannot find symbol | 30,478 | **30,475** |
| Protected access | 27 | **10** |
| Incompatible types | 223 | **223** |
| Package-private access (`is not public`) | 6 | **6** |
| `java.xml` package/module conflicts | 37 | **37** |
| Cyclic inheritance | 0 | **0** |

- Five unchanged Android-dependent Samsung representatives still compile:
  `BiInfoColumns`, `ocr/nfc/e$type`, `ocr/nfc/d$type`, `AppInfoColumns`, and
  `KeyInfoColumns`. Alias-shadowing representatives `m0/q0$type` and `c/c$type`
  still fail on unavailable dependencies, with zero cyclic-inheritance reports.
- Remaining protected-category snippets are **not unnecessary superclass casts**:
  e.g. `NestedScrollView$b.b()` directly calls
  `v0.computeVerticalScrollExtent()`, but `NestedScrollView`'s original override
  is unavailable (`exact Java super target proof unavailable`). Javac resolves
  the inherited protected View member instead. Cross-unit emitted-member closure
  remains incomplete. The six package-access reports are `FactoryFinder` calls
  emitted within their original `javax.xml.stream` package alongside JDK17
  `java.xml` module conflicts; no broad package-access relaxation was applied.

### Latest artifacts and unsupported limits

- Final root:
  `/tmp/opencode/decomphify-java-corpus/run-1790872983292092140-960741/com.samsung.android.samsungpass`
  (`sources/`, `report.txt`, all-original `method-coverage.jsonl`).
- Inventory: `bench/samsung-pass-protected-final.json`.
- Evidence, categories, exact commands/hashes, identity delta rows and categorized
  source snippets: `bench/samsung-pass-protected-validation.json`.
- Full logs and fixture sources: `/tmp/opencode/samsung-pass-protected-final/`.
- Reproduction: existing inventory/continuation tools plus
  `bench/samsung_pass_protected_summary.py`.

Receiver-selection recovery across unknown/intermediate hierarchies and aliased
protected identities is still unsupported. Original constructor/initializer
recovery, interface initializer expressions, cross-unit declaration closure,
missing external APIs and native decompilation remain incomplete. The full subset
does not type-check as a whole; **no compiled-APK or full-recovery claim**. No
commits, device access, security runtime, or protection bypass were performed.

## Previous continuation: interface syntax and package-shadowing fixes

**The goal remains incomplete.** This section supersedes the older wave's final
counts below; historical artifacts remain available for comparison.

### Production changes and their limits

- `crates/decomp/src/java_integration.rs` and `java_members.rs` now refuse blank
  interface/annotation field declarations. Java makes these fields implicitly
  public static final, regardless of omitted DEX flags, and requires an initializer
  expression. A DEX `<clinit>` is not a legal Java interface static block.
  The current emitter has no attested interface initializer-expression recovery:
  these fields are explicitly **unavailable**, including encoded-initializer
  cases, rather than initialized with fabricated null/zero. Dependent bodies
  are rejected by genuine-field closure; their field reads are never erased.
  Unavailable field indexes/reasons are recorded in retained source units.
  Initialization status remains unavailable even when original flags omit final.
- Both prior parser-block cases are handled: `CertificateViewModel` retains an
  interface subset with its unavailable field/body recorded; `Lm0/b0;` loses its
  sole accepted `getKey()` body and is now an unavailable unit. This is a
  publication correction, **not initializer recovery**.
- `java_aliases.rs` reserves root-package names for class simple names in the
  frozen inventory. Actual prior source `m0/q0.java:9` said
  `class q0 implements q0.o, q0.l`: Java resolves `q0` as the declaring class,
  producing false cyclic inheritance. The same general bug affected `m0/m0`
  and `c/c`. Deterministic collision-checked `$type` leaf aliases are shared by
  declaration/reference/path emission, without changing original method keys.
  The final full javac reports **zero cyclic-inheritance diagnostics**. Exact
  `m0/q0$type.java` and `c/c$type.java` were compiled as representatives and still
  fail on unavailable dependencies; no whole-unit pass is claimed for them.
- Existing per-input classpath ingestion and per-DEX `DexMetadata`/resolver
  indexes remain shared by the 30 class workers; no per-class metadata scan was
  introduced. Original identities are reconciled against all four DEX files.

### Exact before/after versus the previous final

| Gate/count | Previous final | Latest final |
|---|---:|---:|
| Original method denominator | 205,497 | 205,497 |
| Explicit complete bodies | 79,365 | **79,351** |
| Proven enum equivalents | 257 | **257** |
| Body/equivalence total | 79,622 | **79,608** |
| Ordinary accepted bodies/equivalents | 69,669 | **69,664** |
| Accepted constructors | 9,129 | **9,120** |
| Accepted static initializers | 824 | **824** |
| Retained no-body declarations | 11,140 | **11,140** |
| Rejected original methods | 114,735 | **114,749** |
| Written units | 24,752 | **24,749** |
| Emitted required fields | 95,388 | **95,385** |
| Initialization-unavailable units | 16,322 | **16,328** |
| Full-source syntax errors | 2 | **0** |

**Zero gained bodies, 14 lost bodies** (5 ordinary, 9 constructors). The exact
original-descriptor delta is saved at
`/tmp/opencode/samsung-pass-continuation-final/identity-delta.json` and its lost
identity rows are included in `bench/samsung-pass-continuation-validation.json`.
Ten lost rows in written units explicitly report aliased protected-hierarchy
proof unavailable; one reports genuine field declaration unavailable. Three
lost rows have no written unit (including `Lm0/b0;->getKey()Ljq/i$c;`); the existing
coverage format does not preserve a per-method reason for unpublished units.
Another ten already-rejected identities move from written to unpublished units.
No gain in recovery is claimed for this syntax-correctness wave.

Final original identities partition exactly into **79,351 explicit bodies + 257
enum equivalents + 11,140 declarations + 90,535 rejects in written units + 24,214
rejects without a written unit = 205,497**. Explicit coverage is **38.614189%**;
body/equivalence coverage is **38.739252%** of all original definitions.
All written units retain fallback/subset provenance. There are **298 explicit
unavailable-interface-field records in written units**, including unused fields;
this is not a count of the three formerly emitted required fields removed.

### Latest gates: exact source, no diagnostic truncation

- Final full30 export after the last production change: **47.504 s**, **30 workers**,
  JDK17 `java.base.jmod` first, Android35 second. **0 output errors, 0 skips**.
- JDK17 `cargo test --workspace`: **410 passed, 0 failed, 11 ignored**.
  New fixtures exercise actual facade publication with a real field read, body
  rejection and original pool-index retention, exact subset javac success, and a
  negative blank-interface-field javac failure. Implicit/illegal field flags are
  tested. Package-shadowing regression compiles under `--release 17` and checks
  collision handling. Existing enum, initializer, access and identity tests pass.
  An initial run without JDK on PATH failed to launch javac; the recorded final
  full run explicitly supplies the JDK17 PATH and passes.
- `cargo clippy -p decomp --all-targets -- -D warnings`: passed. Touched Rust
  files formatted. No git operation or generated-source rewrite was used.
- `bench/SamsungPassParseGate.java` uses javac's parse API on **all 24,749 exact
  files**, in memory-bounded batches without diagnostic truncation: **0 syntax
  errors**, exit 0, **10.392 s**. Parsing is separate from type attribution.
- Full exact-source javac, with empty sourcepath and **no file exclusions**:
  exit **1**, **21.919 s**, **40,083 observed diagnostics**; maximum errors set to
  **2,147,483,647**, not reached. This is a completed diagnostic pass, not a
  census of all latent defects: javac's earlier attribution failures can suppress
  later checks. It includes **30,478 cannot-find-symbol**, **37 java.xml module
  conflicts**, unavailable package/member declarations, incompatible source
  types, and **27 protected-access diagnostics**. Missing symbols mix unpublished
  APK dependencies and absent external APIs; do not attribute everything to
  missing Android/Samsung libraries.
- **Five exact Android-dependent original units compile** with JDK17+Android35:
  `passservice/bi/BiInfoColumns.java`, `ocr/nfc/e$type.java`, `ocr/nfc/d$type.java`,
  `passservice/common/storage/db/AppInfoColumns.java`, and `KeyInfoColumns.java`.
  Full source paths, javac commands, SHA-256 and results are saved in the new
  validation JSON. No APK code was executed. Hashes verify all emitted files
  were unchanged by validation.

### Latest artifacts and next significant blocker

- Final root:
  `/tmp/opencode/decomphify-java-corpus/run-1790871140841023213-740155/com.samsung.android.samsungpass`
  (`sources/`, `report.txt`, all-original `method-coverage.jsonl`).
- New inventory: `bench/samsung-pass-continuation-final.json`.
- Compact final evidence: `bench/samsung-pass-continuation-validation.json`.
- Full diagnostics, identity delta, representative logs and passing tests:
  `/tmp/opencode/samsung-pass-continuation-final/`.
- Reproduction tools: `bench/samsung_pass_continue.py`,
  `samsung_pass_continuation_summary.py`, and `SamsungPassParseGate.java`.

The next strongest **source-correctness** probe is protected-receiver emission:
the final exact `androidx/constraintlayout/widget/Guideline.java:18` calls
`setMeasuredDimension` through a superclass-typed receiver and javac reports
protected access despite having real Android35 metadata. This is distinct from
missing dependencies and deserves an exact-source, receiver-type regression.
For recovery volume, whole-input final-write indeterminacy (2,135 unknown write
observations) still rejects **19,156 constructors** and **4,830 static
initializers**; interface initializer expressions are additionally unsupported.
No full-source type-correctness, compiled-APK, or 100% recovery claim is made.

## Scope and immutable input

Static managed decompiler development on the user-provided APK only. No device
access, app execution, dynamic dumps, credential inspection, or protection bypass.
Original-source 100% recovery is not promised: compilation discards information.
**This goal remains incomplete; the emitted subset does not compile as a whole.**

- Input: `tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk`
- Size: **54,063,076 bytes**
- SHA-256: `d49a8bbf7aac35ec4d2e842535fd85ac105c49291d9efe23e041ae8afee12c41`
- Static aapt package: `com.samsung.android.samsungpass`
- Version: **5.2.10.10**, versionCode **521010900**
- minSdk 30, targetSdk/compileSdk 37. Android35 metadata is explicitly supplied;
  it cannot establish missing API37 or Samsung proprietary declarations.
- Four DEX files; **34,579 defined classes**.
- Manifest and `resources.arsc` inventoried, not reconstructed as original resources.
- **37 assets / 3,457,123 uncompressed bytes**, not decompiled.
- Four native files: arm64/v7a `libDiagMonKey.so` and `libquram_ocr.so`.
  **Native libraries are unsupported and were not decompiled.** Names/sizes and
  DEX hashes are in `bench/samsung-pass-final.json`.

## Exact denominator and before/after

Independent DEX `class_data` enumeration gives **M = 205,497 original defined
methods**, including constructors, static initializers, and declarations. Pool
references are not counted as definitions. Of M, **194,286 have code** and
**11,211 have no code**. The inventory asserts reconciliation with facade counts.

| Category | Original definitions | Baseline accepted | Final accepted | Final rejected |
|---|---:|---:|---:|---:|
| Ordinary bodies | 164,926* | 68,034 | 69,669 | 84,117 |
| Constructors | 34,205 | 9,112 | 9,129 | 25,076 |
| Static initializers | 6,366 | 807 | 824 | 5,542 |
| Retained declarations, no body | included above* | 11,140 | 11,140 | included above* |

*Ordinary original definitions include no-code declarations. The rejected total
includes **71 no-code declarations** not retained. Never count declarations as
recovered executable bodies.

- Facade accepted body/equivalence count: **77,953 → 79,622**;
  **37.933887% → 38.746064% of all M**.
- The facade includes **257 proven Java enum/synthetic equivalents**, unchanged
  across runs. These have compiler-generated semantics rather than explicit
  per-method body markers. Report separately from explicit bodies.
- Explicit original complete-body provenance: **77,696 → 79,365**;
  final **38.621002% of all M**.
- Body/equivalence coverage among code-bearing methods: **40.981851%** final.
- Rejected original methods: **116,404 → 114,735**;
  final **55.832932% of all M**. Of these, **114,664 are code-bearing**.
- Identity delta: **1,669 gained, zero lost**. Exact descriptor-based identities,
  original pool indexes, and source units are in
  `/tmp/opencode/samsung-pass-validation/coverage-delta.json`.
- Generated and successfully written units: **24,552 → 24,752**; output errors 0,
  skipped classes 0. Generated classes are not completely recovered classes.
- All **24,752 units have `SourceStatus::Fallback` provenance** in the facade;
  banner partition: **24,698 honest-subset units**, **54 enum equivalence units**.
  **16,322 units** mark field initialization unavailable
  (baseline 16,165). This can rise as previously unpublished units become visible.
- Final field declarations: **95,388**. Declarations do not prove initialization.
- Final method identities partition into **79,365 explicit bodies**, **257 enum
  equivalents**, **11,140 declarations**, **90,534 rejects in written units**, and
  **24,201 rejects without a written unit**; these sum exactly to M.

Every original method has a machine-readable row in the final run's
`method-coverage.jsonl`, including DEX name/index, exact original descriptor,
access flags, presence of code, publication status, and available source/reason.
No original identity is invented for an implicit Java constructor.

## Implemented general fixes

1. `crates/backend-java/src/filled_arrays.rs`: explicit category-one raw-zero
   constants without a descriptor consumer now retain a proven `int` local.
   Applies to dead constants and arithmetic, without treating unknown values,
   phi selections, null, or category-two values as interchangeable. JDK17 fixture
   verifies arithmetic and dead-zero behavior. Wave 1 had **zero net corpus gain**;
   the unresolved-zero histogram is not solved by this limited fix.
2. `crates/backend-java/src/invoke_arguments.rs` and `src/lib.rs`: normalize DEX
   invocation register words to descriptor operands. `long`/`double` high words
   may be removed only when they have the **same attested SSA identity** as the
   low word. Logical managed operands remain supported. Arity and mismatched
   high-word rejection remain explicit. Contextual constant consumer indexing
   uses the same normalized operands. JDK17 regression verifies a real
   `Math.addExact(long,long)` invocation above 32 bits, overflow exception
   behavior, and rejection of a distinct high-word identity.

Tests added in `crates/backend-java/src/tests.rs`. Existing publication,
initialization, dispatch, checked-exception, metadata, and alias checks remain.
Changes do not supply missing proprietary metadata or invent constructor bodies.
The existing per-input classpath/index sharing is retained; no repeated classpath
inventory or previous 208+ benchmark sweep was added.

## Original Samsung package evidence

The original `com.samsung.android.samsungpass` namespace contains **51,941
definitions**: final **21,762 explicit bodies**, **71 enum equivalents**, **2,211
declarations**, and **27,897 rejects**. **7,402 source units** were written there.
Gain examples include:

- `ExportFailureSummary.g(ExportFailureSummary,long):void`, pool index 19582;
- `ExportFailureSummary.i(ExportFailureSummary,long):void`, pool index 19593;
- `backupandrestore.CxfJsonData.hashCode():int`, pool index 20582;
- `backupandrestore.IndexJsonData.toString():String`, pool index 20791.

Inspected an actual Android-dependent emitted original method:
`ocr.nfc.e.a(android.nfc.NfcAdapter):boolean`, index 43907; its emitted body calls
`NfcAdapter.isReaderOptionEnabled()` and returns the result. No application
method was executed during validation.

## Validation and timings

Full30 release exports all use these exact metadata inputs, JDK first:

```sh
cargo build --release -p decomp --example java_corpus
target/release/examples/java_corpus --workers 30 \
  --classpath /home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod \
  --classpath /home/jtjisgod/Android/Sdk/platforms/android-35/android.jar \
  --app tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk
```

| Run | Export-reported seconds | Process wall time | Peak RSS KiB |
|---|---:|---:|---:|
| Baseline | 46.622 | 46.83 | 1,400,904 |
| Wave 1 | 43.285 | 43.44 | 1,401,192 |
| Wave 2 final | 44.124 | 44.28 | 1,401,504 |

Timings are single observations, not a controlled speedup claim.

- JDK17 `cargo test --workspace`: **407 passed / 0 failed / 11 ignored**;
  wall **91.67 s**, peak RSS **1,036,676 KiB**. Test fixture construction initially
  failed compilation due to misuse of `ParameterBindings`; corrected before the
  successful full test run. Original negative checks were retained.
- `cargo clippy -p backend-java --all-targets -- -D warnings`: passed.
- Touched Rust files formatted with rustfmt (no broad unrelated formatting).
- `git diff --check`: passed.
- **Five unmodified Android-dependent original Samsung units compiled**, using
  JDK17 javac + Android35, no stubs: `passservice.bi.BiInfoColumns`, `ocr.nfc.e`,
  `ocr.nfc.d`, `passservice.common.storage.db.AppInfoColumns`, and
  `passservice.common.storage.db.KeyInfoColumns`. This is compile-only evidence;
  small representative units do not establish broad method correctness.
- Full unmodified 24,752-unit javac: **failed at two syntax diagnostics** (`=
  expected`) for uninitialized final interface fields in
  `certificate.viewmodel.CertificateViewModel` and `m0.b0`. This is an emitted
  declaration bug, not a missing dependency. No fabricated initializer was added.
- A separate **diagnostic-only partial pass** excluded those exact two invalid
  files, with empty sourcepath to prevent automatic reintroduction. **24,750
  units**, **26.393 s**, failed and reached **5,000-error cap**. Categories include
  4,842 `cannot find symbol`, 37 `java.xml` module-package conflicts, 3 cyclic
  inheritance reports, and missing Samsung fingerprint/iris and AndroidX window
  extension packages. `cannot find symbol` mixes unpublished input dependencies,
  unavailable members, and absent external dependencies; it is not evidence that
  all errors are proprietary metadata gaps. Diagnostics are capped, not exact
  counts of all defects. The module conflicts involve Android-bundled Java XML
  definitions under the JDK17 module system. **No compiled-APK claim.**

## Exact residual blockers and next decisive probes

Full categorized method rejection histograms and method-level identity samples
are saved in `bench/samsung-pass-final.json`; these are first recorded rejection
causes, not a census of every latent defect.

| Recorded category | Final count |
|---|---:|
| Ordinary: unresolved/uninitialized `Zero` | 16,170 |
| Ordinary: required complete body unavailable | 14,762 |
| Ordinary: field requires external class closure | 11,063 |
| Ordinary: overlapping CFG region | 10,949 |
| Ordinary: exact Java super target proof unavailable | 3,730 |
| Ordinary: unresolved field metadata | 3,694 |
| Constructors: whole-input final-write proof indeterminate | 19,156 |
| Constructors: field initialization unavailable | 3,173 |
| Constructors: unused parameter | 1,720 |
| Static initializers: whole-input final-write proof indeterminate | 4,830 |

Next decisive engineering steps:

1. Reproduce interface-field syntax failure with a minimal interface whose
   final reference is initialized by DEX `<clinit>`. Require attested source-level
   initializer recovery or an explicit unavailable field/publication outcome;
   never substitute null solely to satisfy javac.
2. Instrument the exact failing `Zero` value's definition, register provenance,
   consumer and emission stage. Distinguish a literal, incoming register, move,
   and phi before widening support. This wave's literal fallback demonstrably
   did not resolve the high-volume zero category.
3. Narrow the **2,135 unknown final-write observations** that conservatively
   block initialization across owners. Localize uncertainty only with evidence
   about actual possible write targets; do not discard global checks.
4. Use capped javac diagnostics to select individual unmodified source units for
   minimal source/metadata closure probes. Do not interpret missing subset
   dependencies or module conflicts as fabricated body correctness.

## Artifact locations

- Final sources:
  `/tmp/opencode/decomphify-java-corpus/run-1790868720256956227-453171/com.samsung.android.samsungpass/sources`
- Baseline root:
  `/tmp/opencode/decomphify-java-corpus/run-1790868280002616667-404337/com.samsung.android.samsungpass`
- Wave 1 root:
  `/tmp/opencode/decomphify-java-corpus/run-1790868529451525254-430754/com.samsung.android.samsungpass`
- Final root has `report.txt` and **all-original-method** `method-coverage.jsonl`.
- Compile logs, identity delta, and class files:
  `/tmp/opencode/samsung-pass-validation/`
- Full passing test log: `/tmp/opencode/samsung-pass-workspace-tests.log`
- Compact static inventories, category samples, commands and timing logs:
  `bench/samsung-pass-{baseline,final,summary,validation,semantic-validation}.json`
  and `bench/samsung-pass-*-{run,time}.txt`.
- Reproducible static accounting tools: `bench/samsung_pass_inventory.py`,
  `samsung_pass_validate.py`, `samsung_pass_semantic_javac.py`,
  `samsung_pass_summary.py`.

No git commits/pushes or model/vendor subprocesses were used. Existing unrelated
checkout changes were preserved.

## 2026-10-03 checkpoint: corrected Java 25 wave 1, validation finished

The active 99% goal remains incomplete. Original inventory stays at 205,497
definitions: 194,286 code-bearing and 11,211 genuinely bodyless abstract/native
forms. Corrected Java 25 retains 168,628 explicit bodies +298 proven enum
equivalents =168,926, plus 11,177 genuine declarations; 25,394 remain rejected.
Strict recovery is 82.203633% of all originals or 86.947078% of code-bearing
originals. Genuine original-form retention is separately 180,103 /205,497
=87.642642%; these views do not redefine the goal or count declarations as bodies.

Compared with the completed session baseline: 7,498 accepted identities gained,
zero lost, and 37 additional genuine declarations retained. The initial Java 25
withdrawal `Ltu/j;-><clinit>()V` is restored as `complete_body` by the independent
static fallback. Java 17 remains the default; additional constructor recovery
requires explicit source release 25. Corrected export finished in 276.465616
seconds, 278.359199 seconds including the subprocess, with 30,596 source files.

All 30,596 exact corrected sources parse. Full uncapped javac fails with 56,146
diagnostics, cap not reached, exact source hashes unchanged. Java 17 and initial
Java 25 validations also finished: all sources parse; full compilation fails
with 59,784 and 56,138 diagnostics respectively. No whole-app compilation,
compiled APK, Android execution or runtime-equivalence claim is made.

The pre-static-fix workspace run passed 705 tests; subsequent static regression
2/2, actual source-25 constructor 2/2 and fresh release session-cache 6/6 checks
passed. Parent-reported decomp all-target strict Clippy passed. The earlier
189.117 →1.436 second result remains a same-process source replay benchmark,
not a cold acceleration measurement for this wave.

Evidence: [Java release report](../bench/SAMSUNG_PASS_99_JAVA_RELEASE.md) and
[terminal wave summary](../bench/samsung-pass-99-wave1-summary.json).

## 2026-10-04 terminal checkpoint: wave33 candidate, no-loss gate not met

The active objective remains 100% of the original target. Accepted baseline stays wave29; wave33 is measured but not adopted because nine previously recovered identities are unavailable. No source/input/classpath drift occurred during the frozen export and validation.

Wave33 retains 182,649 explicit original bodies plus 429 separately proved enum equivalents: 183,078 /194,286 executable originals =94.231185%. Genuine bodyless declarations are separately 11,183 /11,211 and are not counted as executable bodies. Remaining: 11,208 executable originals and 28 bodyless forms. Compared with wave29, 1,238 identities gained and nine lost.

The actual array-reader #122, Lottie #23985 and four callers, SeMobileService #31994, and four AEAD methods are restored. Remaining losses are six checked-contract root methods and three dependency callers. Exact runtime/source facts and allocation ownership must be proved before a bridge extension; missing facts do not imply no runtime exceptions.

All 32,417 written sources pass parsing, but full uncapped javac still fails with 50,727 diagnostics. Workspace: 1,085 passed, zero failed, 151 ignored. Formatting, strict all-target Clippy and release passed. Export took 380.63 seconds with peak RSS17,477,060 KiB while workspace tests ran concurrently. Whole-app compilation, resources/native decompilation and Android execution remain unverified.

Evidence: [wave33 report](../bench/SAMSUNG_PASS_100_WAVE33.md), [terminal summary](../bench/samsung-pass-wave33-final.json), frozen artifact root `/tmp/opencode/samsung-pass-100-wave33`.

## 2026-10-04 terminal checkpoint: wave34 no-loss method-coverage progress

Accepted method-coverage baseline advances from wave29 to wave34. All original identities and metadata remain fixed; both wave29/wave26 no-loss gates pass. This is progress toward the unchanged100% objective, not whole-app completion.

183,217 explicit bodies +429 separately proven enum equivalents =183,646 /194,286 executable originals (94.523537%). Gain1,797, loss0 versus wave29. Remaining10,640 executable originals and28 bodyless forms. All nine prior wave33 losses are restored, as is actual constructor #54616; its full genuine dependency compilation still fails.

All 32,420 sources parse. Full uncapped javac still fails50,711 diagnostics. Workspace1,091passed/0failed/152ignored; fmt, strict workspace/all-target Clippy and release pass. Export276.59s/17,234,664KiB peakRSS versus wave33 380.63s/17,477,060KiB; window240→960 improves measured occupancy, with fidelity changes and concurrent tests retained as comparison limits. Whole compilation, native/resource recovery and Android execution remain unverified.

Evidence: [wave34 report](../bench/SAMSUNG_PASS_100_WAVE34.md), [terminal summary](../bench/samsung-pass-wave34-final.json), `/tmp/opencode/samsung-pass-100-wave34`.


### Wave35 cache measurement

Frozen export: 273.94 seconds, compared with wave34 276.59 seconds. Executable-body acceptance remains 183,646 / 194,286 (94.5235%); gained/lost original identities both zero. Both baseline no-loss gates pass; all 32,420 files parse. Whole-source compilation still fails. Full byte equivalence is not established: 5,311 source files differ, including dispatch state numbering, currently under investigation. Receipt: `bench/samsung-pass-wave35-cache-final.json`. The 100% goal remains incomplete.


## Wave36 terminal verification

183,732 / 194,286 executable definitions accepted (94.567802%); 86 gained and zero lost versus wave35. Remaining 10,554 executable definitions plus 28 bodyless forms. All 32,420 sources parse; full compilation still fails with 50,658 diagnostics. Workspace 1,094 passed, zero failed, 155 ignored. See `bench/SAMSUNG_PASS_100_WAVE36.md`. Goal incomplete.


## Wave37 terminal verification

183,850 / 194,286 executable definitions accepted (94.628537%); 118 ordinary identities gained and zero lost. Original-form coverage including no-body declarations: 195,033 / 205,497 (94.907955%). Remaining 10,436 executable definitions plus 28 no-body forms. All 32,420 sources parse; full compilation still fails with 50,662 diagnostics. Workspace 1,099 passed, zero failed, 157 ignored. See `bench/SAMSUNG_PASS_100_WAVE37.md`. Goal incomplete.


## Wave38 corpus verification

184,067 / 194,286 executable definitions accepted (94.740228%); 217 identities gained and zero lost. Original-form coverage 195,250 / 205,497 (95.013553%). All 32,420 sources parse; whole compilation still fails with 50,613 diagnostics. Remaining 10,219 executable definitions plus 28 bodyless forms. See `bench/SAMSUNG_PASS_100_WAVE38.md`. Workspace 1,108 passed, zero failed, 157 ignored. Goal incomplete.


## Wave39 candidate rejected by corpus gate

28 gained /18 lost identities; candidate is not accepted. Workspace1,116 passed but no-loss gate fails. Last accepted executable coverage remains94.740228% (wave38). Corrected original n0.q8355 emits complete, but13 direct contextual assignment regressions and5 downstream callers need repair. See `bench/SAMSUNG_PASS_100_WAVE39_CANDIDATE.md`. Goal incomplete.


## Wave40 corpus verification

184,349 / 194,286 executable definitions accepted (94.885375%); 282 gained, zero lost versus accepted wave38. All 18 wave39 losses restored. Both no-loss gates pass. 32,420 sources parse; full compilation still fails with 50,535 diagnostics. Remaining 9,937 executable definitions plus 28 bodyless forms. Workspace tests terminal exit 0: 1123 passed, zero failed, 159 ignored. See `bench/SAMSUNG_PASS_100_WAVE40.md`. Goal incomplete.


## Wave41 candidate rejected

27 gained/1 lost; no-loss gates fail. Accepted executable coverage remains wave40 94.885375%. Exact ECCurve#20966 selected-Phi monitor identity regression now repaired in MAIN; aggregate restoration pending. Workspace 1130 passed. See `bench/SAMSUNG_PASS_100_WAVE41_CANDIDATE.md`. Goal incomplete.


## Wave43 accepted corpus verification

Executable originals: 184,565/194,286 (94.996551%). Gain216, loss0 against accepted wave40; both wave40 and wave26 no-loss gates exit0. Remaining9,721 executable originals and28 bodyless forms.

All32,420 sources parse. Full uncapped javac still fails with 50,111 diagnostics. Workspace1,153 passed,0 failed,159 ignored; formatting, strict workspace/all-target Clippy, release and immutable artifact gates pass. Export241.79s, peak16,668,180KiB under concurrent tests; this is not an isolated speed comparison.

Integrated exact selected monitor identity, child-loop parent-header exits, captured Phi availability, shared cleanup, original protected source scopes, package-shadow static qualification, cleanup reference casts, handler-reentry allocation state and per-instruction allocation readability. Original identities retained.

JADX1.5.1 comparison remains a reference; Java compilation/runtime proof stays separate. Full APK compilation, native/resources and Android runtime are not complete. The100% goal remains active.

Evidence: `/tmp/opencode/samsung-pass-100-wave43` and `bench/samsung-pass-wave43-final.json`.


## Wave44 matching-runtime candidate rejected

Matching device/APK/framework provenance verified. Explicit runtime metadata candidate restores310 original identities but loses35 versus accepted wave43; both no-loss gates fail. Export terminal0, driver1. All32,423 files parse; whole javac still fails50,078 diagnostics. Candidate184,840/194,286 is not accepted coverage. Accepted baseline remains184,565/194,286 (94.996551%). Exact covariance bridge and typed-capture rethrow regressions are being repaired; runtime metadata does not attest javac availability. See `bench/samsung-pass-wave44-runtime-candidate.json`. Goal incomplete.

### Wave45 aggregate audit

Candidate95.2724% rejected:541gained but5accepted original identities lost (strong26loses4). All32,423 files parse, fullJava25 compile49,972diagnostics. Export416.63s,peakRSS29,469,876KiB; hashesstable. Full workspace1,165passed,0failed,161ignored; strictClippy/fmt/releasepass. Accepted baseline remainswave43; repairs in progress. See bench/samsung-pass-wave45-final.json. Goal staysactive.


### Wave46 integration and live corpus audit

MAIN now includes complete-table guarded cleanup fallback, same-method handler-aware checked source bridge, genuine capture-domain lineage refinement, result-producing private allocation arguments, exact post-chain allocation/Phi entry, and atomic original-handler checked argument contracts. The five wave45 losses were restored individually in actual output; aggregate no-loss remains unverified. Source-only bridge helpers remain attributed separately from original DEX methods.

Additional actual evidence: JsonTreeReader ordinary bodies11→27; TimeModel two constructors with128 JVM field-contract inputs; AttributeCertificateHolder.getNames and whole generated unit compile on JDK25 with genuine independently versioned BC1.78.1; LazyEncodedSequence.force and whole generated unit compile likewise. JDK17 whole-unit failures from existing Java25 constructor syntax are retained. These are not Android application execution or whole-APK compilation proofs.

Parent focused checks: reflection boundary1pass; private allocation2pass; protected allocation6pass plus explicit original-bytecode oracle1pass; capture lineage3pass plus two explicitly executed selected-handler JVM oracles. The initial corpus formatting gate required module sorting and was corrected. The full workspace/export/compiler/no-loss driver is running as session37928 at `/tmp/opencode/run_samsung_wave46_gates.py`; no candidate coverage is promoted before terminal artifacts. Accepted baseline remains wave43 (94.996551%).

Integration receipt: `bench/samsung-pass-wave46-integration-checkpoint.json`. Goal incomplete.


### Wave46 terminal candidate rejected

Full driver exit1: candidate185,562/194,286 executable definitions (95.509712%),1,048 gained but51 lost against accepted wave43. Fifty losses share the caught-domain common-bound regression; one checked invocation contract remains under diagnosis. Candidate is not accepted. All32,423 files parse; uncapped javac fails with49,691 diagnostics. Export481.82s,peak35,621,596KiB. Workspace1,183 tests passed; build gates and source hashes stable. Accepted baseline remainswave43.

Post-candidate MAIN now includes bounded local allocation argument regions and filled arrays (four parent JVM/heap-OOM tests passed), reviewed sibling-capture bounds and class-literal cleanup contracts. Their full aggregate verification is pending. JADX output is only comparative evidence; raw DEX scopes and compile/runtime checks remain authoritative. Goal incomplete.


### Wave47 frozen verification started

MAIN incorporates original-catch-aware transactional rollback, bounded constructor natural-loop tail placement, local allocation/filled-array argument regions, class-literal cleanup contracts, and proven sibling capture bounds. Original20433 restores all completeness flags in isolated actual replay; original313 now passes constructor structuring but remains unpublished until receiver proof. Parent focused rollback1 and loop-tail3 tests including JVM oracle pass. Earlier workspace preflight1190passed,0failed,163ignored; source changed subsequently, so it is not the final cohort gate. Fresh full gates started as session44068, script `/tmp/opencode/run_samsung_wave47_gates.py`; fmt exit0, workspace tests running. Accepted baseline remainswave43 until immutable full no-loss/compiler receipts. Goal incomplete.


### Wave47 accepted executable recovery baseline

Full driver44068 terminal0, all immutable source/runtime/APK/binary/classpath checks pass. Executable originals185,753/194,286 (95.608021%),1,188 gained,0 lost vs accepted43; both43 and26 no-loss gates pass. Explicit bodies185,324 plus429 separately proven enum equivalents. Remaining8,533 executable originals and28 bodyless forms. Workspace1,193passed,0failed,164ignored; all gates0.

All32,423 files parse; full uncapped Java25 compile remainsFAILED with49,591 diagnostics. Accepted recovery baseline is nowwave47; this does not establish compiled APK, native/resources or Android execution equivalence.100% goal remains active. Receipt: bench/samsung-pass-wave47-final.json.


### Wave48 verified recovery baseline

185,758 / 194,286 executable originals (95.61059469%): 185,329 explicit bodies and 429 proven enum equivalents. Five gained, zero lost against wave47; strong wave26 no-loss passed. 32,554 files parse without syntax errors. Full uncapped Java25 compiler still fails with 45,156 diagnostics (wave47: 49,591). Export 473.21 seconds; peak RSS 36,209,712 KiB. Workspace 1,201 passed, zero failed, 165 ignored. Goal remains incomplete: 8,528 executable originals and 28 original bodyless forms unresolved, whole-source compilation/native/resources/Android equivalence unproved. Receipt: `bench/samsung-pass-wave48-final.json`.


### Wave49 verified recovery baseline

185,918 / 194,286 executable originals (95.69294751%): 185,489 explicit bodies plus429 proven enum equivalents.160gained/0lost versus48; strong26 no-loss also passed.32,554sources parse0. Full uncapped Java25 compiler still fails45,055 diagnostics. Export484.45s; peak33,623,576KiB. Workspace1,209passed/0failed/165ignored. Goal incomplete:8,368 executable +28 bodyless originals unresolved; full Android/native/resource fidelity unproven. Receipt: `bench/samsung-pass-wave49-final.json`.


### Wave50 verified recovery baseline

186,105 / 194,286 executable originals (95.78919737%): 185,676 explicit bodies plus 429 separately proven enum equivalents. 187 gained, zero lost versus wave49; strong wave26 no-loss also passes. 32,555 sources parse without syntax errors. Full uncapped Java25 compile still fails with 44,884 diagnostics (wave49:45,055). Export507.07s, peak36,600,792KiB. Workspace1,218passed,0failed,166ignored. Source/APK/runtime/classpath/binary hashes unchanged. Goal remains incomplete:8,181 executable originals and28 original bodyless forms unresolved; native/resources/whole Android execution equivalence not established. Receipt: `bench/samsung-pass-wave50-final.json`.
