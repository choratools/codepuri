# Next original allocation-argument capability

Read-only actual Samsung classes.dex method #5015, ComponentActivity.getDefaultViewModelProviderFactory: allocate androidx.lifecycle.k0 at offset4, getApplication6, first getIntent10, branch14, nonnull getIntent16/getExtras20 versus null25, joined Bundle consumed by constructor26, store29.

The next generic capability is a single-entry acyclic argument diamond, one join and one original initializer. Preserve allocation before argument evaluation, exact call order, and the repeated getIntent call. Java source25 switch-expression blocks can express the argument computation without a synthetic helper object allocation. No handler, escape, shared-effect consumer or unproved call may be accepted.

Required original-JVM/emitted-Java differential cases: alternating getIntent results, exceptions at every call, class-initialization failure, true heap OOM before any argument invocation, and handler/escape negatives.

Status: mapped, not implemented or corpus-recovered. Evidence: /tmp/opencode/wave23-readonly-allocation-5015.log.

Private enum followup: OSmosisStatus constructor20840 and clinit20839 keep the same $VALUES array through mq.b.a into $ENTRIES; values() clone replacement is not equivalent. Thirty private owners/199 rejected records share this pattern. JavaType constructor11292/clinit11291 has actual payload effect-order mismatch (valueOf17 before Integer.TYPE24/class26), not a flag-check defect. PUBLIC constructor enums remain a separate faithful Java-source boundary.
