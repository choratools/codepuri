# 기능별 시간·공간 복잡도

분석 기준일: 2026-10-01.

현재 소스 구현을 읽어 산정한 복잡도다. 전체 파이프라인을 `O(N)` 또는
`O(I)`로 묶을 수 없다. CFG 검증, 구조화, Phi 처리, 타입 전파, 이름 충돌,
멤버 의존성 정리에 비선형 비용이 남아 있다.

이 문서는 실측 시간이나 향후 구현의 성능 보장을 대신하지 않는다.
구현 변경 후에는 해당 행과 소스 근거를 갱신해야 한다.

## 기호와 산정 기준

| 기호 | 의미 |
|---|---|
| `N` | 입력 또는 해제 데이터 바이트 수 |
| `Z`, `z` | ZIP 엔트리 수, 선택한 DEX 엔트리 수 |
| `C`, `M`, `F`, `T` | 클래스, 메서드, 필드, 타입 수; pool과 실제 선언 수는 문맥에 따라 구분 |
| `n` | 메서드 code-unit 수 |
| `I` | IR 명령 수; 합성 명령 포함 |
| `B`, `E` | 해당 단계 CFG 블록·엣지 수 |
| `V`, `R` | SSA 값 수, 실제 register-slot 수 |
| `G` | 레지스터 history에 저장된 value membership 총수; wide 값은 두 history에 속할 수 있음 |
| `P`, `A` | Phi 수, Phi 입력 총수 |
| `p_b`, `d_b` | 합류점의 Phi 수, incoming edge 수; `A = Σ p_b d_b` |
| `U` | IR 인자·사용·정의 등 가변 길이 목록의 방문 총량 |
| `D` | dominator parent-walk 최대 길이 |
| `J`, `w` | multi-def candidate 변수 수, `max(1, ceil(J/64))` bitset word 수 |
| `τ`, `a` | 타입 객체 크기, 배열·타입 재귀 깊이; 이름 문자열도 타입 크기에 포함 |
| `Y` | 생성 소스 바이트 수 |
| `K`, `ℓ` | 해당 행의 메타데이터 항목 수, 최대 문자열·key 길이 |
| `k` | 고정점·정리 알고리즘의 반복 횟수 |
| `X` | `Σ(deg⁺² + deg⁻²)`: 인접 리스트 재검색 비용 |
| `K_IR` | 명령의 owned 목록·타입 tree·register history를 포함한 실제 IR 복제 크기 |

- 메서드별 비용은 앱 전체에서 합산된다. merge 전, chain mirror,
  materialize 후의 `B`, `E`는 서로 다르다.
- 해시 조회의 상수 비용은 기대/amortized 기준이다. key 해시·비교·복사에는
  문자열 길이가 포함된다. hash collision까지 포함한 절대 최악 `O(1)`을 주장하지 않는다.
- 공간은 입력 자체 외 추가 공간이며 캐시, 중간 표현, 생성 결과를 포함한다.
- 고정점 분석에는 반복 횟수 또는 누적 작업량을 남긴다. 단조성 설명만으로
  반복 횟수가 상수라고 가정하지 않는다.
- 디스크·압축 라이브러리 내부 비용과 OS 지연은 별도다.
- 아래 소스 경로는 `crates/` 기준이다.

## 1. APK·DEX 파싱 및 명령 디코딩

