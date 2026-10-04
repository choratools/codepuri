# Samsung Pass wave26: checked generic super selection

This is focused implementation evidence, not a new aggregate coverage result.
The latest completed SDK corpus baseline remains wave25: 181,096 / 194,286
executable originals (93.211039%), with 52,151 uncapped javac diagnostics.

The source selection proof now parses bounded genuine method and class
Signatures, checks exact descriptor erasure, and follows raw generic superclass
ancestry. A nongeneric intermediate binding `Box<String>` retains that binding:
an original-erased `Object` argument is not accepted as a Java `String` argument.
The metadata chain uses the first authoritative owner; a later classpath cannot
fill missing generic evidence in a shadowing declaration.

The root reran the focused tests against the current worktree and genuine
JDK17/Android35 inputs:

- `java_super_parameterized_actual`: 2 passed, including all four original
  super-call sites in Samsung methods 11811, 11812 and 8716; 10.21 seconds.
- `java_super_parameterized_signatures`: 1 passed; genuine JDK compilation,
  javap target-descriptor checks and raw-list runtime, plus a bound-parent
  negative and authoritative-provider precedence; 5.98 seconds.

The pinned jadx1.5.1 output for the same methods uses `super.addAll(...)` and
`super.setSystemGestureExclusionRects(...)`. This identified a real overly narrow
selection gate. The comparison does not establish whole-class compilation or
Android execution. Method type parameters, unavailable Signature evidence and
unproved overload/bridge/access/throws cases still reject.

Exact root verification: `/tmp/opencode/wave26-root-super-verify.log`.
Worker implementation/evidence: `/tmp/opencode/wave26-super-signature-report.md`.
Full SDK export and exact identity comparison remain pending other active
regression repairs. The 100% goal remains active and incomplete.
