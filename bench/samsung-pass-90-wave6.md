# Sixth measured wave and pending source-scope repair

Immutable205,497 original definitions.140,632 strict bodies+300enum equivalents
=140,932accepted (68.581050%).51,638gained/zero lost relative to original baseline;
44,016remain for90%.11,140bodyless declarations are excluded.
All34,579classes visited;29,672files written;147.658seconds;errors/skips0.
Exact source syntax passes. Full uncapped javac fails69,735diagnostics.

This snapshot adds attested common-ancestor Phi typing and exception declaration
planning. Actual biometricg.a#9301 now emits a strict body; q9h.i#55648 remains a
semantic-only body with missing dependencies. kr/i.a#51424 and its callers reach
strict body publication. A direct javac of that original method body exposed a
label-scope bug: an allocation local was not visible at its Phi edge copy.

Subsequent head fixes the source-scope defect: hoisted allocation slots are
read-forbidden before the unique constructor and released only after that exact
call. Writes do not read the uninitialized slot; argument folding uses the same
pending state. Repeated initialization and premature reads reject. JDK object
order fixtures and branch throw tests pass. The exact unchanged kr/i.a method
body now compiles under its original aliased owner in a declaration-only test
container with no fabricated dependencies; this is not whole-class compilation.
Artifact:/tmp/opencode/kr-i-wave7-method/i$type.java

Full-source snapshot evidence:-wave6.json,-inventory.json,-validation.json,
-run/stderr/time.txt. Actual root:
/tmp/opencode/decomphify-java-corpus/run-1790933654597570040-3992744/com.samsung.android.samsungpass

Wave7 full30 remeasurement and workspace suite are running; do not count future
coverage or claim global compilation. Goal remains active and incomplete.
