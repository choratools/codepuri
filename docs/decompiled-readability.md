# 의미를 보존하는 디컴파일 출력 재구성

목표는 주석/공백 제거가 아니라 임시변수, 분기 중첩, PC dispatch를
읽을 수 있는 표현식과 제어 구조로 재구성하는 것이다.

## 적용한 변환

- **SSA 표현식 복원** (`pure_expression.rs`): 정확한 타입의 single-use primitive
  연산과 Move 체인을 소비 표현식에 포함한다. 유일한 unconditional 경로에서
  정확한 비예외 primitive 연산은 넘어갈 수 있다. 호출, heap/volatile 접근,
  할당, 예외 가능 연산, Phi, 분기/합류는 이동 경계를 유지한다.
- **표현식 예산**: 연산 노드 128개와 깊이 64를 제한한다. 깊이가 작은 넓은 트리도
  크기 제한을 받으며, 초과한 입력 부분식은 변수로 남긴다.
- **구조화된 source AST** (`readable_ast.rs`, `lib.rs`): 이미 타입과 SSA 경로를
  검증한 ordinary/loop-control plan에서 AST를 만든다. 임의 Java 파일을 파싱하여
  치환하지 않는다. 개별 typed emitter의 한 줄 대입/return만 명시적인 AST 노드로
  승격하며, 나머지 출력은 opaque statement로 둔다.
- **조건 복원**: 인접한 single-use Boolean 대입을 조건에 넣고, 중첩 if를 `&&`,
  공통 true-body의 else-if를 `||`로 합친다. 오른쪽 조건의 short-circuit 실행을
  보존하며, Phi-edge 읽기도 사용 횟수에 포함한다.
- **guard와 공통 꼬리**: 종료하는 true-arm의 불필요한 else를 제거한다. 기존
  else의 지역변수 범위가 필요하면 독립 block을 유지한다. 공통 꼬리 추출은
  명시적으로 확인된 hoisted primitive 대입으로 제한한다.
- **조건부 return**: 같은 정확한 primitive 타입의 두 return을 conditional
  expression으로 복원한다. ternary 깊이는 16, 구성 표현식 길이는 4,096자를
  제한한다. byte/short/char와 서로 다른 타입의 숫자 분기는 자동 병합하지 않는다.
- **루프 복원**: 첫 guard만 루프 조건으로 강화한다. header의 effect나 exit의
  edge copies가 있으면 해당 연산은 원래 위치에 남는다. 직접적인 마지막 continue와
  불필요한 label을 제거하되, 중첩 루프에서 바깥 루프를 가리키는 transfer는 유지한다.
  같은 hoisted induction 변수의 초기화/마지막 갱신을 for-header로 옮길 때는 그
  루프로 향하는 중간 continue가 없는지 확인한다.
- **Phi copy 축약**: 단일 copy는 RHS를 한 번 평가하는 직접 대입으로 출력한다.
  여러 copy의 swap/fan-out/cycle은 전체 snapshot 후 대입하는 기존 순서를 유지한다.
- **dispatch region 복원** (`dispatch.rs`): 유일한 unconditional successor 체인을
  병합하고, 배타적인 직선 branch-arm과 공통 join/terminal, 증명된 single-entry
  직선 natural loop를 if/loop로 출력한다. 원래 BFS emitter callback 순서와
  각 block/edge의 scope 및 copies를 유지한다. 조건은 source scope 안에서 한 번
  평가하고 필요하면 Boolean에 저장한다.

이 변경은 논문의 기법을 참고한 제한적인 재구성이다. DREAM/SAILR 전체 알고리즘을
구현한 것은 아니다. 복잡한 nested/branching cyclic region, irreducible CFG,
exception region은 기존의 검증된 구조 또는 정확한 dispatcher fallback을 사용한다.
SSA 표현식 planner의 whole-CFG acyclic gate도 유지한다. cyclic loop의 Boolean
header 정리는 별도의 typed AST 변환으로 수행한다. static/exception 특수 emitter의
field-initialization 및 exception-scope 규칙은 해당 emitter에서 계속 검증한다.

## 검증과 측정

Java 17 실행 oracle로 입력 경계, overflow, 나눗셈 예외, 호출 횟수/순서,
short-circuit 조건의 예외 trace, else-local scope, loop polarity, 중첩 transfer,
Phi swap 및 branch+Phi 다중 사용을 검증한다. 원본 앱 전체 실행의 동일성을
증명하는 것은 아니므로 source coverage와 출력 구조 지표를 별도로 기록한다.

```bash
JAVA_HOME=/path/to/jdk17 PATH=/path/to/jdk17/bin:$PATH \
  cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings

python3 bench/source_readability.py \
  --before /path/to/before/java --after /path/to/after/java \
  --before-log /path/to/before.log --after-log /path/to/after.log \
  --test-log /path/to/workspace-tests.log --out /path/to/readability.json
```

