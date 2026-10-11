# Focused export and bounded LLM context

## System changes

- `--source-prefix` and `--library-prefix` are repeatable original DEX owner
  scopes. They affect Java rendering, not input declaration inventory.
- Existing `--class` also seeds source-contract analysis from selected original
  method definitions. Required invocation/implementation/override dependencies
  are followed; unrelated source bodies are not eagerly inspected.
- Discovered dependencies are processed in ordered FIFO waves on bounded
  workers, preserving breadth-first requirement/error order. Default unfiltered
  export still seeds every original definition. Field-write inventory and other
  whole-input boundary checks remain intact.
- Profiler scheduler statistics include dependency waves.
- `llm_pack.py` removes comments and blank lines while preserving literal/text
  block contents, including URLs and Unicode line separators. Eligible Java
  Unicode preprocessing inputs are retained unchanged rather than rewritten.
- Original identity comments are verified before member extraction. Fresh scope
  files are located by exact identity/index, not stale line offsets. If evidence
  no longer matches, extraction uses original DEX rather than another member.

## Source scope

```bash
mkdir /tmp/my-app-export
./target/release/decomp app.apk /tmp/my-app-export/java --workers 30 \
  --source-prefix 'Lcom/example/app/' \
  --source-prefix 'Lcom/example/appsupport/' \
  --library-prefix 'Lcom/example/appsupport/vendor/' \
  --classpath /path/to/android.jar
```

Prefixes describe original owners, not sanitized output paths. Known exact
library artifacts may be supplied as JVM classpaths. A misleading/relocated
prefix alone is not sufficient to classify library code: Absher's Application
and substantial application references are under `com.google.internal`.
Its omitted code remains available for targeted original-code retrieval.

The scope is a supported subset, not a promise that app Java compiles against
every skipped library. It retains the existing dependency/initialization
diagnostics and source publication rules.

## Absher scope observation

Same `/home/jtjisgod/absher/base.apk`, 30 workers and Android35 metadata:

| Export mode | Wall observation | Java files |
|---|---:|---:|
| Full Java export, prior run | 158.22 s | 86,869 |
| App filter, previous whole-program contract scan | 89.19 s | 5,604 |
| App filter, focused dependency contract scan | 52.23 s | 5,604 |
| New source-prefix CLI, same scope | 50.83 s | 5,604 |

The latter two used `Lcom/elm/android/individual/`. Before/after app-filter output
paths and method counters matched, with three diagnostic-comment-only file
differences and no non-comment Java changes. Single runs and different source
output scopes are not a universal 3x algorithm speedup claim.

## Build the LLM pack

```bash
python3 bench/llm_pack.py build \
  /tmp/opencode/absher-apkm-codepuri-analysis/analysis.sqlite \
  --java /tmp/opencode/absher-app-scope-final/codepuri \
  --out /tmp/opencode/absher-llm-final --max-chars 80000 --retain-runtime
```

Generated:

- `CONTEXT.md`: primary components, seven critical method records, deduplicated
  symbolic call summaries and bounded code pages. Feed this file first.
- `methods.jsonl`: retrieval index (39,322 scoped original definitions) with
  statuses, bytecode features and structured rejection diagnostics; not intended
  as one prompt.
- `components.json`: explicit component attributes and separate intent filters.
- `compact-src/`: 5,604 app files, 151,900,839 to 102,677,763 UTF-8 bytes after
  comment/blank-line reduction. Three conservative files remain unchanged.
- `runtime-dex.jar`: 63,172,685-byte byte-exact original root-DEX archive,
  including app and dependency code. It is **not JVM `.class` bytecode or a
  library-only DEX rewrite**, and must not be passed to javac as a class JAR.
  It is compatible with Codepuri's `--metadata-dex` archive format.
- `summary.json`: source reduction, source extraction/paging, archive format and
  context budget receipts.

This case's initial context is **50,435 characters/UTF-8 bytes** within an
80,000-character limit. No model-specific tokenizer was used; no exact token
count is claimed. Some pages are explicitly incomplete and provide `next_line`.

## Retrieve the next bounded context

```bash
python3 bench/llm_pack.py lookup \
  /tmp/opencode/absher-apkm-codepuri-analysis/analysis.sqlite \
  'MainActivity;->onNewIntent' --limit 10

python3 bench/llm_pack.py excerpt \
  /tmp/opencode/absher-apkm-codepuri-analysis/analysis.sqlite 5466 \
  --java-root /tmp/opencode/absher-app-scope-final/codepuri \
  --start-line 172 --max-chars 16000
```

Line offsets here are zero-based within the extracted/compacted member, not the
class's original source lines. Lookup and excerpt output are bounded. Oversize
individual lines are identified rather than silently split as complete code.
Symbolic calls, partial Java and absent records must not be treated as taint,
resolved runtime dispatch or vulnerability proof.

## Meaning-preserving source reconstruction

The source emitter now additionally performs bounded SSA expression recovery,
typed AST condition/guard/loop cleanup, single-copy Phi lowering, and conservative
dispatch-region recovery. See [implementation and evidence](../docs/decompiled-readability.md).

The final verified app Java is at `/tmp/opencode/absher-readable-verified/codepuri/`.
The rebuilt pack is `/tmp/opencode/absher-llm-readable-final/`:

- Compact Java: **92,388,317 bytes**, down **10.02%** from the prior
  comment/blank-line-only 102,677,763-byte pack.
- Initial context: **50,297 characters** within the same 80,000-character budget.
- Original retrieval identities: 39,322; runtime archive: 63,172,685 bytes.
- Source paths, method identities, publication flags, and coverage report scalar
  counts matched the previous scope. Same three lexer-conservative files retained.

Use this fresh Java root for subsequent excerpts; previous source line offsets
are not valid locators for the reconstructed output.

## Configurable evidence-qualified library separation

The CLI now accepts `--library-evidence`, `--library-min-score` (default 0.95),
and `--library-out`. Exact original class implementations and globally distinctive
body anchors are required independently of the chosen score. See
[library separation and evidence](LIBRARY_SEPARATION.md).

The latest separated full source is `/tmp/opencode/absher-library-split-095/`:
86,865 app-or-unknown Java files and 4 published library Java files, from 8
verified original library classes. The original runtime DEX remains complete.
The app-focused LLM bundle `/tmp/opencode/absher-llm-library-split-final/` retains
`library-selection.json` with the threshold and original owner decisions. Its
initial context is 50,297 characters; no token count is asserted.
