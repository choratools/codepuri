# Samsung Pass wave23 — normal CFG dispatcher and explicit-store initialization

160,254 strict explicit bodies +306 proven enum equivalents =160,560 /205,497 (78.13252748%). 24,388 additional original identities needed for90%. Wave22 exact identity delta3,628gained,13lost. Losses are original static initializers newly coupled to incomplete instance constructor families; independent static proof recovery is in progress, not counted yet.

Normal-flow dispatch preserves original states/edges, pre-transfer simultaneous Phi copies, concrete local types and SSA availability. Entry Phi without an external input edge rejects; cyclic Phi-cluster walks terminate. Monitor/exception flow remains on dedicated paths. Typed explicit-store initialization keeps complete DEX-store/Java-definite-assignment proof separate from native/reflection runtime immutability; old runtime-exclusive APIs remain strict. Structural and symbolic uncertainty still reject. Field catalog and atomic facade now use the same contract. Exact producer-return contracts refine acyclic reference copies only after all consumers agree. Actual PassOperation.process is retained; whole-unit compilation/runtime remains unproved.

All34,579classes visited;30,524sources parse with0syntax errors. Uncapped JDK17/Android35 javac still fails with60,854diagnostics (wave22:60,809). Full30 export272.304seconds,zeroerrors/skips. No complete-app compilation/runtime claim.

Frozen workspace gates passed:636testcases, strict all-target Clippy, fmt, release example/build. Three final dispatcher production tests pass including240runtime traces and both malformed entry-Phi cases. First attempted export aborted on catalog/facade contract mismatch and is explicitly excluded in samsung-pass-90-wave23-aborted.json.

Artifact: /tmp/opencode/decomphify-java-corpus/run-1790955482601570562-2642916/com.samsung.android.samsungpass
Evidence: samsung-pass-90-wave23.json,-inventory.json,-validation.json and retry logs. Goal active/incomplete.