측정은 원본 source bytes와 comment/blank-line 제거 후 bytes를 모두 기록한다.
generated-local 이름의 휴리스틱 개수, Phi snapshots, dispatch cases/PC 대입,
if/while/for, logical/conditional operator 개수도 기록한다. 이들은 syntax 형태의
관측이며 token 수나 의미 정확도 점수가 아니다. Java Unicode preprocessing 때문에
lexer가 보수적으로 제외한 파일은 따로 나열한다. 원본 method identity와
publication flags 및 export report의 coverage counters를 함께 비교한다.

## Absher 실제 출력 비교

동일한 `/home/jtjisgod/absher/base.apk` (SHA-256
`172575d755606326dd5fd1d8c21dff7d3f5390ee782a0584746195952ac936e4`),
`Lcom/elm/android/individual/` source scope, workers 30, Android35 metadata를 사용했다.

| 관측 지표 | 이전 | 적용 후 |
|---|---:|---:|
| Java files | 5,604 | 5,604 |
| 원본 source bytes | 151,900,839 | 140,748,823 |
| comment/blank-line 제거 후 bytes | 102,677,763 | 92,388,317 |
| generated-local 선언 휴리스틱 | 811,009 | 741,804 |
| Phi snapshot 선언 | 51,644 | 39,179 |
| dispatch cases | 357,825 | 236,137 |
| PC 대입 | 408,454 | 286,669 |
| syntax nonblank lines | 3,687,217 | 3,295,198 |
| for nodes | 0 | 7 |

순수 구조 재작성에 따른 compact bytes 감소는 **10.02%**, dispatch case 감소는
**34.01%**다. 원본 method identity와 publication flags 변경은 0건이고, 모든
export report scalar coverage counters가 같았다: complete ordinary methods 10,697,
constructors 1,911, static initializers 3,429, unsupported methods 19,556.
Unicode preprocessing이 필요한 같은 3개 파일은 syntax-count에서 제외되었으며,
byte 합계에는 보수적으로 원본 bytes를 포함했다.

기존 export는 50.83초/RSS 12,642,756KiB, 최종 통합 release export는
58.26초/RSS 12,633,788KiB였다. 서로 다른 시점의 단일 관측으로, 속도 향상이나
paired regression 측정으로 해석하지 않는다. 이번 변경의 확인된 개선은 출력 구조다.

최종 검증: workspace **1,296 passed / 175 ignored / 0 failed**, workspace/all-target
Clippy `-D warnings`, formatting, `git diff --check`, release build 통과.
별도 LLM-pack Python tests 8개도 통과했다.

로컬 산출물:

- Java: `/tmp/opencode/absher-readable-verified/codepuri/`
- 원본 identity/coverage/크기 영수증:
  `/tmp/opencode/absher-readable-verified/readability.json`
- 최종 LLM context:
  `/tmp/opencode/absher-llm-readable-final/CONTEXT.md` (50,297 characters,
  80,000-character budget; model-specific token count 미측정)
- retrieval index: 같은 LLM pack의 `methods.jsonl` (39,322 original definitions)
- byte-exact 원본 DEX container: 같은 pack의 `runtime-dex.jar` (63,172,685 bytes;
  JVM class JAR가 아님)

## 참고 자료

Subsequent checked-contract and inherited-final recovery results are recorded in
[checked-contract-recovery.md](checked-contract-recovery.md), including 422
additional app-scope complete ordinary methods and recovered Main/Login lifecycle
bodies with their remaining dependency diagnostics.

- Naeem/Hendren, **Programmer-friendly Decompiled Java**, ICPC 2006:
  <https://www.sable.mcgill.ca/publications/papers/2006-3/nanaeem-icpc06.pdf>
  — AST 조건/루프 재작성, reaching definitions/copies, 반복 단순화.
- JADX 1.5.1 **CodeShrinkVisitor**:
  <https://github.com/skylot/jadx/blob/v1.5.1/jadx-core/src/main/java/jadx/core/dex/visitors/shrink/CodeShrinkVisitor.java>
  — SSA single-use 인라인과 연산 이동의 경계.
- Yakdan et al., **No More Gotos**, NDSS 2015:
  <https://www.ndss-symposium.org/wp-content/uploads/2017/09/11_4_2.pdf>
  — reaching-condition 구조화와 조건 재평가 방지.
- Basque et al., **SAILR**, USENIX Security 2024:
  <https://www.usenix.org/conference/usenixsecurity24/presentation/basque>
  — 컴파일러 변환을 고려한 구조 복원. GCC/C 결과를 Java/DEX 증거로 간주하지 않는다.
- **dewolf**:
  <https://github.com/fkie-cad/dewolf>
  — DREAM 계열 구조화와 표현식 복잡도 제어.
