# Test corpus

Real-world regression fixtures for parser and decompiler tests.

## `apks/`

| file | source | license |
|---|---|---|
| `termux-app_v0.118.0_arm64.apk` | github.com/termux/termux-app releases v0.118.0 | GPLv3 (test fixture only, not distributed) |

## `dex/clean/`

Well-formed DEX files (BSD-3-Clause; see `dex/LICENSE`, from github.com/droidsaw/droidsaw-dex
`tests/fixtures/`):

- `classes.dex`, `classes_named.dex` — javac-compiled samples
- `r8_block_outlining.dex` — R9-processed (optimized) DEX

## `dex/adversarial/`

Malformed / hostile inputs (same source, BSD-3-Clause) used by robustness
tests: the parser must return typed errors, never panic. Includes fuzz seeds
(`fuzz_*`), OOM-bait inputs (`oom/`), and semantic-invariant violators
(`opcode_invariant/`, `try_item_range/`, ...).
