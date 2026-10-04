# Java backend

`emit_method(&MethodIr<SsaForm>, &JavaMethod, &ParameterBindings, &impl JavaResolver)`
returns `SourceResult::Complete { source, method_name, parameter_names }` or
`SourceResult::Unsupported(Unsupported)`. Complete source is a public method
declaration to insert into its declaring class. Unsupported results contain no
source and must not be counted as decompiled methods. There is no stub generator.

## Caller contract

* `JavaMethod` supplies owner, name, static flag, return type, and descriptor-order
  parameter types, and declared checked exceptions (`throws`). `JavaType` models Java primitives, source-level qualified class
  names, and arrays. Nested classes use source dots, not descriptor separators.
* `ParameterBindings` supplies one `ValueId` per parameter (wide values are one
  binding), an optional instance receiver, and optional naming hints. Unknown
  parameter types are refined by the signature; known types must agree exactly.
  Explicit bindings also cover synthetic `TAG_INITIAL` opaque definitions outside
  the CFG. Inside the CFG, only read-free, effect-free opaque definitions whose
  writes are explicitly bound parameters are elided. Other opaque instructions
  are unsupported. No frontend tag interpretation or DEX dependency is required.
* `JavaResolver` resolves `MethodRef` to `JavaMethod`, `FieldRef` to `JavaField`,
  `ClassRef` to a source class name, and `StringRef` to UTF-16 units. It can be
  implemented over any frontend's metadata pools. `instruction_type(TypeRef)`
  additionally resolves exact array/cast/instance-of instruction descriptors and
  frontend-owned numeric conversion targets. Missing answers fail closed.
  Resolver answers must be stable.
* Names on declarations are sanitized and parameter names made unique. Resolved
  external names must already be valid Java names; changing them would change
  linkage. Package/class qualifier roots are reserved against local shadowing.

## Supported subset

Linear, unconditional CFG paths ending in return, with no unreachable blocks or
exception edges. Supported instructions: typed constants (including null/zero
with a resolved concrete type), moves, 32/64-bit arithmetic and shifts, floating
arithmetic, Boolean/integer bitwise operations, comparisons, negation/not,
static/virtual/interface invokes, static/instance field get/put, and return.
Typed array allocation, get/put/length, primitive payload fill, numeric/reference
casts, and instance-of are supported. Multi-dimensional descriptors allocate only
the requested outer dimension. Byte/short/char stores explicitly truncate; narrow
loads may widen to int. Boolean and wide/floating components remain exact. Fill
checks null and whole-payload bounds before mutation, and raw-bit constants retain
their existing representation.

Object allocation requires an adjacent special `<init>` invocation with the exact
allocated owner, void/non-static descriptor, and already materialized arguments.
The pair emits one `new Owner((DescriptorType) argument, ...)` at the allocation
site. Constructor overload selection and argument order are explicit. Nonadjacent
initializers, uninitialized uses, aliases before initialization, missing metadata,
and undeclared checked exceptions are unsupported.

Each SSA definition becomes an initialized, explicitly typed local. No expression
is inlined, duplicated, or reordered: even unused invoke results and field reads
are evaluated. Integer and long operations retain Java overflow and division
semantics. Float/double constants use raw-bit conversion calls (including signed
zero and NaN payloads). String constants use UTF-16 char arrays followed by
`intern()` to preserve literal identity, escapes, and unpaired surrogates.

Type compatibility is intentionally exact: no guessed inheritance, narrowing,
ambiguous Zero/Null/Unknown local types, or implicit numeric promotion. Supported
receiver metadata must name the exact source receiver type. Arithmetic on narrow
byte/short/char results is unsupported rather than guessing truncation semantics.

## Limitations and integration

The integrated region planner supports acyclic branches, Phi edge copies, and
single-latch reducible pretest while loops with a linear evaluation prefix and
one normal exit. Header effects execute on every test; entry/backedge Phi copies
use simultaneous snapshots. Nested acyclic conditionals and sequential loops
are supported. Nested loops, break/continue paths, multiple entries/latches,
posttest loops, switches, exceptions, monitors, and opaque filled-new-array
instructions remain unsupported. See `bench/JAVA_WHILE_LOOPS.md` for compiled
execution evidence and final-build APK measurements.
Constructor declarations use dedicated lowering; general
special/super dispatch and polymorphic invokes remain unsupported.
This backend does not reconstruct access flags, annotations, generics,
classes, or imports. Referenced classes/members
must be accessible and source-representable. Caller-supplied checked-exception
declarations are emitted; invoke exception types must be declared by exact name
on the enclosing method (no guessed exception inheritance). `decomp-emit` is not needed for this small
statement-only subset; typed locals avoid expression-precedence reconstruction.

## Verification

`cargo test -p backend-java --locked --offline` runs focused semantic source
checks and rejection tests. When both `javac` and `java` are available, it also
compiles and executes fixtures covering arithmetic overflow, wide parameters,
moves, invoke/field evaluation order, unused calls, floating raw bits, and UTF-16
string identity. Missing Java tools explicitly skip only those runtime checks
(visible with `-- --nocapture`).
