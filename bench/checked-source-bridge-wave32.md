# Caller-local checked-source bridge (bounded wave32)

The source projection and runtime escaping domain are now separate facts. Runtime facts survive source-only override conflicts, but are absent for unproved exception analysis. No global throws erasure is performed.

Production admission is limited to genuine ordinary input class methods with an unavailable projected source contract, no encoded handlers, no monitor/synchronized metadata, and no object-allocation recipe. Existing valid derived declarations remain unchanged: otherwise original checked catches in other callers could become unreachable to javac. Restoring already-valid declarations (MessageUtil 14395) is bounded backend feasibility only, deferred pending graph-wide source-contract compatibility.

Every necessary uncovered invocation or explicit throw is matched to the original InsnId, operand identities, dispatch, target signature/current callee throws, and proved runtime domain. Original targets, argument order and normal result materialization remain unchanged. Invoke results are declared before a callsite-local try; the catch throws a value-returning same-owner private generic helper, satisfying Java definite assignment. The helper's type parameter erases to Throwable; no narrower runtime cast is introduced.

The frontend freezes a collision-free helper name against original/aliased declarations and genuine ancestor methods. The name inventory is cached by original owner and uses the once-built exact InputIndex. The helper is attached only after successful complete declaration emission, attributed as synthetic and never inserted as an original method-ledger identity. Failed backend recipe adoption retries the previous caller projection. Constructors, static initializers, interface/annotation/enum declarations, encoded handlers, monitor scopes and missing typed facts remain excluded.

## Evidence and limits

`crates/backend-java/tests/checked_source_bridge.rs` emits the Java value-returning caller through the real backend. JDK17 and pinned JDK25 compile and run the unchanged emitted body/helper, retaining checked/unchecked/OOME object identity, captured stack, suppressed exceptions and target call count. Missing runtime facts, a changed target throws clause, and constructor identities reject.

The independently retained standalone `bench/checked_source_bridge_probe_wave31.py` additionally checks argument/target ordering, cleanup override effects, value-return definite assignment, javap erased helper descriptors and no checkcast. It is boundary/compiler evidence, not APK recovery evidence. Preconstructed OOME does not measure real memory exhaustion.

Generic helper frames affect reflection/live-stack/bytecode observability. Captured exception identity/message/stack/suppressed preservation does not establish universal equivalence under stack inspection, debuggers, allocation failure or JVM resource limits.

Actual classes2 originals selected for the production/bounded test: Fido Supplier `16702 get():Object` and MessageUtil `14395 toJson(Message):Optional`; both have no encoded handlers. Production 16702 must retain its original no-throws Supplier contract; 14395's existing source projection must remain intact. Exact original-source fragment evidence is written by the bounded ignored test, not a full export.

No full-corpus run, recovery-percentage claim or complete-APK compilation is established by this change.

## Terminal bounded results

- Exact production/bounded original-method test passed: `actual_fido_supplier_and_messageutil_original_contract_bridge`, 99.29s including the proved dependency-domain closure. Fragments: `/tmp/decomphify-actual-checked-bridge-1204676`.
- Independent runtime-domain retention regression passed despite source override invalidation.
- Backend generated caller test and proof-negative test passed (JDK17 and JDK25).
- Backend/decomp all-target strict Clippy passed.
- `bench/checked_source_bridge_actual_wave32.py` compiled and ran both unchanged original method fragments on JDK17 and pinned JDK25, 46 assertions each. The Supplier's previously emitted original constructor and fields were retained; target dependencies were explicitly fabricated boundary fixtures. No original method body was edited. Artifacts: `/tmp/opencode/checked-source-actual-wave32-mjzzsiww/report.json`. javap confirms the private helper erases its input to Throwable and contains no checkcast.
- This proves the exact original emitted body under the stated boundaries, not genuine whole-APK dependency closure or a full-corpus no-loss result. 14395 is not a production signature-restoration/recovery claim.