| 기능 | 시간 | 추가 공간 | 구현 근거 및 주의점 |
|---|---|---|---|
| ZIP inventory·multidex 추출 | `O(엔트리 이름/메타데이터 크기 + z log z + 해제 DEX 바이트)` + 파싱·ZIP 라이브러리 비용 | inventory + 해제 DEX + 모델 | `dex-format/src/apk.rs::parse_apk`; 선택 엔트리 정렬 후 해제 |
| DEX container·header·map·raw sections | `O(N + Σ map_items log map_items)` + 하위 파싱 | `O(N + map_items)` + 모델 | `dex-format/src/model.rs`, `header.rs`, `map.rs`; 원본 Arc 공유, map 중복 검사 |
| ID·prototype·interface type-list 파싱 | `O(ID 항목 + type-list 항목)` | 동량 | `dex-format/src/ids.rs`; 동일 참조 list도 다시 파싱 |
| checked ID accessor | `O(1)` | `O(1)` | `dex-format/src/model.rs`; 반환 데이터의 별도 처리 비용 제외 |
| strings·MUTF-8 | `O(참조별 읽은 문자열 바이트 합)` | 디코딩 문자열 합 + 최대 UTF-16 임시 buffer | `strings.rs`, `uleb.rs`; 동일 offset도 재디코딩 가능 |
| 고정 폭 LEB 값 | 값 하나당 `O(1)` | `O(1)` | `dex-format/src/uleb.rs`; 고정 bit 폭 제한 |
| class data·annotations·encoded arrays | `O(C + F_decl + M_decl + 펼쳐진 annotation/list 크기)` + code 파싱 | 모델 크기 + annotation 재귀 깊이 | `class.rs`, `annotations.rs`, `encoded_value.rs`; offset memoization 없음, encoded value depth guard 256 |
| code·try·handler 파싱 | `O(n + t + h + handler 항목 + t·h)` | `O(n+t+h+handler 항목)` | `dex-format/src/code.rs`; try마다 handler offset을 선형 검사 |
| strict/recovering decode | `O(n + U_dec + 참조별 switch target 검사량)` | decoded 명령·operand·진단 | `dex-decompiler/src/decode/{mod,recover,payload,operands}.rs`; recovery budget은 모든 진단의 상한이 아님 |
| payload indexing·lowering·register binding | `O(I_dec + payload 크기 + U_lift + 타입 조회 비용)` | payload + `I+V+R+G` + 타입 크기 | `lift/{payloads,lower,values}.rs`; payload 복사, fill folding은 참조당 최대 4096 elements |

파서의 선형 기준은 물리 파일의 유일 바이트 수와 다를 수 있다.
strings, lists, annotations, code items의 공유 offset을 여러 항목이 참조하면
반복 파싱·보유할 수 있다. 전체 parser의 더 정확한 형태는 다음과 같다.

```text
O(N + 참조별 파싱 총량 + map 정렬/중복 검사 + Σ try_count·handler_count)
```

## 2. CFG 및 그래프 분석

| 기능 | 시간 | 추가 공간 | 구현 근거 및 주의점 |
|---|---|---|---|
| block·edge 추가 | amortized `O(1)` | 전체 `O(B+E)` | `decomp-core/src/cfg/mutation.rs`; edge kind 문자열 비용 별도 |
| edge 삭제·redirect·split | 관련 endpoint degree에 비례 | degree 크기 임시 자료 | 인접 Vec의 검색·retain·이동 |
| CFG validate | 기대 `O(B+E+L_adj)`; 정상 CFG에서 `L_adj=2E` | `O(E+최대 degree)` | `cfg/validation.rs`; 방향별 edge membership set, 기존 오류 검사 순서 유지; 손상된 adjacency의 실제 길이도 포함 |
| exception 후보 및 CFG assembly | `O(t·h + Σ 보호 code-unit 수·handler 수 + I+E+U+X)` | 후보 + IR·CFG | `lift/lower.rs::exception_edges`, `lift/cfg_build.rs::assemble`; 모든 보호 code-unit offset에 후보 생성 후 throwing/명령 위치로 필터 |
| merge/fuse·try masks·dead pruning | 보수적 `O(K_IR + B·min(t,64) + E + B² + X)` | `O(K_IR+B+E)` | `merge/fuse.rs`, `merge/mod.rs`; union-find path compression만 있고 rank 없음 |
| chain mirror·variable/width lineage | `O(I+E+V+G+R log R)` | `O(I+E+V+G+R)` | `ssa/chains.rs`, `ssa/vars.rs`; history 정렬·방문 |
| DFS/RPO | `O(B+E)` | `O(B)` | `decomp-core/src/analysis/traversal.rs`; iterative |
| Tarjan SCC | `O(B+E)` | 보수적 `O(B+E)` | `analysis/scc.rs`; frame이 successor Vec 보유 |
| CHK dominators | `O(B+E+k_d(B+E·D))` | `O(B+E)` | `analysis/dominance.rs`; dense RPO fixed point |
| core postdominators | CHK와 같은 형태 | `O(B+E)` | `analysis/post_dominance.rs`; reverse graph·virtual exit |
| parent lookup / 일반 dominates | `O(1)` / `O(D)` | `O(1)` 조회 | `Dominators`; 일반 dominates는 parent walk |
| dominance frontier | `O(B+E+F_D+Σ f_b log f_b)` | `O(B+F_D)` | `analysis/frontier.rs`; 이미 방문한 ancestor suffix 중단, 출력 정렬 |
| natural loops | `O(E·D + L_back(B+E·D))` 보수적 | `O(B+E+L_back·B)` 보수적 | `analysis/loops.rs`; backedge별 역탐색 |
| reducibility 검사 | dominator 구축 + `O(B+E·D)` | `O(B+E)` | `analysis/reducibility.rs` |

