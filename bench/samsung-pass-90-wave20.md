# Samsung Pass wave20 — SSA correctness and measured partial recovery

156,146 explicit strict bodies +306 proven enum equivalents =156,452 /205,497 original definitions (76.133472%). 90% requires28,496 more accepted identities. Against wave19:880 gained, three withdrawn identities.

Physical-register single explicit definitions now enter pruned dominance-frontier/liveness Phi placement. Original constructor43967 now merges genuine incomingregister1 with its actual defaultallocation; SSA verifier passes and inference haszero diagnostics. Missingincoming evidence remains unresolved, ephemeral values remain excluded, and widepair atomicity remains checked. Exact referenceCast allocation arguments preserve NEW-before-CHECKCAST order. Detailed constructor chain/local evidence is retained without changing accepted bodies or granting initialization proof.

Two withdrawn constructors g9/g41301 and g9/j41348 expose formerly incorrect output: it assigned the incoming parameter in both arms, whereas original SSA must select incoming parameter or null. Their new Phis are correct; suffix lowering currently rejects them. g9/g.m41306 is dependent closure. The old wrong bodies are not restored. Read-only evidence /tmp/decomphify-wave20-ctor-g.log and -j.log.

599 workspace tests passed; strict all-target Clippy and fmt passed. Initial Clippy caught a redundant optional-borrow expression in parentdiagnostics; it was corrected, finalstrictcheck passed. This borrow-only correction does not alter releasebehavior. All34,579classes visited,30,500files,exporterrors/skipszero. All30,500exactfiles parse; uncapped JDK17/Android35 javac still fails with60719diagnostics. Whole-app compilation/runtime unestablished.

Full30program243.315seconds, peakRSS7,844,716KiB; sharedhost/concurrenttests, no controlled speedup claim. Constructor diagnosticinventory7794records nowdistinguishes7317chain-level failures and215local-evidence failures; the5664preinitcategory was sampled only20times, not a fullopcodepopulationhistogram.

Artifact: /tmp/opencode/decomphify-java-corpus/run-1790949779115690949-1838417/com.samsung.android.samsungpass
Evidence: samsung-pass-90-wave20.json, -inventory.json, -delta.json, -validation.json, -constructor-diagnostics.json, -tests.txt, -clippy-final.txt, -fmt.txt, -time.txt.
Goal active/incomplete.
