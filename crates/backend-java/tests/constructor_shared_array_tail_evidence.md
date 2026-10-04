# Bounded exclusive shared constructor suffix

Original witness: classes4.dex method21183,
`Lorg/bouncycastle/math/ec/LongArray;-><init>([JII)V`, code offset2746376,
size56, no try ranges. Six distinct retained caller identities in Wave14 have
an explicit unresolved reference to this constructor. Original SSA has two
conditional tests converging on one NewArray / field-store / System.arraycopy
return tail; the alias arm stores the original array and returns. There are no
Phi joins in this constructor.

Shortest isolated seeded-original lowering failure was `unsupported constructor
branch suffix`. The global coverage set incorrectly also served as a cycle
oracle, rejecting the second mutually exclusive source occurrence of the tail.

The suffix now distinguishes original-ID coverage from active execution path.
Acyclic no-Phi tails can be cloned inside exclusive Java branches; all generated
occurrences retain branch-local value availability and original operation order.
Active-path cycles still reject. The shared lowering budget is bounded by
`min(8 * original block count, 4096)` and depth128. Exception edges and Phi state
remain unsupported. Existing local final path and chain proofs are unchanged.

Three focused tests verify one copy effect per executed arm, input alias on
full-range use, copy independence on partial range, negative allocation before
copy, range-error behavior, blank-final Java definite assignment, cycles and an
exponential shared DAG exhausting the budget.

The exact original method was independently parsed and descriptor-seeded from
the Samsung APK, then lowered using DEX-resolved method/field descriptors and
known JDK Object/System declarations. Its emitted constructor is unmodified in
`/tmp/opencode/ctor-readonly-probe/LongArrayConstructor.java.txt`. Wrapped only
with the original m_ints declaration and a runtime harness, it compiles/runs on
JDK17. Artifacts: `/tmp/opencode/ctor-readonly-probe/longarray-jdk`.

This is constructor-body subset evidence, not a complete APK/class rebuild or
production coverage gain. Parent must verify actual production publication and
closure. Global native/reflection final-write uncertainty is not waived.
