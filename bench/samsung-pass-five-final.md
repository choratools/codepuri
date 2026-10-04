# Samsung Pass five — fresh final evidence

Supersedes `samsung-pass-five-blockers.md` and its intermediate/final metrics. **Partial integration; cross-DEX source-field publication is still deferred.**

| Metric | Original | Fresh final |
|---|---:|---:|
| Strict explicit bodies | 79,340 | 81,828 |
| Proven enum equivalents | 257 | 263 |
| Bodyless declarations | 11,140 | 11,140 |
| Strict rejects | 114,760 | 112,266 |
| Java files | 24,749 | 26,270 |
| Uncapped javac errors | 38,158 | 51,302 |

Immutable denominator: **205,497** original definitions. Strict body/equivalence identities: **2,494 gained, 0 lost**. `Ld1/g;->a(I)J` is restored and asserted complete in the final ledger.

Semantic explicit bodies: **97,199**; outside strict closure: **15,371**; directly unresolved same-class dependencies: **13,459**. Semantic gained/lost against original explicit set: **17,859/0**. These metrics remain separate.

## Production bounds

- **ctor_writeinv_initializer:** Production initializer inventory remains atomic; direct-parent external constructor-body invoke-super hook integrated.
- **contextual_locals:** Atomic checked_phi_plan family declarations active in ordinary structured/switch/shared-forward paths; whole-arena consumers and original reaching inputs checked. Edge sources snapshot before assignments, zero coercions preserve availability. Loop/exception family adoption remains bounded.
- **dependency_publication:** Ordinary semantic callers retained separately from legacy strict dependency-closure metric; exact method_closure diagnostics active. Constructor/clinit closure stays atomic.
- **externalfields:** Public inherited classpath fields use descriptor-exact actual declaring owner and genuine metadata, original caller/receiver access checks and actual-owner aliases. Input-owned/cross-DEX field publication remains deferred; protected inherited production fields remain fail-closed.
- **structure_flow:** Canonical shared acyclic regions active in production with path-availability intersection, edge copies and typed terminal rendering.

Numeric Cast consumers resolve the actual instruction target through `JavaResolver::instruction_type`; numeric source categories remain distinct from conversion targets. Synthetic targets are int, long, float, double, byte, char and short. Boolean/void conversion targets reject; unavailable metadata stays indeterminate. Genuine category-one zero materializes as int when hoisted; proven edge coercions select 0/false/null without replacing selected phi identities. Every selected phi and downstream copy receives one atomic concrete declaration. Original SSA and predecessor copy snapshots are retained.

Inherited fields use actual owner qualification and aliases. Instance receiver casts retain hidden-field selection; protected inherited production fields remain bounded by the existing attestation path. Cross-DEX publication and globally attested explicit initialization are **not implemented**. Existing static ConstantValue source reads retain the prior Java constant-inlining limitation; exact class-initialization preservation is not claimed for that subset.

## Validation

Workspace tests: **442 passed, 0 failed, 12 ignored**. Full workspace/all-target strict clippy, fmt and release build recorded in final checks. JDK17 runtime phi fixtures cover integer/boolean/reference zero joins, selected branch effects, copies and incompatible arena escapes; original Samsung SSA regression and genuine Android35 inherited-owner source compile pass. Existing protected receiver, hidden-field, staticness and cross-DEX resolver tests remain in the full workspace gate.

All **26,270** exact published sources parsed; parser exit **0**. Whole-source javac exit **1**; cap **2,147,483,647**, reached: **False**. No published bodies excluded.

| Attribution category | Errors |
|---|---:|
| missing_output_member_or_unresolved_symbol | 25,399 |
| missing_external_or_unmapped_class | 7,895 |
| missing_output_override_or_signature_alias | 5,662 |
| missing_input_class_or_alias | 4,376 |
| constructor_signature_or_implicit_parent | 4,264 |
| other_source_attribution | 2,813 |
| missing_external_package | 627 |
| missing_input_package | 143 |
| access | 86 |
| module_conflict | 37 |

Common files: **24,749**; old common errors **38,158**, final common errors **48,091**. Old-only errors **0**; new-only errors **3,211**. Per-file old/final counts and messages are saved in `samsung-pass-five-final-per-file-errors.json`. This measures emission attribution on overlapping files, not isolated matched-method correctness; expanded bodies and dependencies affect common units too. No aggregate error-reduction claim is implied.

Final source root: `/tmp/opencode/decomphify-java-corpus/run-1790897304245626033-3778956/com.samsung.android.samsungpass`. Exact original three-class JDK17 and Guideline/Android35 subset outcomes and hashes are retained in `samsung-pass-five-final-validation.json`. All five category/reason counts are in `samsung-pass-five-final-inventory.json`; strict and semantic exact identities in `samsung-pass-five-final-summary.json`. Export time/RSS: **72.096 s / 1,477,776 KiB**. Final logs: `/tmp/opencode/samsung-pass-five-final/`.
