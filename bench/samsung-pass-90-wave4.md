# Samsung Pass: fourth verified 90 percent goal wave

Original denominator remains 205,497. Complete explicit bodies: 129,517.
Proven enum equivalents: 288. Accepted: **129,805 (63.166372 percent)**.
Target: 184,948; remaining: 55,143. Bodyless declarations (11,140) and unresolved
semantic-only bodies do not count. Compared with original source-closure
baseline, **40,511 identities gained, zero lost**.

Wave 3 reached 61.50% but regressed 1,570 identities. Matching selected plans
for argument skipping and initializer emission fixes the undefined-value
regression. Seven allocation tests and original Samsung newThread/default
layout/saved-state methods reproduce restoration. Original wave 3 evidence is
retained, not overwritten.

Checked-exception coverage uses attested subtype edges and unchecked
RuntimeException/Error descendants. Undeclared IOException remains rejected.
Real method584 Lab/b.run now emits; methods119/120 clear the wrong exception gate
but still hit allocation-order blockers. No exceptions or stub bodies are
fabricated. JDK runtime checks preserve thrown object identity.

Full30 export: 76.463 seconds, 3,181,756 KiB peak RSS; 34,579 classes visited,
29,628 exact files written, zero output errors or skips. Every file parses.
Full uncapped javac **fails with 64,521 diagnostics** (wave3:62,784). Body coverage
is not global declaration closure, complete class initialization, compiled APK,
or Android runtime proof. No error-reduction claim.

Validation: preceding full workspace497 tests passed/0failed/15ignored; following
focused backend54 tests and checked-exception2 tests pass; allocation7 tests
pass. Strict all-target workspace Clippy, formatting and diff checks pass.
Evidence: -wave4.json, -inventory.json, -validation.json, -run/stderr/time.txt.
All rejected original identities are preserved in -wave4-rejected-methods.csv.

Artifact: /tmp/opencode/decomphify-java-corpus/run-1790931666986198604-3706678/com.samsung.android.samsungpass

Next bounded experiment: disjoint sequential try scopes with original catch
priority/prethrow SSA and single block placement. Nested/cyclic/shared handlers
remain unsupported absent separate proof. The active goal remains incomplete.
