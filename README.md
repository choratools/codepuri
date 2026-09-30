# decomphify

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
