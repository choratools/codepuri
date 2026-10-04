# Samsung Pass wave36 verified result

Original executable definitions: 194,286. Accepted explicit bodies plus proven enum equivalents: 183,732 (94.567802%). Explicit bodies: 183,303; equivalents: 429. Compared with wave35: 86 identities gained (46 constructors, 40 ordinary), zero lost. Remaining: 10,554 executable definitions and 28 genuine bodyless forms.

The constructor fix applies loop/receiver final-assignment restrictions only to genuinely final fields; missing field metadata still rejects. CFG rebuilding preserves original edge insertion order. Two repeated original class exports were byte-identical; whole-APK determinism is not established.

All 32,420 emitted files parse. Full uncapped javac fails with 50,658 diagnostics. Export: 305.12 seconds, peak RSS 16,493,356 KiB under concurrent tests/probes. This is not an isolated performance comparison. Whole Android execution, resources and native decompilation remain unverified.

Workspace: 1,094 tests passed, 0 failed, 155 ignored across 184 suites. Format, strict Clippy, release build and identity no-loss gates passed.

Frozen receipts: `/tmp/opencode/samsung-pass-100-wave36`. Detailed constructor evidence: `bench/samsung-pass-wave36-mutable-constructors.json`. The 100% goal remains active and incomplete.