Natural loop 분석 구현과 Java loop 구조 복원 구현은 별개다.

## 3. SSA 변환과 검증

| 기능 | 시간 | 추가 공간 | 구현 근거 및 주의점 |
|---|---|---|---|
| 공유 SSA 분석 컨텍스트 | dominator·frontier 구축 1회 | `O(B+E+F_D)` | `ssa/analysis.rs`; CFG immutably borrow, topology 변경 시 재사용 불가 |
| dead synthetic 제거 | `O(I+U+V+S)` | `O(I+U+V+S)` | `ssa/place.rs`; 사용 수 worklist, 실제 dead DEX 명령과 합성 정의 구분 |
| 살아 있는 synthetic 배치 | `O(k_s(I+B+S+U_s)+S·I+S log S)` 보수적 | `O(I+U+S+B)` | 반복 position map 재구축, unplaced 검사, `Vec::insert`; `k_s≤S+1` |
| liveness | `O(I+U+B+E+k_l(B+E)·w)` | `O(B·w+B+E+J)` | `ssa/liveness.rs`; bitset·deduplicated worklist |
| pruned Phi 배치 | `O(I+V+W_DF+P log B+A)` | `O(I+P+A)` | `ssa/phis.rs`; `W_DF`는 실제 frontier 방문량, 초기 definition block 중복 enqueue 가능 |
| SSA rename·Phi fill | `O(K_IR+B+E log B+P+A+R₀ log R₀+G)` 보수적 | `O(I+V+P+A+R₀+B)` | `ssa/phis.rs::PhiDef::fill`; edge→input position hash index; instruction clone 포함 |
| trivial Phi cleanup | `O(I+U+A+B+W_φ·P+P²)` 보수적 | `O(I+U+P+A+B+H_dep)` | `ssa/phis.rs`; `H_dep`는 누적 distinct (resolved target, dependent Phi) memberships, 보수적 `H_dep≤A+P²`; replacement traversal의 비선형 상한 유지; safety filtering 반복 제거 |
| Phi dominance 안전성 검사 | 인덱스 구축 `O(B+I+U+A+P)`, candidate use 총량 `O(U_rep)` | `O(B+I+U+A+P)` | interval dominance, chain→block index; immutable direct target별 1회 검사, same-chain strict position 유지 |
| materialize·provenance remap | `O(I+P+E+A+U+B log B)` | `O(I+P+E+A+B)` | `ssa/materialize.rs`; 명령·Phi마다 공개 CFG 블록 생성, 최종 indexed validate 포함 |
| SSA layout·defs·uses 검증 | `O(B+E+T_dom+I+U+A+V+G+Σ g_v log g_v)` | `O(B+E+V+G+진단)` | `managed-ir/src/verify*`; interval dominance·Phi-head memoization·incoming set cache·wide history 정렬 |
| 검증기 dominance query | `O(1)` | index `O(B)` | `verify/uses.rs::DominanceIndex`; 일반 core dominates와 다름 |

