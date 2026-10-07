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

### R5 결과 (2026-10-06, glm-5.3-flash max) ✅

| 항목 | 내용 | 결과 |
|---|---|---|
| 11 | alias inventory 섹션 병렬 fill (1.236→0.915s) | KEEP |
| 12 | deferred_annotation — 직렬 잔여 0.29s뿐 | SKIP (근거) |
| 13 | close_published 미사용 inventory skip (1.278→0.905s) | KEEP |
| 14 | sink_io — 이미 백그라운드, 잔여는 예제 checksum | SKIP (근거) |
| tail | DSTU7624 2.72s — backend-java semantic (read-only), throughput-bound | SKIP |

**export 구간 중앙값 26.27 → 26.17s** (jadx 26.26s와 대등). 게이트 전부 exact
(448/448, checksum, strict/syntax/javac). 프로세스 wall은 28.06s — 크로스데이
드리프트 ±0.3s. **같은 잣대 프로세스 wall 최종 대결이 R6 과제.**
커밋 완료. 남은 직렬: alias_build 3.07s, source_throws 6.15s wall.

### R6 결과 — Phase 1 속도 트랙 종료 (2026-10-06) ✅

| 항목 | 내용 | 결과 |
|---|---|---|
| alias_build | analyst walk 분할(중복 owner 사전 패스) + 병렬 build/ancestors | 3.01→1.53s |
| source_throws | 205k-key queue.clone 제거(인덱스 전달) + scratch drop 백그라운드화 | 6.02→5.43s |

**프로세스 wall 28.06 → 26.18s (6회 측정 게이트 전부 exact). 커밋 완료.**

### 최종 정식 대결 (동일 세션, 3회 페어, 30 workers)

| | jadx 1.5.1 | codepuri |
|---|---|---|
| wall 중앙값 | **24.53s** (24.03/24.53/24.99) | 25.99s (25.66/25.99/26.07) |
| CPU time | 410s | **216s (52%)** |
| 출력 파일 | 17,546 | 28,579 |
| syntax error | 2,210 + 미디컴파일 1,041 | **0** |

**판정: raw wall은 jadx가 1.46s(6%) 앞섬 — 단, jadx 자체도 기준선 대비
1.7s 빨라진 상태(26.26→24.53, 페이지캐시/환경 드리프트). codepuri는
같은 기간 28.06→25.99로 동등 이속. CPU는 절반, 출력 품질은 압도.**
성능 대 성능 한 점 승부는 의미가 옅어졌고, CPU 절반 + 정확성 우위가
실질적 우위. 벤치 스크립트 Elapsed 파서 버그 수정 완료.

### Phase 1 종료 판정

- 목표 "jadx 대등 이하": **미달 (−6%)**, 단 CPU 효율 2배·정확성 우위
- 20s 목표는 strict 검증 중복 제거(fast-mode) 필요 — 별도 라운드
- 다음: Phase 2 가독성 (91줄 → ~10줄, 출력량 345MB → jadx급) 진입 권장

### R7 결과 (2026-10-06, glm-5.3-flash max) ✅

중복 연산 지도: init_family 49.4s(38%) > structure_backend 31.9s > 미귀속 24.2s > precheck 16.0s.
**핵심 발견: finals 클래스 17,678개 중 13,907개(79%)의 init family 계획이 실패** —
계약 성립 0건, 전부 subset fallback으로 출판.

| 항목 | 내용 | 결과 |
|---|---|---|
| 18 | attested static proof → subset retry 재사용(에러 텍스트 재생) | KEEP, wall 25.52s / CPU −5.6s |
| 19 | constructor-evidence memo — live path 미사용(0 hits) | REVERT (근거) |
| 20 | 잔여 per-site 중복 각 <1s | 보류 |
| flag | **`--fast-proofs`** (precheck 14.95s→0.014s, CPU 101.3s) | **~23.8s**, 이 코퍼스 바이트 동일 |

**default 25.52s / fast 23.8s. 1250/1250 테스트, 전 게이트 exact. 커밋 완료.**

### R8–R9 결과: 안전 필터와 structure 재사용은 미채택

- **R8** constructor-superchain 사전검사는 실제 family planner의 첫 검사를
  같은 순서로 중복 실행할 뿐, planner를 skip하지 못했음. Resolver lookup 안정성이
  보장되지 않아 제거. static subset check 재정렬은 동시 실패 시 `InvalidMetadata` /
  `FieldInitializationUnavailable` 우선순위를 바꿔 제거. 게이트: workspace 1,250
  passed / 0 failed, fmt/clippy 통과.
