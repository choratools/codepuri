# codepuri 마스터 플랜

decomphify 포크(업스트림 `/storage_0/decomphify`, HEAD `61f517b`) 기반.
목표: **jadx보다 빠른 DEX→Java 디컴파일러 코어**를 만들고, 공유 IR 코어 위에
멀티 타겟(JVM/.NET/native)을 확장한다. UI 없음, 라이브러리 우선.

## 기준선 (2026-10-05 측정, Samsung Pass APK, 동일 머신 30스레드)

| 항목 | jadx 1.5.1 | decomphify HEAD |
|---|---|---|
| wall clock | **26.26s** (user 457s) | 53.3s |
| peak RSS | 16.1 GiB | 12~14.8 GiB |
| syntax error | 2,210 + 미디컴파일 1,041 | **0** |
| strict body 복원률 | 미측정 | 96.4% (187,288/194,286) |
| 출력 크기 | 85.9 MB | 345.5 MB (주석 88.7 MB 포함) |
| 가독성 witness | 8줄 | 91줄 |

jadx 측정 원본: `bench/local-artifacts/jadx-samsung-pass-speed-20261005-1/`.
비교 제약: `bench/jadx-samsung-pass-output-comparison.json` 참고.

스테이지 프로파일(태스크 경과 합, `bench/samsung-pass-stage-profile-evidence.json`):
`structure_backend` 113.2s ≫ `initialization_family` 58.2s ≫ SSA 54.2s ≫
`structure_precheck` 15.1s ≫ `source_serialization` 8.6s ≫ `source_throws` ~11s(wall).
프론트엔드(decode/lift)는 1초 미만 — 파서 손댈 필요 없음.

## 현재 상태

- 워크스페이스 체크: 통과 (클론 직후 13.5s).
- 테스트: `backend-java --lib` 367개 중 **49 실패 / 304 통과 / 14 무시**.
  업스트림 작업토리에서도 동일 결과 — 포크 문제가 아니라 HEAD가 레드.
- 코퍼스: `tests/corpus/{apks,apks_bulk,apks_device}`는 업스트림을 향한
  심볼릭 링크(6.2GB 중복 없음). 단일 출처 유지.

## 크레이트별 방침

| 크레이트 | 줄수 | 방침 |
|---|---|---|
| `dex-format` | 2.3K | **동결·재사용** — 프론트엔드 1초 미만, 손댈 곳 없음 |
| `decomp-core` | 1.4K | **동결·재사용** — CFG/dominator/loop, 무결점 |
| `decomp-emit` | 0.9K | 재사용 |
| `managed-ir` | 1.9K | 재사용 |
| `decomp-structure` | 3.7K | 재사용 + 병목 프로파일링 대상 |
| `dex-decompiler` | 9.6K | 재사용 |
| `backend-java` | 65.7K | **속도·가독성 주전장** (Phase 1~2) |
| `decomp`(facade/CLI) | 55.0K | 재사용 + fast-mode 옵션 노출 |

jadx에서 **가져오지 않는 것**: GUI, 플러그인 로더, dx/d8 Java 입력 변환,
smali 출력, gradle export. deobf/mappings/resources(arsc)는 Phase 3에서
필요 시 선별 도입.

---

## Phase 0 — 레드 베이스라인 정상화 ✅ (2026-10-06 해결)

**결론: 49개 실패 전부 환경 문제 — javac가 PATH에 없고 JAVA_HOME 미설정(NotFound).
코드 회귀 0건. JDK17 지정 재실행에서 353 passed / 0 failed / 14 ignored.**

```sh
JAVA_HOME=/home/jtjisgod/tools/jdk-17.0.13+11 \
PATH=/home/jtjisgod/tools/jdk-17.0.13+11/bin:$PATH \
cargo test -p backend-java --lib
```

- 테스트의 JDK 탐색은 PATH 룩업 / `JAVA_HOME/bin` / 절대경로 3종 혼용.
  일부 테스트는 정확히 JDK17 요구, Java25 픽스처는
  `/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1` 사용.
- 잔여 리스크: JDK 절대경로가 머신 의존(특히 /tmp의 JDK25), ignored 14개 미검증.
- 증적: `/tmp/codepuri-backend-java-{baseline,jdk17}.log`,
  `/tmp/codepuri-backend-java-failing-tests.txt`

## Phase 1 — 속도: jadx 26.26s 대결 승리 (1차 목표, 진행 중)

### 최신 프로파일 (2026-10-06, glm 에이전트, 30워커)

| 항목 | 이전 베이스라인 | 신규 측정 |
|---|---|---|
| wall (IO) | 54.81s | **50.99s** |
| wall (no-io) | - | 37.63s → **IO 점유 13.4s (26%)** |
| peak RSS | 12~14.8 GiB | 4.46 GiB |
| structure_backend (태스크합) | 113.2s | 30.4s (업스트림 최적화 반영) |
| unattributed (태스크합) | - | **59.5s (35.8%) — 최대 버킷** |
| initialization_family | 58.2s | 54.6s |
| queue_wait | 23.8s | 0 |
| checksum (both runs) | - | `c62e548cb933f3c0` 동일 |

