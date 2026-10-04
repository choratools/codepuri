# Samsung Pass 90 percent recovery plan

The original-definition denominator stays 205,497. The target is at least
184,948 original identities with complete bodies or independently proven enum
equivalents under the existing strict ledger rules. Bodyless declarations,
stubs, omitted bodies, and unresolved semantic publications do not qualify.
This coverage target does not itself establish whole-application compilation.

## Verified starting evidence

The latest completed source-closure ledger contains 89,026 complete bodies and
268 enum equivalents: 89,294 accepted identities, leaving a 95,654-identity gap.
See `samsung-pass-90-rejections.json` for exact source ledger hash, category
counts, and representative failures. Preserve existing worktree changes.

## Execution waves

1. Repair comparison operand contextual typing and typed class literals;
   reconcile exception throw classification with lifted operation semantics.
2. Recover initialization and field declarations using field-specific evidence;
   do not reinterpret arbitrary native/reflection effects as no-write evidence.
3. Repair source declaration closure and missing methodless-class publication;
   preserve the strict/semantic distinction in all measurements.
4. Extend real exceptional/overlapping control-flow and constructor patterns
   selected from the newly exposed rejection inventory.
5. Repeat targeted reproduction, semantic execution, and full 30-worker export
   until the target is verified or an actual external blocker is established.

## Per-wave gates

- Focused positive and negative semantic regression tests, including actual
  original Samsung identities when artifacts permit.
- Compile and execute generated fixtures using the local JDK 17.
- Workspace tests, formatting, and strict all-target Clippy.
- Fresh full Samsung Pass export with 30 total compute workers and the same
  explicit JDK17/Android35 metadata.
- Compare identity sets, original denominator, first rejection reasons, syntax
  validity, and representative whole-unit compilation. Record losses explicitly.
- Keep whole-export compiler diagnostics and unavailable initialization visible;
  never suppress these to improve a percentage.

The goal remains active until fresh authoritative evidence meets the original
90 percent requirement; intermediate improvements are not completion.

## First measured wave

Fresh ledger: `bench/samsung-pass-90-wave1.json`, original denominator unchanged.
Accepted identities increased from 89,294 to 104,493 (50.8489 percent): 15,199
gained and zero lost. The remaining target gap is 80,455 identities. This is
strict body/equivalence coverage, not whole-class initialization or compiled-app
coverage. All 29,493 exact generated sources passed the JDK17 parse gate.

Changes cover contextual comparisons, real class literals, typed Dalvik
three-way comparisons with NaN/signed-zero semantics, exception throw
classification, and proven complete constructor families without instance
finals. Constructor subset publication does not certify class initialization.

The export completed with zero output errors in 64.43 seconds, peak RSS
2,907,312 KiB. Targeted backend/DEX/facade tests, formatting and strict workspace
Clippy passed. Full uncapped javac completed with 56,677 diagnostics versus
57,245 at baseline. Whole-export compilation still fails; exact-source results
and commands are in `bench/samsung-pass-90-wave1-validation.json`.

The next measured first-rejection groups are whole-input final-write uncertainty
(22,921), required complete-body closure (15,802), and unresolved Null locals
(5,401). Null local hoisting is being corrected from definition and complete
consumer evidence; initialization/closure causes are under targeted review.

## Second measured wave

`bench/samsung-pass-90-wave2.json` establishes 121,559 accepted original
identities (121,271 explicit bodies plus 288 proven enum equivalents): 59.1537
percent. Relative to the original baseline, 32,265 identities gained and zero
were lost; 63,389 remain to reach the target. All 29,576 exact sources parse.
Full uncapped javac still fails with 62,702 diagnostics; expanding body coverage
has not established globally complete declarations or initialized classes.

Changes: definition-proven Null/Zero local hoisting and split consumers, exact
instance-final constructor-family body proofs, branch/shared-tail allocation
materialization without uninitialized-local hoisting, and genuine Throw
terminals with checked-exception proof. Actual Samsung ctor subsets compile/run,
and missing/double assignments reject. Actual application methods and a protobuf
throwing helper pass the production body route.

An initial second-wave attempt encountered a static-initializer successor
panic after Throw support. It is excluded from coverage. A no-successor
diagnostic and reproducing regression were added, then the successful full
retry visited all 34,579 classes with zero export errors in 72.56 seconds;
maximum RSS was 3,095,056 KiB. Failed-run and retry logs remain separate.
Focused tests, strict workspace all-target Clippy, formatting and release build
pass; full source/identity hashes and compiler outcomes are retained.

Next work investigates exact allocation-before-argument ordering and ordinary
invoke-super target proofs. No effect/OOM-order gate is relaxed merely to raise
coverage. The active 90 percent goal remains incomplete.

## Fourth wave checkpoint

63.1664 percent accepted, 40,511 baseline identities gained, zero lost. The
allocation skip-plan regression is corrected. Remaining target55,143. Full
javac still fails64,521 diagnostics. Sequential disjoint try scope proof is the
next bounded experiment; no percentage or completeness claim before measurement.
