# Pinned JADX comparison: Samsung cache/AppCompat checked-contract barrier

## Provenance and bounded commands

Installed JADX reports **1.5.1**. Exact upstream release tag `v1.5.1` resolves to `c4c3d42d164a9b9be76662296382d3770b82b91f` via git ls-remote. Installed all.jar SHA256 `dc717e51c08d31a66357933c8d4896755d7c4939745e03aefd02427041f9aa53`.

Original Samsung Pass APK SHA256 `d49a8bbf7aac35ec4d2e842535fd85ac105c49291d9efe23e041ae8afee12c41`.

Six bounded calls used the installed CLI:

```text
jadx --no-res --single-class <original class> --single-class-output <file> tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk
```

Original selections: `androidx.appcompat.app.AppCompatActivity`, `v5.c$c`, `b0.i`, `n5.j$b`, `t5.g`, `t5.e`. Inner-class selections cause JADX to save the containing original outer class; no full export was performed. Logs and unchanged output are under `/tmp/opencode/jadx-wave23-cache/`.

## Source mechanism, confirmed in installed bytecode

Pinned source links:

- [MethodGen.java](https://github.com/skylot/jadx/blob/c4c3d42d164a9b9be76662296382d3770b82b91f/jadx-core/src/main/java/jadx/core/codegen/MethodGen.java#L171): calls `annotationGen.addThrows(mth, code)`.
- [AnnotationGen.java](https://github.com/skylot/jadx/blob/c4c3d42d164a9b9be76662296382d3770b82b91f/jadx-core/src/main/java/jadx/core/codegen/AnnotationGen.java#L121): reads `mth.getThrows()` and prints a throws list only if nonempty.
- [MethodNode.java](https://github.com/skylot/jadx/blob/c4c3d42d164a9b9be76662296382d3770b82b91f/jadx-core/src/main/java/jadx/core/dex/nodes/MethodNode.java#L482): reads `JadxAttrType.EXCEPTIONS`; absent attribute returns the empty list.

Pinned downloaded files are beside the outputs. `javap -c` of the installed jar independently matches the getThrows and addThrows paths (`installed-MethodNode.javap`, `installed-AnnotationGen.javap`). This does not rely only on a similarly named upstream source file.

## Same-original-method behavior

Actual AppCompatActivity original #5339 output at AppCompatActivity.java:65:

```java
public void addContentView(View view, ViewGroup.LayoutParams layoutParams) {
    m();
    getDelegate().e(view, layoutParams);
}
```

Actual original `v5/c$c.close` #60997 becomes C0525c.close in its original outer output, resource-close.java:424:

```java
public void close() {
    this.f36561e.close();
    e(true);
}
```

`f36561e` remains genuinely typed `t5.g`; actual JADX `t5.g extends t5.e`, and `t5.e extends java.io.Closeable`. Neither redeclares close. The same-method output adds **no catch, no throws, no generic exception-effect model, no helper**.

Actual base-cache.java:50/59/96 likewise emits base virtual callback b and public-final cache d/e with no inferred checked throws. Actual resource-cache.java:133 calls `oldValue.close()` and `super.b(...)` without adding throws/catches. JADX inlines/merges its bridge into typed arguments; this is an additional source-representation difference, not evidence the entire erased-family output compiles.

Thus the observed difference is direct: JADX preserves the original absent checked-throws attribute and emits the readable method despite Java's checked-exception compile restriction. Our source planner derives and checks the genuine uncaught Closeable contract, propagates it through the real callback family, then rejects Activity's incompatible override. There is no evidence JADX has solved this by safe exception specialization.

## Minimal genuine-type compiler diagnosis

JDK17 javac was supplied the **unchanged actual JADX t5/e.java and t5/g.java**. `javac-slice/CloseSlice.java` contains the exact original-derived final field declaration, exact #60997 close body, exact original outer e(boolean) body and boolean field. Only a minimal class envelope was constructed; no Samsung helper body, constructor, exception suppression or dependency implementation was fabricated. This is a compiler diagnosis slice, not an unchanged whole-class success receipt.

Javac output (`javac-slice.log`):

```text
CloseSlice.java:3:20: compiler.err.var.not.initialized.in.default.constructor: f36561e
CloseSlice.java:6:31: compiler.err.unreported.exception.need.to.catch.or.throw: java.io.IOException
2 errors
```

The separate blank-final error is expected because the original constructor was deliberately not replaced by a fake constructor. The line6 diagnostic independently confirms the exact unchanged close invocation cannot compile under its genuine static type without IOException handling/declaration. No full JADX subset compilation or Android execution is claimed.

## Safe next route

Keep strict acceptance unchanged. A separate analyst/readability output tier could expose original-throws JADX-style bodies and attach exact uncaught-check/override diagnostics, without counting them as strict complete methods. That produces inspectable code without falsely claiming Java compilability.

Recovering this as strict Java while preserving the shared erased callback family still requires a proved larger family-specialization/effect transformation, or an appropriate source-language backend. Receiver spelling alone, inferred-contract deletion, unchecked suppression and adding Activity throws are not fixes. Original DEX Signature evidence supplies no generic exception-effect parameter. Other32 losses remain unclassified.

Related exact raw trace: `bench/samsung-pass-wave23-cache-checked-contract.md`.
