# Samsung Pass wave31 — rejected candidate

Frozen full export completed with unchanged inputs/source receipts. Candidate remains unaccepted: **780 gained / 23 lost** against accepted wave29. No-loss gates against wave29 and wave26 both failed.

- Executable bodies/equivalents: 182606/194286 (93.988244%).
- Output units 32416; syntax exit0; uncapped full javac exit1, 50900 diagnostics.
- Workspace1070passed/0failed/148ignored,180suites; release, full fmt and strict workspace Clippy passed. This gate belongs to the frozen wave31 source snapshot, not later in-flight repairs.

Remaining23lost:16callee checked-contract mismatches,5dependency consequences,1shared-producer sibling-Phi proof failure,1nested cleanup representation failure.

Later bounded repair: original classes2#31220 now proves both logger Phi families as Object while retaining Long producer;31contextualtests and unchanged generated JVM runtime fixtures passed. Its full production recovery and two caller closures remain pending.

New sourcebridge and shared-primary nested cleanup capabilities are still being implemented/validated; no recovery claims for them yet. Accepted baseline remains wave29. Full goal active/incomplete.

Authoritative evidence: `/tmp/opencode/samsung-pass-100-wave31`; tests receipt `bench/samsung-pass-wave31-workspace-tests.json`.
