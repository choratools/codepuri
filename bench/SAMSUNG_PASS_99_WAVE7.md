# Samsung Pass wave7: measured recovery and explicit compiler limits

The active 99% goal remains incomplete. This checkpoint is the frozen full
Samsung Pass5.2.10.10 APK export, not the newer wave8 worktree.

| Original accounting | Count |
|---|---:|
| Original defined methods | 205,497 |
| Code-bearing definitions | 194,286 |
| Explicit complete emitted bodies | 177,907 |
| Separately proven enum equivalents | 333 |
| Genuine retained bodyless declarations | 11,177 |
| Rejected definitions | 16,080 |

Accepted bodies/equivalents total178,240: **91.741042%
of code-bearing definitions**, **86.736059% of all
definitions**. Original-form retention is92.175068%.
Another **14,104** code-bearing recoveries are needed for99%; genuine bodyless
methods are never counted as recovered bodies.

## Exact original-identity delta

Against the wave6 cold ledger (every status identical to wave5), **1,942 gained,
80 lost**, net **+1,862**. Ordinary bodies141,186, constructors31,449 and static
initializers5,605. All205,497 original identities retain their denominator.

Losses are explicit:38 body-closure failures,37 callee checked-contract failures,
one Boolean-to-Int backend regression,one checked invocation failure and three
methods now without a source unit. The Boolean regression is being repaired in
wave8; do not assume the other withdrawals are all justified without their
individual original-method evidence. Exact rows are in
`samsung-pass-99-wave7-lost-identities.json`.

There are31,192 source files:five added and one removed
(`retrofit2/k$type.java`). All31,192 unchanged output files pass the Java25
syntax gate. Full uncapped javac **still fails**:52,427→56,705 diagnostics.
Matched old-file diagnostics52,424→56,692; new-only files add13; the removed
file previously hadthree. The dominant increase is4,353 cannot-find-symbol
messages, especially newly recovered enum switch-table initializers whose
source dependencies remain incomplete. Five unchanged representative units
compile; this is not whole-class, APK or Android-runtime completion.

## Integrated correctness repairs

- Synthetic SSA predicate placement now uses current positions after handler
  insertion; original da/m0.e1 branch reads the actual cmp-long result.
- Contextual Phi constraints preserve exact2/-1 integer literals and descriptor
  backed View consumers, including original TabLayout.C0.
- Original allocation/argument effect order uses primitive switch scaffolding
  without new helper frames or objects. WelcomeFragment.getViewHolder now
  emits; its bounded original-order bytecode differential passes10 modes.
- Static subset lowering retains original exceptional scopes and exactly-once
  final stores. Original RoundingMode table whole unit compiles unchanged.
  This does not certify a complete constructor/initializer family.
- Catch-all swallowed invokes prune only unnecessary throws-domain dependencies;
  captured rethrows retain their exact original event domain.
- Genuine alias-distinct non-overrides no longer contaminate abstract source
  contracts. Missing metadata and external no-throws bounds remain enforced.
- Bounded exact super-selection repairs pass10 genuine original references.

Original focused-source hashes are recorded in
`samsung-pass-99-wave7-focused-sources.json`. Backend/portable JVM tests prove
specific effects, identities and dispatch; they do not prove the entire APK.

## Frozen verification and performance

**856 workspace tests passed, zero failed,90 ignored**,147 suites. Full strict
workspace/all-target Clippy, fmt and release build passed. Explicit ignored
original APK probes were run separately and are not relabeled as default tests.
Source hashes and frozen executable matched through export completion; later
wave8 source edits are outside this checkpoint.

The30-worker export took313.853576s (315.84s whole process), peak15,692,352KiB
(~14.97GiB), zero swaps and exit0. Source text314,148,091bytes. Concurrent tests
and host activity make this an uncontrolled cold timing, not a speedup claim.

Enabled cache counters:269,365 SSA hits /194,286 misses;5,623,036 member-resolution
hits /403,704 misses. The older same-session warm replay result remains separate.
Source-throws preparation36.405282s; file delivery1.481963s.

Scheduler profiling observes3,513.20 active task-seconds and3,873.57 admission
wait seconds with a240-result window. Active task-seconds are elapsed work time,
not CPU time. DEX2 DBHandler creates a measured head-of-order stall with239
completed results waiting andone task active. A fixed-duration window simulation
reproduces249.752s versus250.428s measured scheduler time; its larger-window
results are hypotheses for a controlled identical-output experiment, not actual
speedups.128GB RAM provides headroom for pending results, not automatic speed.

## Evidence and next current edits

- `samsung-pass-99-wave7-{environment,inventory,coverage,summary}.json`
- `samsung-pass-99-wave7-{compiler-summary,diagnostic-delta,source-delta}.json`
- `samsung-pass-99-wave7-{workspace-tests,clippy,fmt,build,run,time,stderr}.txt`
- Frozen output `/tmp/opencode/samsung-pass-99-wave7/java25`
- Full uncapped compiler/identity/hash evidence:
  `/tmp/opencode/samsung-pass-99-wave7/validation25`

Wave8 separately repairs original entry absorption in CFG merging and the
missing invocation predecessor in entry-loop SSA, shared typed-handler capture
provenance, the concrete Boolean declaration regression and protected argument
copies. These have focused witnesses; no fresh wave8 aggregate coverage is yet
claimed. Samsung framework method declarations absent from Android35 remain a
real external metadata requirement. Native libraries and Android execution are
unverified. The99% goal remains active.
