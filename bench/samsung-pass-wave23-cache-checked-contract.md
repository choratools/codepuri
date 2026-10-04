# Samsung Pass wave23 AppCompatActivity checked-contract loss

Read-only repository diagnosis; isolated scratch only. Original root classes.dex #5339 AppCompatActivity.addContentView(View,ViewGroup.LayoutParams)V.

SourceThrowsPlan rejects with OverrideConflict(java.io.IOException). Original root has declared [], and original ComponentActivity.addContentView and selected genuine runtime Activity.addContentView both bound checked throws to []. Caller has no catch around h.e call at code-unit offset 7.

Verified source chain (all classes.dex unless specified):
5339 AppCompatActivity.addContentView -> 5706 h.e (abstract) -> 5549 AppCompatDelegateImpl.e -> 5572 o0 -> 5609 a.F (abstract) -> 5884 y.F -> 7891 widget.d0.setWindowTitle (abstract) -> 8407 widget.n1.setWindowTitle -> 8376 G -> 7674 Toolbar.setTitle -> 9001/9002/9003 widget.z constructors -> 8945 widget.y.c -> 8944 b -> 8177 widget.i.h -> 8360 widget.n0.v -> 8344 f -> 8349 k -> 8330 widget.n0$a.l -> 21903 b0/i.e -> 21907 i -> 21900 b0/i.b (concrete virtual callback) -> 50746 n5/j$b.b (genuine bridge override) -> 50748 k -> 57406 s5/e.close (abstract) -> 60997 v5/c$c.close -> original java.io.Closeable.close at code-unit offset 2, genuine external declared IOException.

The callback domain is genuine inheritance, not unrelated name matching. n0$a is the color-filter cache; n5/j$b is another real b0/i subclass. The whole-source b0/i callback contract must cover its genuine subclass calling s5/e.close. Concrete/abstract source widening therefore reaches otherwise non-I/O UI calls.

Original v5/c$c.close output retains no catch and directly calls ((java.io.Closeable) this.e).close(), then super.e(true). Its field type t5/g extends t5/e extends Closeable; neither t5/g nor t5/e redeclares close. Thus retaining narrower receiver spelling alone does not eliminate checked IOException. No unchecked cast, widened Activity override throws, forged catch, or disabled override propagation is justified.

A potential future recovery requires proven receiver-specialized source generation (including base final methods and their virtual callback family) or another source language that permits undeclared checked exceptions. That is beyond a bounded declaration-proof adjustment and must preserve real method/dispatch identities. Current safe behavior is rejection with this exact provenance.

Evidence:
- /tmp/opencode/throws-wave23-appcompat-loss-trace.log (single root, 132.93s, passed)
- /tmp/opencode/throws-wave23-appcompat-checked-origin.log (same root, follow nonempty source contracts, max80 visited, 136.03s, passed)
- /tmp/opencode/samsung-pass-100-wave23-runtime/java25/sources/v5/c$c.java:51
- /tmp/opencode/samsung-pass-100-wave23-runtime/java25/sources/b0/i$type.java:69
- /tmp/opencode/samsung-pass-100-wave23-runtime/java25/sources/t5/e$type.java:10
- /tmp/opencode/samsung-pass-100-wave23-runtime/java25/sources/t5/g$type.java:10

Production mechanisms: java_source_throws_integration.rs:963-1001 (attested implementation domain), :1029-1041 (concrete override widening), :742-839 (exact override bounds). Metadata experiment uses JDK17 boot/JDK unsupported/naming plus runtime Android16 candidate before Android35 SDK. Compiler availability and original runtime compatibility remain unattested.

This investigation verifies IOException origin for one actual loss, not all 33 checked losses or the secondary java.lang.Exception origin. No repository Rust edits or full export.

## Receiver-specific recovery assessment

Observed producer: original widget.n0.<clinit> #8337 constructs exactly n0$a(6) and assigns its original static-final field j. n0.k #8349 loads this field, calls n0$a.k/l. Actual n0$a declares only constructor #8327, key hash #8328 and typed wrappers #8329/#8330; it supplies no a/b callback overrides. n5/j$b is a separate final subclass declaring the real a/b bridges.

Producer provenance can narrow runtime callback dispatch on this cache instance, but cannot independently change javac's declaration contract: original base b0/i.d #21902 and e #21903 are public final; their source declarations must invoke virtual a/b and support the genuine resource-cache overrides. Calling this.e instead of casting to b0/i still binds the same final declaration with checked IOException. Adding a narrower override in n0$a is illegal because these methods are final. Catching the exception and returning/wrapping changes the original behavior; new delegating/inlined helper bodies violate the requested no-fabricated-helper condition.

Therefore precise source-only recovery under unchanged original declarations, throws permission, finalness and runtime virtual dispatch has no demonstrated solution. It is not enough to weaken a call-site context based on the static-final producer.

A future explicitly distinct transformation could split/monomorphize the cache declaration family or introduce an exception-effect type parameter into the Java API while preserving bytecode erasure. Both require global dispatch/alias/type/subtyping proof and changes to source declarations; neither is original Signature recovery, a bounded safe hook, or currently authorized strict recovery. A Kotlin backend can instead express original unchecked-by-language bytecode calls without fabricating Java exception suppression, but is also outside the current Java route.

Only #5339's IOException branch is classified here; other32 checked-contract losses are not declared correct from this sample.

## Original DEX declaration/catch evidence

Fresh bounded original-metadata probe passed (7.19s): `/tmp/opencode/throws-wave23-color-cache-original-metadata.log`.

- Original `b0/i`, `n0$a`, `n5/j$b`, `t5/e`, `t5/g` all have **no retained DEX Signature annotation**. No original generic exception-effect parameter can be recovered. Any such effect parameter would be a new source transformation.
- `n0$a` class flags `0x1` (not final), `n5/j$b` flags `0x11` (final). Producer allocation is exact n0$a but wider receiver escape still requires independent evidence.
- #5339 code offset1658876: tries=[], handlers=[].
- #8330 code offset1804668: tries=[], handlers=[].
- #21900 callback code offset2885020: tries=[], handlers=[].
- #50746 real bridge flags0x1041, code offset4740304: tries=[], handlers=[].
- #50748 code offset4740340: tries=[], handlers=[].
- #60997 code offset5380276: tries=[], handlers=[]. This is the uncaught interface Closeable.close invocation at code-unit2.
- Base #21902 flags0x11 and #21903 flags0x11 establish final methods. Their original catch-all tables cover monitor cleanup, not a typed IOException suppression proof: #21903 range[13,50), catch-all46; #21902 ranges[8,22),[26,32),[44,77), catch-all24/64. Current source retains virtual callback invocation and does not swallow IOException.

No source contract/body change follows from this evidence.
