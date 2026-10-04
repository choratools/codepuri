# Java source release selection

`JavaOptions::source_release` defaults to `17`. The exporter also accepts `25`;
other releases return `OutputError::InvalidOptions` before analysis or delivery.
The CLI, `java_profile`, `java_corpus`, and `java_session_bench` expose the same
choice with `--source-release 17|25`. Each release-25 output unit carries a
`Required Java source release: 25` comment. Use a Java 25 compiler for that output.

Release 25 allows a constructor prologue before its original `super(...)` or
`this(...)` invocation. The exporter retains the original order rather than
moving a potentially throwing validation or captured-field assignment across
the parent constructor. See the official
[flexible-constructor-bodies rules](https://docs.oracle.com/en/java/javase/25/language/flexible-constructor-bodies.html).

The lowering still requires exact constructor dispatch, operand availability,
descriptor-pinned arguments, and complete constructor-family evidence. Early
stores must target a genuinely declared own instance field whose emitted
declaration has no initializer. Reads of the uninitialized receiver, escapes,
instance calls, inherited-field stores and missing field evidence remain
unsupported. This option does not establish complete class initialization or
remove native/reflection uncertainty.

The source release is fixed within `JavaSession`. Create a new session to change
it; an existing source snapshot cannot be replayed as another release.