no-io wall 분해: 직렬 준비 9.4s + **dex별 순차 스케줄러 8.44s**
(`java_integration.rs:3327` `for dex in files`) + source_throws 6.9s +
deferred_annotation 3.1s + sink/teardown 7.5s. dex3 occupancy 11.3/30 —
`DSTU7624Engine` 단일 태스크 3.10s가 dex3 wall 전체.

**결론: fast-mode(검증 축소)보다 직렬 구간 병렬화 + IO 파이프라인이 본판.
아래 랭킹 합계 잠재 ~20-30s → jadx 26.26s 돌파 가능.**

### 공격 랭킹 (glm 에이전트 top-10, 기대 절감순)

| # | 후보 | 근거 | 기대 절감 | 리스크 |
|---|---|---|---|---|
| 1 | sink/IO 전달 경로 파이프라인 | IO-noio Δ13.4s, backpressure ~8s | 6-10s | low |
| 2 | `cache_release` 백그라운드 drop | 4.12s 직렬 drop (java_integration.rs:2854) | 3.5-4s | 없음 |
| 3 | dex별 순차 스케줄러 4개 병합 | 8.44s 합 vs ~3.3-5s 병합 시 | 3-5s | moderate |
| 4 | 직렬 준비 단계 병렬화 | alias_build 2.59 + inventory 1.73 + indices 1.43 + … | 4-6s | low-mod |
| 5 | deferred_annotation_prepare | 3.07s 단일 스레드 (java_integration.rs:3518) | 2.5-3s | moderate |
| 6 | source_throws 직렬 서브페이즈 | 3.7s 직렬 | 2-3s | moderate |
| 7 | initialization_family straggler | 상위 16개 클래스 9.3s; DSTU7624 1.66s 단일 | 1-2s | high |
| 8 | unattributed 태스크 시간 | 59.5s CPU 롱테일 | ~1s wall | low |
| 9 | structure_precheck+backend | 45.9s CPU | 0.5-1s | moderate |
| 10 | throws SSA CPU 트림 | 53.4s CPU / 191,818 태스크 | <0.5s wall | high |

진행 방식: #1-4는 같은 파일(`crates/decomp/src/java_integration.rs`) 권역 →
단일 에이전트가 순차 구현 + 각 단계 페어 측정. #5-10은 1라운드 착지 후 재평가.

**불변 게이트**: 출력 무결성 — checksum `c62e548cb933f3c0` 유지,
syntax_gate_exit_code 0, javac_diagnostics 44,769, strict_executable_accepted
187,288, executable_definitions 194,286. 출력 변경 없는 성능 작업만.

### R1 결과 (2026-10-06, codex+glm 에이전트) ✅

| 단계 | 내용 | wall (중앙값) |
|---|---|---|
| 베이스라인 | - | 50.99s |
| 1 | pipelined sink 전달 (bounded queue 64, receipt 동기화) | 38.16s |
| 2 | SSA cache background reaper (bounded 1+1, fallback 동기 drop) | 36.31s |
| 3 | dex 스케줄러 4개 병합 (중복 owner 시 per-DEX fallback) | 35.00s |
| 4 | 직렬 준비 병렬화 (inventory/super_index/global_indices) | **31.77s** |

**누적 −37.7%.** 전 단계 출력 바이트 완전 동일(checksum `c62e548cb933f3c0`,
28,575 파일 + JavaReport + unpublished.tsv), 448/448 테스트, clippy/fmt 통과,
품질 게이트 exact(syntax 0 / javac 44,769 / strict 187,288·194,286).
변경 파일: `crates/decomp/src/{java_integration,java_ssa_cache,output}.rs`.
잔여 리스크: 병렬 prep 신규 경로(경험적 검증만), step3 마진 ~1.3s(노이즈 2x).

### R2 결과 (2026-10-06, glm-5.3-flash max) ✅

| 항목 | 내용 | wall (중앙값) |
|---|---|---|
| R1 종료 | - | 31.77s |
| #5 | deferred-annotation 스케줄러 병합 | 31.555s |
| #6 | source_throws 준비 병렬화 (Facts∥InputIndex∥SuperIndex∥metadata) | **31.36s** |

**R2 누적 −0.41s (예상 5-6s 미달).** 게이트 전부 exact 통과
(448/448, checksum 동일, strict/syntax/javac 보존). 커밋 `57b61b4`.

핵심 발견: 남은 시간은 총 CPU가 아니라 **straggler 경계**임.
- 메인 패스 스케줄러 wall 7.37s — 상위 개별 태스크 2.3-3.8s가 지배
- throws 스케줄러 wall 2.16s / CPU tail 62.5s
- sink_io 3.56s, RSS 4.46→4.90 GiB
선형 병렬화로는 ~26s 도달 불가. 다음 레버는 #7 straggler 분할
(긴 클래스 태스크 쪼개기 — high risk, backend-java/초기화 증명 영역)뿐.

### 현황 요약

| 지표 | jadx 1.5.1 | codepuri (R2 후) |
|---|---|---|
| wall @30 workers | 26.26s | **31.36s** (−38.5% from 50.99) |
| syntax error | 2,210 | 0 |
| 출력 | 85.9MB | 바이트 동일성 게이트로 검증 중 |

