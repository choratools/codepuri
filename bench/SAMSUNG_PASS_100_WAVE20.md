# Samsung Pass wave20: verified partial progress

The target remains 100%. The APK, original method denominator, ordered JDK17/Android35 classpath, source release25 and30 workers are unchanged. Frozen source, binary and input hashes stayed unchanged throughout validation.

| Metric | Final |
|---|---:|
| Original definitions | 205,497 |
| Code-bearing originals | 194,286 |
| Explicit bodies | 180,520 |
| Proven enum equivalents | 394 |
| Accepted executable originals | 180,914 (93.117363%) |
| Retained genuine bodyless declarations | 11,183 |
| Remaining executable originals | 13,372 |
| Missing genuine bodyless declarations | 28 |
| Java source units | 32,388 |
| Syntax errors | 0 |
| Uncapped whole-javac diagnostics | 52,165 |
| Export wall time | 350.57 seconds |
| Peak RSS | 15,613,336 KiB |

Compared with verified wave18: zero gained or lost executable identities. Compared with failed wave19:219 restored and102 removed; restoration of an experiment regression is not new recovery. Since wave16, net acceptance rose by348;24 earlier identities now require correct checked-caller contracts. An original reflection trace establishes genuine unchecked-by-DEX but checked-by-Java reflection exceptions, so blanket removal of checked contracts would be unsound.

The concrete-producer preservation change now applies only to Phi/copy families with an actual Throw consumer. Ordinary reference families retain their established refinement path. Whole-arena consumers, constructor subtype receivers, object identity, null behavior, original predecessor selection and atomic Phi copies remain checked. Actual method42568 advanced from Unknown Phi to a checked-invoke rejection; this is not body recovery. Method310 still fails the checked-throw contract.

Shared immutable original-class metadata uses Arc payloads to avoid repeated full-class cloning. SSA and resolution caches remain enabled. These runs combine correctness and cache changes under host load; cache-only speedup is not established.

Validation: 987 workspace tests passed, zero failed, 134 ignored. The31 contextual tests and strict backend all-target Clippy passed; full workspace Clippy passed in wave19 and formatting passed after the narrow guard. Full javac still fails, without source exclusion or rewriting. No compiled-APK, native-library or Android-runtime completion claim.

Evidence root: `/tmp/opencode/samsung-pass-100-wave20`.

The failed generalization is retained in [wave19](SAMSUNG_PASS_100_WAVE19_REGRESSION.md). Machine-readable coverage, validation, terminal hash receipts and workspace counts are adjacent to this report.
