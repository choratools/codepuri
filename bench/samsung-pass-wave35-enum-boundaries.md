# Samsung Pass: exhausted enum representation routes (wave35)

Read-only evidence against original SamsungPass5.2.10.10 APK and frozen wave34 ledger. No source-routing changes or additional recovery claims. JADX executable `/home/jtjisgod/tools/jadx/bin/jadx` reports1.5.1.

## Accounting

`/tmp/opencode/samsung-pass-100-wave34/java25/method-coverage.jsonl`:2,262 EnumConstructorsUnavailable **method records**,390 enum owners. Private-constructor-only owners:34/257records;30have `$ENTRIES` (199records);4without `$ENTRIES` (58records). These counts are not2,262 independent constructors.

## Shared values-array identity: clone substitution disproved

Actual IntroState, classes2:

- clinit39319 invokes original array helper39318 at114, stores resulting array into `$VALUES`58476 at118, passes **that same SSA/register array** to `mq.b.a` at120, and stores returned entries object into `$ENTRIES`58475 at124.
- `mq.b.a`, classes3 method53801: null-check input at2; allocate `mq.c` at5; invoke ctor53802 with original input at7; return object at10. No clone.
- `mq.c.<init>`53802: null-check at2; superclass chain at5; `iput-object` original input into field57354 at8. Field57354 is **public final Enum[] b** (`0x11`); array elements remain mutable.
- `mq.c.r`53809: load same field at2, bounds-check at5, reload at8, `aget-object`10, return12.

An ordinary Java caller can cast returned entries to the genuine implementation and execute:

```java
((mq.c) IntroState.getEntries()).b[0] = IntroState.COMPLETE;
```

The original shared array makes `IntroState.values()[0]` return COMPLETE afterward. A replacement entries array obtained through `values()` changes only entries; subsequent `values()[0]` still returns NONE. Public-field exposure is established by original DEX; this is a valid possible client observation, not a claim that an existing APK caller performs the write.

The previously executed minimal JVM counterexample is preserved at `/tmp/opencode/wave23-entries-representation/IdentityCheck.java`. It compares shared vs cloned arrays and asserts this different outcome. Reflection there obtains the inaccessible compiler backing for the **oracle fixture**, not a proposed production implementation.

Pinned JADX confirms direct capture (`this.f26315b = entries`) in `/tmp/opencode/wave34-jadx-entries-c.java`, and factory forwarding in `wave34-jadx-entries-b.java`. Actual fresh raw dumps: `wave34-entries-factory.txt`, `wave34-entries-implementation.txt`.

Vanilla Java enum source cannot name javac's implicit backing field or replace its generated `values()` method. Prior decisive compiler failures are `wave23-entries-representation/access.log` and `override.log`. A second array/values clone would violate observable identity. Reflective access or compiler/classfile patching is **not adopted**: standalone emitted Java must carry its own faithful semantics.

## Four private constructors without entries: no qualifying recipe owner

| Original owner | DEX/private ctor | Payload descriptor after hidden String/int | Original first-constant obstruction |
|---|---|---|---|
| protobuf.FieldType | classes2 #10727,0x10002 | int, Collection, JavaType | GETSTATIC2/4 before hidden name6 |
| protobuf.JavaType | classes2 #11292,0x10002 | Class, Class, Object | VOID classliteral2 before name5; INT valueOf17 before name21 |
| threeten.temporal.ChronoField | classes4 #37795,0x10002 | String, TemporalUnit, TemporalUnit, ValueRange | GETSTATIC2/4 and ValueRange.of11 before name15 |
| threeten.temporal.JulianFields$Field | classes4 #37899,0x10002 | String, TemporalUnit, TemporalUnit, long | GETSTATIC2/4 before name11 |

There is **no numeric-only payload constructor** among these four. FieldType/JulianField having a scalar argument does not erase their reference effects. Subsequent constants additionally reuse prior reference payloads; rereading them is not an identity proof.

Pinned JADX1.5.1 fails all four enum restorations with `Init of enum field ... uses external variables`, removing enum modifier/Enum superclass. Exact outputs: `/tmp/opencode/wave29-next-jadx-FieldType.java`, `wave31-jadx-JavaType.java`, `wave27-next-jadx-ChronoField.java`, `wave34-jadx-JulianField.java`. This is not a faithful working Java-enum precedent.

The tested Java25 private recipe-constructor candidate retains allocation before payload operations and original payload evaluation order, but javac eagerly supplies hidden name/ordinal before calling it. Original INT name resolution follows valueOf17. Production `RecipeEvidence::recipe_hidden_inputs` remains default-closed; candidate subset compilation is not original equivalence. No owner qualifies under the required failure/resolution-order contract.

Scalar IntroState39320 has pure int payload and compatible hidden ordering, but its separate shared `$ENTRIES` array barrier kills whole-owner publication. ChromeBrowserType27825 has String payload3 before hidden name5, so treating literals as free would additionally relax resolution/OOM timing; it also has `$ENTRIES`.

## Alternative representations — explicitly not adopted

An original bytecode/managed-IR output could retain original constructor flags, Enum inheritance, operation order and shared array directly. A future VM/classfile backend could represent semantics unavailable to vanilla Java source. A plain final Java class with hand-written enum-like methods would change genuine Enum inheritance, reflection, dispatch and often flags; it is not equivalent merely because it looks readable. Neither alternative is currently used to admit these definitions as faithful Java enum recovery.

This is a bounded source-representation limitation, not a claim that all remaining decompilation is impossible. Continue other independently actionable method-lowering failures instead of repeating clone substitution, access-flag rewriting, or unproved recipe scheduling.
