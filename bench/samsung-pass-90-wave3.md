# Samsung Pass 90 percent goal: third measured wave

Unchanged denominator: 205,497 original definitions. Strict complete bodies:
126,092; proven enum equivalents: 288; accepted: 126,380 (61.4997 percent).
Target gap: 58,568 identities. Relative to source-closure baseline, 38,656 gained
but 1,570 lost. This wave is a regression checkpoint, not a zero-loss milestone.

New paths: explicit direct-parent invoke-super, allocation argument expressions
preserving allocation-before-argument effects, and narrow integer promotion.
Full30 export visited 34,579 classes; zero export errors or skips; 83.326 seconds.
All 29,612 exact files parse. Full uncapped javac fails with 62,784 diagnostics.
497 workspace tests passed, zero failed, 15 ignored before the next repairs.

The lost identities mainly report undefined values after argument-gap skipping,
plus dependent callers. The skip hook selected the argument plan even when the
initializer selected the ordinary pure plan. A matching-plan regression fix and
fixture are now implemented; fresh full measurement remains pending. No loss is
hidden or reclassified as success. Checked-exception subclass coverage is also
under focused validation for the next wave.

Evidence: samsung-pass-90-wave3.json, -inventory.json, -validation.json,
-run.txt, -stderr.txt, -time.txt, -tests.txt.
Artifact: /tmp/opencode/decomphify-java-corpus/run-1790930928567658939-3620458/com.samsung.android.samsungpass

The active 90 percent goal remains incomplete; body coverage does not establish
whole-class initialization, whole-app compilation or runtime equivalence.