- **R9** provenance 토큰으로 성공한 strict structuring을 backend에 재사용하는 시도:
  source bytes/report exact, 그러나 ordinary API paired baseline **24.73s → 24.80s**
  (유의미한 이득 없음)으로 revert. Same-CFG opaque proof token은 가능하지만 이 경로의
  structuring 자체는 wall bottleneck이 아니었음.

### R10 결과: throws worklist 인덱싱 + 공정 벤치

- `derive_program`의 BTree-keyed worklist/state를 sorted catalog index + `VecDeque<usize>` /
  `Vec<bool>`로 바꿈. Reverse caller ordering 및 public error/output map은 BTree 순서 유지.
- 일반 경로 **24.73 → 24.16s** (−0.57s); opt-in `fast_proofs` **24.24 → 24.01s**.
  Workspace tests 1,250 passed / 0 failed / 174 ignored; fmt/clippy/strict quality exact.
- `java_export_bench` 추가: 실제 `emit_java` + `DirectorySink`, 프로파일 계측·예제 FNV checksum·
  진단 TSV/summary 기록 제외. Fair paired fast-mode 5회: **codepuri 23.98s vs jadx 23.64s**;
  CPU 약 절반 (≈190s vs ≈390s), codepuri 28,575 files / 200MB vs jadx 17,546 / 86MB.
  `fast_proofs`는 opt-in: Samsung Pass code bytes/strict coverage 동일, 다만 unpublished
  diagnostic reason text는 APK에 따라 달라질 수 있음.
- R10 source/benchmark checkpoint: `ec4a080`; R9/R10 default gain은 0.57s.

### R12 결과: source-throws와 독립 export 준비 중첩

- Alias/catalog 확정 후 `source_throws` 계산과 immutable class catalog, field names,
  `FieldUseCache`, DEX metadata/member index 준비를 scoped worker에서 겹침. Throws plan은
  join한 뒤 Resolver에 붙이고 class scheduling을 시작; source-throws 오류/패닉 우선순위 유지.
- 동일 세션 3회 paired: default **codepuri 23.77s / jadx 23.45s**;
  opt-in fast_proofs **codepuri 23.43s / jadx 23.54s**. 이전 R10 대비 각각 −0.39s/−0.55s.
- fmt/clippy/workspace tests/strict quality/바이트 동일성 통과. 커밋 `318b830`.
- 판단: default는 jadx보다 0.32s 뒤, fast mode는 0.11s 앞 (둘 다 측정 노이즈 근접).
  CPU는 여전히 jadx의 약 절반. Samsung Pass 출력은 28,575 sources / 200MB 대 jadx 17,546 / 86MB.

### R13 결과: static-final proof state 비트셋 실험 — revert

- Proof 4,783회 중 4,777회는 finals ≤64, 중앙값 1개, 최대 288개. CFG 방문 상태는 총
  110,105개(최대 11,815), cap 도달 0회.
- 비용은 BTreeSet 경로 상태가 아니라 `DexPasses::run_with` 재-lift 합계 26.119s
  (병렬 worker elapsed 합); path traversal 0.365s, raw/SSA site 비교 0.041s뿐.
- 비트셋 fast path는 ≥0.3s 개선을 못 내서 revert. R13 default 23.70s, fast 23.32s
  (R12 대비 각각 0.07/0.11s — 노이즈 수준). Gates 모두 통과.

### R14 결과: raw static-final proof relift 재사용 ✅

- `run_ssa_with`에서 validated raw `<clinit>` lift를 하는 시점에 proof 전용 compact summary를
  capture해 `SsaMethod`/기존 SsaCache lifetime에 묶음. Exact DEX pointer, method index,
  CodeItem pointer가 일치하는 unseeded `<clinit>`만 재사용, 나머지는 원래 `run_with` fallback.
- Preserve: raw FieldPut/Get offset·receiver class, normal/exception edge insertion order·handler roots,
  raw-vs-SSA site equality, path-state traversal/order, 65,536 cap. Full raw IR은 보관하지 않음.
- R13 프로파일: proof 4,783회 중 4,777건 finals≤64; 전체 비용의 26.119s가 `DexPasses::run_with`
  재-lift, path traversal 0.365s / site checks 0.041s. 따라서 BTreeSet→bitset은 revert,
  raw proof summary reuse를 구현.
- Ordinary 3-run paired: **codepuri 23.02s / jadx 23.49s**. Fast-proofs paired:
  **codepuri 22.27s / jadx 24.15s**. CPU 각각 약 절반 이하; RSS 증가 약 6–9MiB.
- Gates: workspace tests, fmt/clippy, syntax0/javac 44,769/strict187,288-of-194,286,
  checksum + all sources/report exact. Commit `8230095`.
