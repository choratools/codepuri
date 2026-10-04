# Git checkpoint and validation bounds

This checkpoint preserves the standalone Rust APK/DEX decompilation core,
Java backend, recovery tests, caches, and current source-publication pipeline.
It does not claim full Samsung Pass recovery or a compilable reconstructed APK.

The last recorded accepted full-corpus baseline is wave50:

- 186,105 accepted executable identities out of 194,286 (95.78919737%).
- This includes 429 separately proven enum equivalents, not just emitted bodies.
- 8,181 executable identities and 28 original bodyless forms remain unresolved.
- 32,555 generated source files passed syntax parsing.
- Whole-source Java compilation failed with 44,884 diagnostics.

The following full run was interrupted during an environment reset. Its
process handle and temporary artifacts disappeared. It is not an accepted
baseline and must not be reported as completed or as proof of no regressions.

Current checkpoint changes additionally include proof-gated return-collision
alias alignment and original-handler metadata in checked-source bridge local
declarations. Their bounded tests do not establish full-app acceptance.

## Local reports

`bench/local-artifacts-manifest.json` lists large reports retained locally at
their original paths, with byte sizes and SHA-256 hashes. A local archive is
stored under ignored `bench/local-artifacts/`; no source report was deleted.
These optional historical reports are not bundled with a fresh Git clone.

`bench/restore_samsung_validation.py` verifies the pinned framework and JDK
inputs and can restore absent files with `--restore`. Device fingerprint and
download SHA-256 must match the recorded inputs. Input restoration is not an
export, compiler, or runtime-equivalence gate.

The current pre-commit quality-gate summary is saved separately in
`bench/git-checkpoint-validation.json`. Full APK recovery, source compilation,
native-library recovery, resources, and Android execution equivalence remain
unproven.

## Rebuild the current full evidence

Run `python3 bench/run_samsung_checkpoint.py bench/local-artifacts/NEW-RUN`.
The directory must not exist. The runner freezes the compiled release binary,
Rust sources and input hashes, then collects all-method coverage, unchanged
source syntax checks and uncapped compiler diagnostics. With no previous ledger,
identity gains/losses remain unknown; this is never a zero-regression claim.
Use `--baseline PREVIOUS-RUN/java25` only when that exact ledger is available.
Collector exit zero is not a successful APK compile or a 100% goal verdict.