`S`는 합성 정의 수, `R₀`는 fallback initial 수, `W_φ`는 누적 Phi input
재평가량이다. 실제 dependency shape가 반복량을 결정한다.

현재 공개 SSA는 **명령어 하나당 블록 하나**다. 병합 뒤 materialize에서
`B`가 다시 커지며, 후속 검증·구조화 비용에 영향을 준다. 단순한 상수 비용이
아니라 입력 그래프 크기를 부풀리는 표현상의 비용이다.

## 4. 공유 캐시·타입 추론

| 기능 | 시간 | 추가 공간 | 구현 근거 및 주의점 |
|---|---|---|---|
| DexMetadata 구축 | `O(type descriptor 길이 + T + F_pool + M_pool + Proto + prototype 항목)` | predecoded 타입·서명·인덱스 | `infer/context.rs::DexMetadata::new`; 활성 Java export에서 DEX당 1회 |
| cached method-local context | 반환 타입 복제 + local 준비 | 반환 타입·local 진단 | `DexTypes::with_metadata`; 공유 캐시와 진단 상태 분리 |
| type/field/return access | `O(τ)` | owned clone `O(τ)` | index는 상수, 반환 타입 복제는 아님 |
| method params access | `O(인자 수·τ)` | 인자 타입 목록 | prototype cache에서 owned 결과 생성 |
| meet·join·type equality/clone | 최악 `O(τ)` | 반환 tree `O(τ)`·stack `O(a)` | `managed-ir/src/types.rs`; 배열과 문자열 비용 |
| managed descriptor parse | `O(descriptor 길이)` | 타입 크기·재귀 stack | `infer/descriptor.rs`; Java parser와 depth 정책 차이 있음 |
| inference 전체 | `O(I+U+Vτ+Q_eval·τ+W_build+L_eval·τ+W_notify)` | `O(I+U+Vτ+L_cache+최대 탐색량+refinement 크기)` | `infer/mod.rs`, `expansion.rs`; 고정 topology만 캐시, 매 평가에서 현재 타입 재조회; fallback 후 2차 run 포함 |
| Phi/Move 전이적 타입 확장 | root/mode 최초 최대 `O(V+U)`, cache 조회 기대 `O(1)` + leaf별 현재 lattice 처리 `O(L_root·τ)` | `O(L_cache+V+U)`; `L_cache=Σ queried root/mode leaves`, 최악 `O(V²)` | `infer/expansion.rs`; LIFO·cycle·leaf 순서 유지, 다른 root의 cache를 splice하지 않음 |

캐시로 descriptor index 재구축은 제거됐지만, 타입 결과의 owned clone과
같은 root/mode의 topology 반복 탐색은 제거됐다. `W_build`는 최초 탐색
누적량, `L_eval`은 모든 평가에서 방문한 leaf 총량이다. 서로 다른 root의
최초 탐색 및 potentially quadratic leaf-cache memory는 남는다.
고정점 반복 횟수를 상수로 가정하면 안 된다.

## 5. 제어 흐름 구조화

| 기능 | 시간 | 추가 공간 | 구현 근거 및 주의점 |
|---|---|---|---|
| acyclic 순서 확인 | `O(B+E)` | `O(B+E)` | Kahn 순서 |
| 기본 if-region structure | 기대 `O((B+E) log(max(2,B)))` | `O(B log(max(2,B))+E)` | `postdom.rs`; reverse DAG immediate-postdom tree + binary-lifting LCA, virtual exit; validation 포함 |
| extended switch structure | 기본 비용 + `Σ switch_degree²` | `O(B log(max(2,B))+E)` | `extended.rs`; target grouping `.position`는 남음 |
| region tree 생성 | 기대 `O(B+E)` | `O(B+E)` | explicit frames; 성공 시 각 block 1회, recursive tree 표현 자체는 유지 |
| loop source structure | 산정 불가 | 산정 불가 | 미구현; core loop 분석과 별개 |
| exception source structure | 산정 불가 | 산정 불가 | 미구현 |

