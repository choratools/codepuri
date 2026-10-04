# decomphify

Latest production recovery status, validation and final APK measurement:
[`bench/JAVA_PRODUCTION_INTEGRATION.md`](bench/JAVA_PRODUCTION_INTEGRATION.md).

Export-session cache options and measured memory/time results:
[`bench/SAMSUNG_PASS_MEMORY_CACHES.md`](bench/SAMSUNG_PASS_MEMORY_CACHES.md).

Repeated exports can retain the exact Java source snapshot in
`decomp::java_session::JavaSession`. Construct it with owned DEX inputs and fixed
`JavaOptions`, then call `emit` with a fresh sink for each export. The first call
decompiles; later calls reuse the sources and report. Classpath files are loaded
once at construction. `clear_cache()` releases generated sources; changing inputs
or classpath requires a new session. This speeds repeated identical work, not the
initial decompilation.
Measured cold/warm source replay:
[`bench/SAMSUNG_PASS_SOURCE_SESSION.md`](bench/SAMSUNG_PASS_SOURCE_SESSION.md).
Earlier full Samsung Pass replay validation: **350.65 s first render → 1.51 s
cached export**, all 31,188 files and reports identical; retained source text
294.37 MiB and whole-process peak12.26 GiB. This requires keeping the same library
session alive. The new original-input inheritance index also preserves all
method statuses. See
[`bench/SAMSUNG_PASS_CACHE_WAVE6.md`](bench/SAMSUNG_PASS_CACHE_WAVE6.md).

Resolution caching also retains completed contextual field proofs for the whole
export: original symbolic field, caller, receiver and staticness form the key.
Frozen metadata and aliases belong to the cache; declaration/initialization
publication remains outside it. `JavaProfile::field_use_cache_*` reports stage
hits, computations, typed failures and distinct keys. Disabling
`cache_resolutions` uses the original uncached path. Memory grows with actual
results; no fixed share of the host's RAM is reserved.
Latest frozen Samsung Pass check: **317.55 s first export → 1.58 s cached
replay**, with all **31,196 files and reports identical**. Retained text is
301.24 MiB; whole-process peak RSS was 14.79 GiB. This is same-session replay,
not a measured cold-render speedup. See
[`bench/RAM_CACHE_CURRENT.md`](bench/RAM_CACHE_CURRENT.md).

Java source output defaults to release 17. Explicit `--source-release 25` retains
proven constructor prologues in original execution order; those sources require
a Java 25 compiler. See [source release selection](docs/java-source-release.md).

Rust 기반 멀티 타겟 디컴파일 코어 (UI 없음, 라이브러리 우선). 1단계 목표: APK/DEX → Java.

## Crates

| crate | 역할 | 의존 |
|---|---|---|
| `decomp-core` | 타입 arena CFG, 그래프 분석(dominator/frontier/loop), 패스 레지스트리, 진단 — IR 패밀리에 무지(non-family) | - |
| `decomp-emit` | 언어 불문 코드 출력 인프라(토큰, 우선순위, 네이밍, source map) | - |
| `dex-format` | lossless DEX 파서 + APK(ZIP) 컨테이너 추출 | - |
| `backend-java` | structured IR → Java 출력 (phase 2) | core |
| `decomp` | 퍼사드 라이브러리 + 개발용 CLI (phase 2) | all |

## 아키텍처 불변식

- `decomp-core`는 어떤 IR 패밀리도 몰라야 한다 (헥사고날 경계).
- CFG 변경은 `Cfg` mutation API 경유만 허용, 모든 mutation은 revision 증가로 분석 캐시 무효화.
- 엣지는 일급 객체 (switch 라벨 / exceptional flow / fallthrough 구분).
- 패스는 선언적 의존성(requires/provides/invalidates)을 갖는 플러그인.

## 품질 게이트

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

- `unsafe` 전면 금지 (workspace lint).
- 라이브러리 경로에서 `unwrap`/`expect` 금지 — `thiserror` 타입 에러 사용.
- 테스트 픽스처: `tests/corpus/apks/` (실제 APK 회귀 코퍼스).
