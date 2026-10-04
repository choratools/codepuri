# Samsung Pass wave41 monitor identity progress

Accepted whole-corpus baseline remains wave40: 184,349 / 194,286 executable definitions (94.885375%). This work does not claim a new coverage percentage.

The original classes3.dex method35087 `DBHandler.getAppFilter(String):String` has acquired Value1, aliases Move7/16, Phi24(7,25), Phi25(24,16), and release25. The previous iterative union cannot seed either mutually recursive Phi despite all external leaves being Value1. New finite reachable Move/Phi traversal proves identity only with one original non-alias leaf. Distinct origins, empty Phis, unanchored cycles and missing definitions remain unproved. No register-number equivalence or exception ownership relaxation.

Exact classes3.dex-only SSA probe confirms the graph, with original DEX SHA256 420e7e603e4c2487daeb2ca73145bf0aaaf256ca02d2c2266f7d9f7674261b60. JADX1.5.1 comparison emits synchronized getAppFilter. It is an input/output comparison, not correctness proof.

Focused validation: 2 identity tests plus 7 existing monitor/cleanup execution and rejection tests pass. Scoped strict Clippy, formatting check and release java_profile build pass. Independent source review found no must-fix. Full backend suite terminal exit0: 471 passed, 0 failed, 13 ignored, 56 suites.

Production class-filter export terminal exit0, 106.966212seconds. The exact original35087 now rejects at `throw checked exception is not declared by caller`, rather than monitor identity. DBHandler retained71 complete original bodies, 0 gained/0 lost. Thus the concrete monitor blocker is fixed, but original method recovery and whole-class compilation are not established. Native/Android runtime semantics and whole-app compilation remain unproved.

Evidence: `bench/samsung-pass-wave41-monitor-checkpoint.json`, `/tmp/opencode/wave41-monitor-exact-35087-ssa.log`, `/tmp/opencode/wave41-monitor-dbhandler`, `/tmp/opencode/wave41-monitor-jadx-DBHandler.java`.
