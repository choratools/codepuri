# Ordinary throw-object initialization inside a constructor suffix

Actual Wave14 witness: classes4.dex original method index 21256,
`Lorg/bouncycastle/math/ec/SimpleBigDecimal;-><init>(Ljava/math/BigInteger;I)V`,
code item offset 2749664, size52, no encoded try ranges. The original SSA has
Object.<init>, negative-scale branch, ordinary IllegalArgumentException
allocation, String literal, its exact constructor, then throw. The successful
branch assigns BigInteger and scale fields and returns. Eleven unresolved
constructor-reference records in the existing output point at this identity.

The suffix emitter previously rejected every Special invoke, including this
ordinary exception constructor. It now requires a genuine NewInstance root and
an existing typed ObjectPlan whose exact initializer equals the current
instruction. The ordinary object hook still validates owner/signature, pairing,
argument evaluation order, initialized state and checked-exception coverage.
Original this/super second chains and unpaired/repeated targets remain rejected.

The JDK17 fixture uses the same constructor signature and control/data shape;
unchanged generated source compiles and runs. It retains BigInteger identity,
scale2 and negative-scale original message. Six negative mutations reject:
uninitialized escape, unpaired receiver, owner mismatch, repeated initializer,
second superclass chain and missing normal-path final assignment.

No global/native final-write uncertainty was waived. Local body support is not
whole-class initialization proof. No actual Samsung constructor gain or full
APK compilation is claimed until the parent production run verifies the exact
original method identity. Read-only raw probe log:
`/tmp/opencode/ctor-readonly-probe/wave14-ssa.log`.