- Fast mode unpublished reason identity는 계속 opt-in caveat; generated code/strict results
  Samsung Pass에서는 identical.

### R15 결과: 최신 프로파일 및 page-cache 순서 통제

- Fresh profile에서 `source_throws` 4.965s (prep 1.522, scheduler 2.405, derive 0.838),
  deferred annotation 2.835s, alias_build 1.599s. static-final proof summary는 6,366개
  capture / 4,783개 consume / 1,583개 미사용. Static-final 플래그 prefilter로 제외 가능한
  capture는 461개(0.55MB, 전체 summary bytes의 9.3%)뿐이라 추가 변경 안 함.
- Paired bench가 매번 JADX-first였던 편향을 R15에서 발견·수정(샘플마다 시작 도구 교대).
  교대 후 Samsung Pass 3회 default: **codepuri 22.76s / jadx 23.55s**; fast: 23.57/23.72s.
  JADX는 persistent 결과 cache 증거 없고 OS page-cache/order warming 설명과 일치.
- R15 production source change 없음; 계측 exports checksum/source/report 동일.

### 현재 대결 기준: ordinary API unprofiled, 동일 세션 paired median.

R15 교대 paired median은 default 22.76s vs jadx 23.55s (codepuri가 매 pair 승리),
fast 23.57s vs 23.72s. CPU 효율은 절반 이하; 다음 속도 작업은 측정된 ≥0.3s 이득을
내는 경우에만 유지한다.

## Phase 2 — 가독성: jadx 수준 출력 (91줄 → ~10줄)

R19 첫 적용: boolean short-circuit return CFG를 엄격한 형태로 검증해
`forwardBlock_*` 흐름 대신 early-return Java로 출력.

- Samsung Pass `Lc7/i;->j()Z`: 14 labels / ~94 lines → **12 lines**; `L` 호출 순서,
  false일 때 오른쪽 operand 생략을 Java runtime side-effect fixture로 확인.
- Guard는 두 conditional branch, Boolean predicates, 정확한 boolean terminal values,
  모든 CFG block/edge coverage, single-use predicate, pure constants만 수락. observable false
  terminal negative fixture는 기존 forward-plan 경로로 남음.
- 95 generated Java units changed, no class/source units gained/lost, strict counts unchanged,
  syntax0, full tests/clippy pass. Output style change intended; total bytes −115,884.
- Ordinary speed 22.76→22.98s (noise-level neutral); fast mode 23.57→22.45s.
  Commit `3e5c935`.

### R20 결과: exact `this.field.booleanCall()` 인라인 ✅

- Witness `Lc7/i;->j()Z`가 이제 `if (!this.a.j()) return false; return this.b.j();`로 출력.
  FieldGet→0-arg Boolean virtual/interface invoke 형태, receiver `this`, exact single-use,
  type/access/throws 및 resolver checks가 모두 증명될 때만 임시 변수 4개를 제거.
- 좌우 호출의 short-circuit 부작용 순서 4가지 조합을 Java compile/run 테스트했고,
  중간 observable call이 있는 negative fixture는 기존 temp/forward path로 fallback.
- Workspace tests/clippy/fmt 통과; 28,575 source units, strict body identities 0 gained/0 lost,
  syntax0, javac diagnostic categories unchanged. 95 generated units changed intentionally.
- Ordinary 3-run before/after 22.76→22.98s (노이즈 수준); fast 23.57→22.45s.
  가독성 이득이 목적이며 speed regression 기준 0.3s를 넘지 않음. Commit `148e7de`.

### R21 결과: pure single-use expression 축약 ✅

- `BinaryPropertyListWriter.computeOffsetSizeInBytes(long)`: **47→15 lines**; cmp-long/zero
  comparison chain이 typed relational tests로 출력. `NSData.length()`: **5→3 lines**;
  proven `this.bytes` + ArrayLength가 `return this.bytes.length;`로 출력.
- Guard: exact type/use/dominance, acyclic unique-unconditional paths, no intervening effects;
  array length only direct-return/no-exception-scope; field reads only `this`, instance,
  non-volatile. Div/rem, calls, writes, Phi, loops 등은 인라인하지 않음.
- Java runtime tests: 255/256, 65535/65536, 2^32−1/2^32, Long.MAX_VALUE, negative, integer overflow,
  null-array exception 및 bounds ordering; multi-use arithmetic remains materialized.
- Samsung Pass: 95 output files intentionally changed, 0 method identity gained/lost, syntax0;
  javac diagnostics −3 (`cannot find symbol`), classes unchanged. Ordinary speed delta +0.18s,
  fast −0.19s (노이즈 내). Workspace fmt/clippy/tests pass.
