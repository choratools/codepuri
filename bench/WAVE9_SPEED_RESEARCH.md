# Wave9 speed research: contextual field proofs

This is a read-only source finding, not a measured speed improvement.
The active wave9 recovery export freezes source and retains the existing caches.

The remaining candidate is a completed contextual field-use proof cache.
The declaration lookup cache already exists, but requirements, source-use and
receiver-retention paths independently resolve the same contextual field.

Evidence paths: `java_integration.rs` requirements/field receiver,
`java_field_publication.rs::source_use_indexed`, and
`java_external_fields.rs` actual-owner, subtype and hidden-name checks.

Key one export-owned cache by original symbolic FieldKey, original caller
descriptor, optional receiver descriptor and opcode staticness. Freeze provider
and alias identity in the owning cache. Retain typed failures. Do not include
mutable publication or dependency-closure acceptance, and do not cache recursive
path-dependent partial results. Symbolic and actual declaring owners remain distinct.

Next decisive measurement: outermost call time, call count and unique full keys
for the contextual path on DBHandler/PrefManager. Do not double-count nested
times or equate summed task-stage time with wall time. Source/output parity and
protected receiver/shadowing regressions are required before a speedup claim.