Core postdominator의 dense CHK과 structurer의 DAG-specific LCA는 별개다.
이전 집합 알고리즘은 differential test oracle에만 남아 있다. Extended
switch target grouping의 quadratic degree 비용은 이번에 바꾸지 않았다.

## 6. Java 선언·alias·resolver·멤버 의존성

| 기능 | 시간 | 추가 공간 | 구현 근거 및 주의점 |
|---|---|---|---|
| descriptor·incoming layout | `O(descriptor 길이)` | parsed 타입 | `java_descriptors.rs`, `java_incoming_registers.rs`; Java array rank 255 제한 |
| recursive Java 타입 rendering | 최악 `O(aτ)` | source + `O(a)` stack | 배열 suffix를 단계별로 복사할 수 있음 |
| class·method 선언 | 메타데이터·lookup·생성 문자열 비용 | 생성 선언 | `java_class_kinds.rs`, `java_method_flags.rs`; flag planning 자체 `O(1)` |
| annotation 반환 타입 검증 | local class 탐색 시 `O(Cℓ)` 추가 가능 | 검사 자료 | 전체 annotation 복원과 별개 |
| initial parameter mapping | 보수적 `O(R log R+V log V+G(log V+log R)+p log R+G_in+G_bound)` | `O(R+V+G+p)` | `java_initial_parameters.rs`; value→register BTree index; 입력 register history 검사와 bound membership 합계, whole history 재검색 제거 |
| alias inventory | `O(K_sym·ℓ log K_sym)` 보수적 | symbol key bytes | `java_integration.rs::inventory`; 모든 input DEX pool |
| alias build·collision allocation | 정렬·prefix·signature 처리 + `O(Q_suffix·ℓ log K_sym)` | aliases·key·prefix·counter bytes | `java_aliases.rs`; sanitize key 1회, owner별 member index, base별 next suffix; overlapping base/reserved 충돌의 실제 총 시도 `Q_suffix` 유지 |
| alias lookup | `O(ℓ log K_sym)` | 작은 lookup 비용 | BTree 기반 |
| indexed Java resolver 구축 | `O(Σ A_c log A_c+Σ M_c(log A_c+log M)+F log F)` + 실제 annotation/throws/alias 처리 | `O(M+F+최대 A_c+throws)` | `Resolver::new`; annotation member index는 duplicate/original order 보존; 동일 method 참조의 실제 annotation 방문량 포함 |
| static field 위치 검색 | 구축 `O(F_static log F_static)`, 조회 `O(log F_static)` | `O(F_static)` | `(class,field)→position` BTree index; first duplicate position 보존 |
| member requirements | IR 방문 + resolution 비용 | 의존성 집합 | 실제 타입·명령 요구에 의존 |
| member closure | `O((m+r_m+r_f) log(max(2,m+F)))` 보수적 | `O(m+r_m)` + 후보 source·의존성 | `closure_failures`; reverse dependency FIFO, candidate 1회 제거; `r_m/r_f`는 requirement memberships, 초기 simultaneous-round reason 보존 |
| legacy/public resolver | class/member 수에 비례한 검색 가능 | 조회 자료 | 활성 indexed Resolver와 동일 복잡도 아님 |

`M_c`는 class method 수, `A_c`는 annotation directory 크기다.
Alias suffix에는 next-suffix 인덱스가 적용됐다. 문자열·prefix·signature 복사,
정렬 및 실제 충돌 시도량은 여전히 계산해야 한다.

## 7. Java 본문·Phi copies·초기화