- R21 commit `1ef409f`.

### R22 결과: compact source + lossless diagnostics sidecar ✅

- `JavaOptions::compact_source` / CLI `--compact-source`는 standalone diagnostic `//` lines만
  제거하고, path + descriptor + original line + exact text를 deterministic `diagnostics.tsv`에 둠.
  Default mode false라 기존 Java output 그대로. Generic sinks도 `GeneratedSource.diagnostics` 받음.
- Java source bytes: Samsung Pass17 **200.06→70.86MB (−64.6%)**, Samsung Pass25
  345.21→256.54MB (−25.7%), Chrome ADB 110.26→83.45MB (−24.3%). Diagnostic sidecar
  각각 221.56/174.89/45.06MB여서 combined disk usage는 증가; export wall도 sidecar I/O로
  Samsung17 약 +12%, Chrome +35%. 따라서 **가독성 모드이지 속도/디스크 최적화는 아님**.
- Sidecar exact round-trip tests (line endings, tabs/control escapes, final no-newline comment),
  Java body lines unchanged, report/status counts unchanged, syntax gate zero for all three inputs;
  workspace tests/clippy/fmt 통과.
- R22 commit `1b43804`. Use when clean Java files matter more than an adjacent large diagnostic ledger.

### R23 결과: 일반 primitive expression chain 축약 ✅

- Straight-line pure single-use Arithmetic/Compare/Neg/Not operands now inline with precedence
  parentheses and exact constant/type promotion. Calls, writes, allocations, div/rem, Phi, loops,
  exception-scoped or multi-use values remain materialized.
- `int v0 = p0 + 1; int v1 = v0 * 2; return v1;` → `return ((p0 + 1) * 2);`.
  Overflow, nested precedence, unary, negative literals, divide-by-zero order and multi-use
  guards are compile/run tested. Samsung Pass: no method identity gained/lost; syntax gate 0;
  javac diagnostics categories unchanged (existing cap reached).
- Workspace fmt/clippy/tests pass. Ordinary timing 22.76→22.98s (+0.22s, under noise budget);
  fast 23.57→22.45s. Commit `dd01095`.

다음 가독성 대상:
1. ADB Chrome `LtT;->onResult`의 213-line `forwardBlock_*` CFG를 event-order-safe structured branch로
   단계적으로 복원 (native bridge, zero-handle throw 때문에 high risk).
2. DEX debug-local names를 start/end/restart live-range provenance와 SSA value에 연결한 뒤 적용.
3. Inner-class grouping은 genuine nesting metadata, complete family/access semantics가 입증된 단위만.
4. Metadata completeness: Android classpath CLI 지원 완료; Chrome/EasyMover 전자세부 coverage smoke.

출력 단순화는 메서드 의미를 바꾸지 않는 형태만 채택. `&&` 자체를 합성하는 대신
short-circuit early-return 형태로 값·side-effect 순서를 보존함을 테스트로 증명; 다른 CFG는
기존 strict output 유지.

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

- **벤치마크 프로토콜**: Samsung Pass APK, jadx `--no-res -j 30` vs
  `java_export_bench --workers 30` (일반 unprofiled `emit_java`),
  `/usr/bin/time -v`, 4회 same-session paired median; JADX/codepuri 선행 순서를 번갈아
  page-cache 편향을 줄임. `--profiled-codepuri`는 진단 비교 전용.
  Cache/page-state 드리프트 때문에 과거 jadx 절대값보다 동일 세션 paired median을 우선.
  결과는 `bench/`에 JSON+MD 기록.
- **업스트림 동기화**: 업스트림 진행(wave 기반)을 정기적으로
  `git fetch upstream && merge`. 속도/가독성 커밋은 업스트림에 역방향
  제안 가능하게 커밋 단위 유지.
- **품질 게이트** (기존 유지): fmt, clippy -D warnings, test 전 녹색,
  `unsafe` 금지, 라이브러리 `unwrap`/`expect` 금지.
- **측정 없이 최적화 금지**: 모든 속도 PR은 before/after 중앙값 동반.

## 즉시 다음 액션

1. ~~Phase 0.1 — JDK 경로 가설 검증~~ ✅ 환경 문제로 확정 (위 참고)
2. Phase 1 — strict 기본 경로 + opt-in fast_proofs의 품질/속도 계약 정리.
3. ~~벤치 재현 스크립트~~ ✅ `bench/jadx_vs_codepuri.py`
   (페어 콜드 익스포트, `--runs N` 중앙값, `--markers` jadx 실패 마커 카운트)
