# Bounded constructor prefix expression recovery

Production entry remains `plan_constructor` / `plan_constructor_reachable_subset`;
no global initialization attestation or declaration modifier changes were added.

The bounded expression tree accepts definition-proven contextual null, array
allocation and metadata-backed static field read only as chain arguments. Event
traces preserve original resolution/allocation/read order, exact operand identity,
single reads for effectful values, and whole-arena escape rejection. Literal
String reuse is special: the central native Java literal renderer supplies the
same canonical interned identity without the former fresh `String` allocation;
only repeated loads of the exact same proven SSA literal event are collapsed.
Class literals and distinct String events retain original resolution order.

## Terminal focused gates

`constructor_prefix_expressions`: four JDK17 compile/runtime tests: descriptor
null overload, nested byte array rank, failed allocation before parent call,
static field class initialization order. Negatives cover array escape,
field-order reversal, field escape/missing metadata, and nonzero-as-null.

`constructor_argument_allocations`: six tests, including canonical repeated
String identity, literal/allocation reversed-event rejection, two allocation
argument order, missing delegation metadata and unreachable extra read.

## Actual Samsung Pass filters

Immutable parent release reused; no binary rebuild or full corpus run by helper.
Three bounded commands returned exit 0 with genuine JDK17/Android35 classpath:

- `/tmp/opencode/samsung-prefix-status`: original constructors 24984, 24985,
  24986 written with `complete_body=true`, using descriptor-pinned nulls.
- `/tmp/opencode/samsung-prefix-asn`: original constructor 5492 written with
  `complete_body=true`: `this(p0,p1,p2,new byte[11][])`; exact closed constructors
  5491 and 5493 also written. Other constructors retain honest rejection.
- `/tmp/opencode/samsung-prefix-blake`: prefix static field expression blocker
  crossed, but constructors 9379/9380 remain `required complete method body
  unavailable` due to downstream chain. No completed-body gain claimed there.

The filter sources are not established as whole-class compiled Android units.
JDK runtime evidence applies to deliberately bounded semantic fixtures, not
Samsung device execution. Native/reflection global final-write uncertainty,
unsupported branch/catch prefix effects and unavailable chain targets remain
rejections.