| 기능 | 시간 | 추가 공간 | 구현 근거 및 주의점 |
|---|---|---|---|
| 일반 instruction emission | operand·type·name 처리 + `O(Y)` | locals·source | `backend-java/src/lib.rs` |
| branching/Phi plan | 구조화 + 기대 `O(B+E+U+V+Aτ+W_delta+W_step)` | `O(I+U+A+Vτ+H_delta+region_depth)` | `branching.rs`; cluster당 backward/forward traversal 1회·incoming set; `W_step`은 nested Step Vec 이동 총량 |
| if arm available-set | 기대 `O(W_delta)` | `O(V+H_delta+region_depth)` | branch delta journal rollback/intersection; `H_delta`는 동시에 보유한 suspended arm deltas; nested join에 같은 value가 다시 포함될 수 있어 `W_delta,H_delta=O(region_depth·(V+A))` 보수적; full-set clone 제거가 선형 worst-case 보장은 아님 |
| simultaneous copy schedule | `O(copy 수)` | snapshot 수 | 모든 snapshot 후 대입; swap 의미 보존 |
| copy temporary 이름 검색 | 기대 `O(Q_temp·ℓ)` | reserved name bytes | `lib.rs`; reserved HashSet + monotonic next; 실제 collision 시도량 `Q_temp` |
| nested Step/source rendering | `O(Step nodes+Y)` + instruction/type/name 비용 | `O(Y+region_depth)` | direct depth rendering; 새 line만 1회 indent, 중첩 output 전체 재indent 제거; plan의 Vec 이동 비용은 별도 |
| constructor plan | linear emission + delegation 탐색·parameter 이름 처리 | plan·body | `constructors.rs`; 안전한 제한된 chain만 지원 |
| static initializer | linear emit + `O(write 수·허용 field 수·τ)` | body source | `static_initializer.rs`; 검증용 emit 뒤 재출력, writable field Vec 검색 |
| class serialization | source bytes 및 declaration 처리 | `O(Y)` | class별 결과 생성, 완전 앱 복원 아님 |

Java backend의 cluster 재탐색 및 incoming membership의 이전 quadratic
요인은 제거됐다. Recursive lowering/rendering 및 nested Step Vec 이동,
delta 재처리 비용은 남으며 verifier와는 별도 구현이다. Direct rendering은
benign whitespace formatting 차이가 가능하나 이번 SmartThings 출력에서는
diagnostic 외 formatting 차이가 없었다.

## 8. 스케줄러·파일 출력·공통 인프라

| 기능 | 시간 | 추가 공간 | 구현 근거 및 주의점 |
|---|---|---|---|
| persistent class task 배정 | task 수 + 동기화 비용 | worker·queue 상태 | `java_scheduler.rs`; atomic 배정, delivery마다 notify_all |
| ordered result delivery | 중심 비용 `O(C log W)` + callback | **`O(W·최대 class 결과 바이트)`** | BTree reorder buffer; `W=max(1,8·workers)` |
| worker IR | task 실제 작업량 | `O(workers·active_IR)` 보수적 | class 안 method IR은 local |
| profiling | 기록 수에 비례 | class profile `O(C)` | 활성화 여부에 따라 추가 비용 |
| DirectorySink | `O(path bytes+prefix 처리+최초 child-name scan+Y)` + I/O | directory·name·duplicate path 캐시 | `output.rs`; directory 최초 1회 scan, create_new로 덮어쓰기 방지 |
| 파일 write | 파일 수×open/create latency + 바이트 I/O | OS buffer 등 | flush는 fsync와 다름 |
| generic Doc render | `O(doc nodes+Y)` | source·doc stack | `decomp-emit/src/document.rs` |
| source-map lookup | ordered non-overlapping single-line map `O(log K)`, fallback `O(K)` | map `O(K)`, 추가 flag `O(1)` | `sourcemap.rs`; overlap/out-of-order/empty/multiline 입력은 기존 first-match scan 유지 |
| precedence table build | `O(operator_count² + 문자열 정렬 비용)` | table | `precedence.rs`; 작은 설정 데이터 |
| NameAllocator | sanitize + `O(Q_suffix·ℓ log K)` 누적 | 이름·base counter `O(Kℓ)` | `naming.rs`; BTree next-suffix, reserved/raw 충돌 의미 유지 |
| pass registry 등록 | 총 `O(pass_count²)` 가능 | registry | `decomp-core/src/pipeline.rs` |
| pass scheduling | invariant/provider match·edge 정렬 비용 | registry·dependency edges | 변환 알고리즘 자체 비용과 분리 |

