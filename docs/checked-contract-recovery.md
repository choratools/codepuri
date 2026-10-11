# Original checked-contract recovery

## Implemented boundary

An ordinary, nonsynchronized caller whose original declaration cannot be widened
can now use an unchanged whole-body source boundary for a mixed checked clause
containing `Throwable`. Approval requires a target-specific `BoundTarget` built
from original runtime escaping evidence, not just the presence of a broad clause.

The frontend validates the selected source owner/name/full descriptor, original
defining DEX and method-table index, code item, flags and original annotations.
The source clause must match the original-plus-proven-escaping set. Cross-DEX
targets retain their defining identities while approval remains bound to the
caller's pool reference. Static or final-instance targets are supported; unknown,
nonfinal virtual, native, abstract and synchronized targets remain unapproved.
The original caller's unknown escaping evidence stays explicitly unknown.

Mixed clauses approve only the witnessed `Throwable` entry; all other entries
still require known checked hierarchy. The complete original body stays in its
caller, with one identity-rethrow boundary and a separately attributed synthetic
helper. Original caller ABI, callee clauses, invocation target and operand order
remain unchanged. Synthetic helpers have no original DEX identity and retain the
existing reflective/live-stack and stack-exhaustion observability attribution.

## Inherited final input declarations

References naming an input child can now resolve a public/protected final
instance declaration through a complete, unambiguous input superclass chain.
Return-only ambiguity, private/static/abstract targets, incomplete classes,
cycles, incompatible frozen source aliases and unattested generic source
signatures reject. Requirements and exception dependency discovery use the actual
declaring identity. Protected calls preserve the original child-static receiver
instead of introducing an illegal parent-typed qualifier.

`CODEPURI_TRACE_METHOD=MainActivity.onCreate,LoginActivity.onCreate` enables
source-contract, bridge planning and transactional rendering rejection traces.

## Absher verification

Input: `/home/jtjisgod/absher/base.apk`, SHA-256
`172575d755606326dd5fd1d8c21dff7d3f5390ee782a0584746195952ac936e4`.
App scope `Lcom/elm/android/individual/`, workers 30, Android35 metadata.

| Scope report | Previous readability export | Recovery export |
|---|---:|---:|
| Complete ordinary methods | 10,697 | 11,119 |
| Unsupported ordinary methods | 19,556 | 19,134 |
| Complete constructors | 1,911 | 1,911 |

There are **422 additional complete ordinary methods** in this scope. Both
exports count 30,253 ordinary methods. Emitted classes change from 5,604 to 5,606.
These are source-publication results, not whole-APK execution equivalence or a
paired performance measurement.

Both original lifecycle bodies now emit:

- `classes.dex`, method 12710: `MainActivity.onCreate(Bundle)`.
- `classes2.dex`, method 3090: `LoginActivity.onCreate(Bundle)`.

Each retains `public final void onCreate(Bundle)` with no added throws clause.
Both are marked `semantic_complete=true`, **`dependency_complete=false`**:
their own Java bodies are recovered, but the strict callee closure is incomplete.
The remaining diagnostic is `required complete method body unavailable`.
This is not a claim that all Absher failures or whole-class compilation are solved.

Artifacts: `/tmp/opencode/absher-oncreate-recovery/app-scope/java/`, report
`app-scope/report.log`, and `validation.json` in the same recovery root.

## Validation

- Workspace: **1,303 passed / 175 ignored / 0 failed**.
- Workspace/all-target Clippy `-D warnings`, fmt, diff check and release build.
- Java17 oracle: unchanged returned values, before/invoke/after traces and exact
  exception object identity for checked exceptions, `Error`, bare `Throwable` and
  runtime exceptions; original no-throws ABI still compiles.
- Genuine javac/D8 input: symbolic child-owner reference to a protected-final
  getter through multiple input ancestors; emitted Java compiles and executes
  across packages without an illegal parent-typed receiver.
- Wrong pool reference, contract drift, missing hierarchy and nonfinal virtual
  runtime witnesses reject; existing allocation-order and handler rejection
  tests pass.
