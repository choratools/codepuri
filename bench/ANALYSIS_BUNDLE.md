# Codepuri original-code analysis bundle

The analysis bundle links existing Codepuri Java output to exact original DEX
definitions, decoded instructions, payloads, try/catch ranges and symbolic
invocation references. It permits inspection of a method even when Java export
is unavailable or has dependency diagnostics. It does not repair those Java
bodies or claim whole-app compiler/runtime equivalence.

## Generate and inspect

```bash
cargo build --release -p decomp --example method_inventory
python3 bench/analysis_bundle.py build \
  --apk /home/jtjisgod/absher/base.apk \
  --java /tmp/opencode/absher-apkm-codepuri/codepuri \
  --java-log /tmp/opencode/absher-apkm-codepuri/codepuri.log \
  --manifest /tmp/opencode/absher-apkm-codepuri/apk-manifest.txt \
  --out /tmp/opencode/absher-apkm-codepuri-analysis

python3 bench/analysis_bundle.py show \
  /tmp/opencode/absher-apkm-codepuri-analysis/analysis.sqlite \
  'Lcom/elm/android/individual/MainActivity;->onCreate(Landroid/os/Bundle;)V'
```

Output roots must be fresh. APK/native code is never executed. If the Java
directory's parent has an export `manifest.json`, its APK SHA-256 must match.
Without that receipt the Java/APK binding is explicitly unverified. Java
source locators and original pool method indexes are retained; duplicated
original identity/index combinations receive ambiguous attribution status.

SQLite stores compressed original instruction bytes and readable disassembly,
method metadata, Java evidence, symbolic calls and manifest components. The
manifest input is the `aapt dump xmltree` text form. Explicit component
attributes and separate intent-filter/action/category/data groups are retained;
resource references are not guessed, and absent exported flags are not resolved
to Android defaults.

## Important interpretation

- `body_with_diagnostic` is still an emitted body, not an absent method.
- `not_observed` is not automatically Java failure; some enum emission has no
  ordinary identity comment.
- Bytecode offsets and branch/payload addresses are **16-bit code units**.
- Invocation targets are original symbolic references. Virtual dispatch,
  reflection, native transitions and input taint are not resolved by this table.
- Native/abstract definitions have no original DEX body. Native implementation
  needs a separate analysis path.

For caller lookup:

```sql
SELECT m.identity, m.java_status, c.offset, c.opcode, c.target
FROM calls c JOIN methods m ON m.id=c.caller_id
WHERE c.target LIKE '%->addJavascriptInterface(%';

SELECT name, explicit_exported, permission, filters
FROM components WHERE kind IN ('activity','provider','receiver','service');
```

These are review entry points and references, not vulnerability findings.

## Absher observations

| Input | Original definitions | DEX bodies | Decoded bodies | Symbolic invokes |
|---|---:|---:|---:|---:|
| AppGallery standalone 4.0.50/2679 | 428,906 | 415,864 | 415,864 | 3,145,853 |
| `/home/jtjisgod/absher/base.apk`, same version | 475,340 | 458,065 | 458,065 | 3,288,013 |

Both bundles had zero instruction decode errors and passed SQLite quick checks.
Their APK hashes and DEX contents differ despite the identical version label.
Configuration splits in the second input contain native/resources but no DEX;
all Java-analysis code is in its base APK.

In both variants, `RoutingActivity.onCreate` and the inspected `onNewIntent`
bodies are available with diagnostics. `MainActivity.onCreate`,
`LoginActivity.onCreate` and `DeepLinksRoutingActivity.onCreate` lack Java bodies
but now have complete original instruction evidence and call references.
The AppGallery routing wrapper directly invokes its `CommonLogger(Object[],
int...)` dispatcher, illustrating why wrapper recovery alone is insufficient.

Priorities for Java reconstruction remain caller checked-exception handling,
large exception-dispatch scope availability, primitive assignment evidence,
string-decoder helpers and dependency/initialization planning. No boundary was
relaxed to manufacture a supported Java body for the analysis bundle.

Local evidence roots:

- `/tmp/opencode/sa-gov-moi-codepuri-analysis/`
- `/tmp/opencode/absher-apkm-codepuri-analysis/`

Tests: disassembly unit cases preserve exact symbolic invoke and branch offsets;
Python cases preserve emitted-body diagnostics, unknown statuses, component
permission scope and distinct intent-filter URI groups.