병렬화는 총 계산량을 줄이지 않는다. 실행 시간은 큰 class의 편차,
ordered-delivery head-of-line 대기, admission window, serial sink에 좌우된다.
`총 작업량/workers`를 보장하지 않는다. 현재 window는 결과 **개수**를
제한하지만 결과 바이트 수 자체는 제한하지 않는다.

`--limit`도 frontend 작업량의 강한 상한이 아니다. class task는 limit 없이
계산하고 delivery에서 global limit를 적용하므로 이후 결과가 버려질 수 있다.
class filter는 task 내부에서 early skip한다.

## 개선 우선순위

실측 우선순위와 동일하다고 단정하지 않는, 확장성 위험 기준 순위다.

| 순서 | 대상 | 이유 | 개선 방향 |
|---|---|---|---|
| 1 | 공개 SSA의 instruction당 block | 후속 단계 입력 `B` 증가; 이번에도 unchanged | Phi·명령 목록·terminator를 가진 basic block 표현 |
| 2 | parser try×handler 및 exception assembly | try마다 handler 검색·보호 offset 후보 확대 | offset index 및 interval 기반 처리 검토 |
| 3 | 살아 있는 synthetic placement | 반복 position map·Vec insert 비용 | 안정된 순서·위치 자료 구조 검토; 이번에도 unchanged |
| 4 | 타입 leaf cache | 서로 다른 root의 탐색·최악 quadratic memory | 공유 표현 또는 bounded cache 검토, leaf order 보존 |
| 5 | merge·Phi cleanup·core dominance | 반복·parent walk·mutation adjacency 비용 | 실제 hot path별 index/worklist, 의미 보존 검증 |
| 6 | extended switch·backend nesting | degree² grouping·delta/Step 이동·recursive consumers | grouping index 및 iterative consumers 검토 |
| 7 | ordered admission·출력 limit | head-of-line 대기·불필요 계산 | byte-budget·취소·정렬과 실행 분리 |

이미 개선된 비용: DEX별 metadata 공유, dense CHK, liveness worklist,
dead synthetic use-count worklist, 공유 SSA 분석, verifier dominance interval,
verifier Phi-head cache, persistent class scheduler, directory cache,
DAG LCA postdominator·iterative region build, indexed CFG validation·Phi fill·
Phi safety, topology leaf cache, annotation/static-position index,
parameter membership index, reverse closure worklist, alias/naming suffix
counters, source-map binary lookup, backend Phi cluster index·delta journal·
direct depth rendering.

## 미구현과 감사 범위

미구현 기능은 산정할 코드가 없으므로 복잡도를 확정하지 않는다:
loop source structuring, exception reconstruction, binary XML/resources,
annotation defaults, enum constructor 복원, 일반 cross-class closure 등.
Extended switch structure가 구현돼도 활성 Java export의 switch emission은
미지원이다.

다음은 이번 감사에서 별도 제외했다: 각 test/example/bench harness의 비용,
ZIP/압축 라이브러리 내부 알고리즘, OS latency, 실제 상수·peak RSS.
`DUMP_SSA_FAIL` 추가 진단 경로는 value마다 history를 재검색하여
`O(VG)` 및 출력량 비용이 추가될 수 있다.

실측 참고:

- `../bench/SMARTTHINGS_PROFILE.md`
- `../bench/SMARTTHINGS_CACHED.md`
- `../bench/SMARTTHINGS_ALGORITHMS.md`
- `../bench/JAVA_INTEGRATION.md`

**요약:** 캐시와 병렬화로 상수 비용은 크게 줄었지만, 큰 CFG·많은 Phi·깊은
타입·긴 의존성 chain의 비선형 비용은 남아 있다. Big-O와 실제 profile을
함께 사용하여 다음 개선을 결정한다.
