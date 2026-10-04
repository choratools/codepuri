# Contextual null dispatch evidence

## Confirmed backend defect

`Emitter::operand` replaced a proven category-one zero with a bare `null`
expression while returning the expected reference type as its expression type.
The Invoke arm consequently skipped its descriptor-pinning cast. Java's null
type does not carry that descriptor, so this is not valid overload evidence.

The fix gives the contextual expression its actual expected source type. It
does not change hierarchy compatibility, phi declarations, path availability,
typed local identities, or the resolver's dispatch attestations.

`contextual_null_dispatch.rs` uses existing JDK methods and no helper overload
declarations: the same SSA zero feeds `Arrays.hashCode(int[])` and
`String.valueOf(Object)`.

* Old expressions: `Arrays.hashCode(null)` fails JDK17 javac with
  `reference to hashCode is ambiguous`.
* An array-only repair compiles but `String.valueOf(null)` selects the
  `char[]` overload and throws `NullPointerException`.
* Both expressions pinned to their original descriptors compile and execute,
  returning the string `"null"`.
* Three malformed nonzero constant inputs remain unsupported (3/3).

Validation: JDK 17.0.13+11, `cargo test -p backend-java` with JAVA_HOME and PATH
set to that JDK: 99 passed, 1 ignored. The final strengthened differential test
also passes separately. `cargo clippy -p backend-java --all-targets -- -D
warnings` passes.

## Samsung attribution limits

Input evidence is `bench/samsung-pass-five-final-per-file-errors.json`:
24,749 common files, 38,158 old errors, 48,091 final common-file errors,
3,211 new-only errors, 51,302 final total errors. Message deltas are candidate
rankings, not counts of root causes repaired by this patch.

Inspection of the high-count families found publication/dependency problems:

* `createBuilder` (76 positive same-file message deltas), byte-array `hashCode`
  (72), `decode32` (46), and `littleEndianToInt(byte[],int)` (44) cannot be
  counted as contextual-null repairs without the original selected overload
  declarations being available.
* The three new `reference to of is ambiguous` diagnostics occur in
  `DateTimeUtils`, `OffsetDateTime`, and `ZonedDateTime`. The retained
  `OffsetDateTime.java:159` has seven integer arguments, not null arguments;
  javac reports competing int and Month overloads while `Month.java` is absent.
* Eleven new `Node cannot be converted to Element` errors in `Element.java`
  include lines 37/43: bridge bodies call covariant overloads whose retained
  comments explicitly state the selected original signature is absent.
* `Toolbar$g` extends `androidx.appcompat.app.a$b`, whose source is absent;
  its casts to MarginLayoutParams cannot justify weakening hierarchy checks.

Retained source root:
`/tmp/opencode/decomphify-java-corpus/run-1790897304245626033-3778956/com.samsung.android.samsungpass/sources`.
Exact diagnostic locations are in
`/tmp/opencode/samsung-pass-five-final/validation/full-javac.log`.

**Confirmed Samsung diagnostic classes/errors fixed: none established.**
This patch has a genuine backend/JDK differential regression, but does not
fulfil the requested original-verbatim Samsung before/after regression gate.
No Samsung total-error reduction or selected-source compilation success is
claimed. No integration API is necessary for the null-expression fix.