### R3 결과 (2026-10-06, glm-5.3-flash max) ✅

| 레버 | 내용 | wall (중앙값) |
|---|---|---|
| R2 종료 | - | 31.36s |
| A | 비순차 스케줄러 전달 + ordinal 재조립 (체크섬 순서 보존) | 30.415s |
| B′ | per-method 샤딩은 **증거로 기각** — straggler는 클래스 단위 init family 계획이 병목(메서드 스테이지는 0.03s). 대신 heavy-first dispatch(Σinsns_size) | 29.97s |
| C | DirectorySink 백그라운드 write pool(≤4) + flush barrier | **29.805s** |

**R3 누적 −1.56s, 전체 −41.5% (50.99 → 29.81s).** 게이트 전부 exact.
커밋 `d49a1df`. 핵심 학습: straggler는 initialization_family의
이중 계획(실패 family plan 1.44s + clinit subset 1.46s)이 본체 —
메서드 샤딩으로는 못 쪼갬.

### R4 결과 (2026-10-06, glm-5.3-flash max) ✅

| 항목 | 내용 | wall (중앙값) |
|---|---|---|
| R3 종료 | - | 29.805s |
| 8 | init family attested proof 1회 계산 후 재사용 (clinit 재계획 제거) | 28.35s |
| 9 | throws aggregation BFS → 스케줄러 owner 스레드 스트리밍 (0.555s→1µs) | **27.78s** |
| 10 | build_hierarchy hoist — 노이즈 수준, 규칙대로 revert | - |

**R4 −2.03s. 누적 −45.5% (50.99 → 27.78s).** 8회 측정 전부 바이트 동일.
커밋 `9c2c585`. jadx까지 1.52s.

### R5 대상 (잔여 버킷)

- deferred_annotation_prepare 2.78s / alias_inventory 1.19s / method_closure 1.20s / sink_io 0.87s
- DSTU7624Engine 클래스 페이즈 tail 2.72s (init family 다음 straggler)

### 완료 기준: 동일 프로토콜에서 jadx 26.26s 대비 동등 이하 (목표 ~20s).

## Phase 2 — 가독성: jadx 수준 출력 (91줄 → ~10줄)

1. witness 메서드 세트 구축: `jadx-samsung-pass-output-comparison.json`의
   비교 방식 확장 — 동일 메서드 jadx vs codepuri 줄수/구조 diff 리포트.
2. `backend-java` 출력 단순화: 임시 변수 인라인, 중복 캐스터 제거,
   구조 평탄화(불필요한 블록/라벨), 표현식 병합.
3. 출력 크기 345MB → jadx 대비 ±2배 이내.

## Phase 3 — jadx 패리티 선별

- deobfuscation, rename/mappings(Tiny/Enigma/ProGuard) — 필요 시.
- resources(.arsc/manifest) — 별도 크레이트로 분리 가능 시점에 검토.
- 채택 기준: "jadx에 있어서"가 아니라 실사용 워크플로 필요 여부.

## Phase 4 — 멀티 타겟 확장

순서: JVM `.class`(스택→SSA 변환기로 .NET과 공유) → .NET IL(ECMA-335
메타데이터 파서 직접 구현, ILSpy급 메타데이터 풍부성) → native
(`object`+`iced-x86`/`capstone`+`gimli`, lifting/raising).
프론트엔드만 추가하고 `decomp-core`/`decomp-structure`/`decomp-emit` 재사용.
세부는 DEX 속도·가독성 목표 달성 후 별도 플랜(`docs/`에 개별 문서).

## 운영 규칙

- **벤치마크 프로토콜**: Samsung Pass APK, `--no-res -j 30`, 캐시 초기화,
  `/usr/bin/time -v`, 3회 실행 중앙값. jadx 기준선 26.26s/16.1GiB와
  항상 동일 조건 비교. 결과는 `bench/`에 JSON+MD로 기록(기존 관례 유지).
- **업스트림 동기화**: 업스트림 진행(wave 기반)을 정기적으로
  `git fetch upstream && merge`. 속도/가독성 커밋은 업스트림에 역방향
  제안 가능하게 커밋 단위 유지.
- **품질 게이트** (기존 유지): fmt, clippy -D warnings, test 전 녹색,
  `unsafe` 금지, 라이브러리 `unwrap`/`expect` 금지.
- **측정 없이 최적화 금지**: 모든 속도 PR은 before/after 중앙값 동반.

## 즉시 다음 액션

1. ~~Phase 0.1 — JDK 경로 가설 검증~~ ✅ 환경 문제로 확정 (위 참고)
2. Phase 1.1 — fast-mode 설계: `JavaOptions`에 모드 추가, strict 기본값 유지
   (프로파일링 에이전트 결과 반영해 후보 확정)
3. ~~벤치 재현 스크립트~~ ✅ `bench/jadx_vs_codepuri.py`
   (페어 콜드 익스포트, `--runs N` 중앙값, `--markers` jadx 실패 마커 카운트)
