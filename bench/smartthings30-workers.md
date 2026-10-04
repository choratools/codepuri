# Java class worker controls

`JavaOptions.workers` defaults to 1. The Java CLI accepts `--workers N`.
The `java_corpus` runner uses `--workers N` as a total compute budget and
`--app-workers A` (default 1) for concurrent APKs. Each APK receives
`floor(N / A)` class workers. To retain the earlier APK-only scheduling,
specify `--workers N --app-workers N`.

Class tasks share immutable aliases and per-DEX metadata. At most one batch
of class results is retained; callbacks are serialized in original DEX/class
order. Method closure remains local to each class. Limits count accepted
compilation units, including classes with no emitted source correctly.
