# 다음 단계 설계: 지역 구조화, use-site 표현식, 원인 중심 복원

상태: **설계 제안**. 이 문서는 다음 구현의 인터페이스와 검증 기준을 정한다.
여기서 제안한 region planner, ExpressionPlan, FailureLedger는 아직 구현하지 않았다.
현재 적용한 기능과 실제 측정은 [출력 재구성](decompiled-readability.md)에 기록되어 있다.

## 1. 코드와 실제 입력에서 확인한 병목

### 구조화 문법이 나뉘어 있다

- `crates/backend-java/src/lib.rs`의 planner 선택은 `has_switch`로 갈린다.
  switch가 있으면 extended planner로 보내며, 일반 control-loop planner는 그 경로에서
  직접 시도하지 않는다.
- `crates/decomp-structure/src/extended.rs`는 cyclic CFG를 거절한다.
- `crates/decomp-structure/src/loop_control.rs`의 outgoing 검증은 switch edge를 거절한다.
- 따라서 **switch와 loop가 함께 있는 reducible CFG에도 통합된 문법이 없다**.
  이 제한의 존재는 확인했지만, Absher의 전체 dispatcher 중 해당하는 비율은 아직
  original CFG별 census가 필요하다. source case 개수만으로 irreducibility를 판정하지 않는다.
- 현재 `dispatch.rs` 재구성은 배타적인 직선 arm과 직선 loop body를 처리한다.
  더 큰 중첩 영역은 method-wide dispatcher에 남는다.

### 표현식 결정과 출력 위치가 분리되어 있지 않다

- `pure_expression.rs`는 exception scope 또는 cyclic CFG가 있으면 전체 메서드의
  expression planning을 중단한다.
- 표현식 계획은 `HashSet<ValueId>`이고, `lib.rs`는 이 집합을 보고 hoisted local을
  생략한다. 이후 `Emitter.values`에 저장한 문자열을 `get`으로 읽는다.
- 이 방식은 출력 순서에 민감하다. 과거 cycle gate를 단순 제거했을 때 loop의
  undefined value가 발생했으며, gate는 복구했다.
- 현재 typed source AST가 Boolean header를 별도로 개선하지만, 일반적인 loop-body
  expression DAG를 복원하는 공통 인터페이스는 아직 없다.

### 미지원 메서드의 큰 원인은 CFG 이외에도 있다

최종 Absher app-scope report에서 관측한 exact reason counts:

| 거절/전파 사유 | 관측 건수 |
|---|---:|
| required complete method body unavailable | 7,914 |
| callee exceptions not declared by caller | 5,299 |
| assignment Int to java.lang.Object | 2,971 |
| assignment Byte to Short | 1,197 |
| initializer exceptions not declared by caller | 824 |

이 수치는 report의 사유별 관측이며, 합산해서 unique unsupported-method 개수로
해석하면 안 된다. 원인 거절과 그로 인한 dependency/publication 거절을 구분해야 한다.

최종 Java에서도 `MainActivity.onCreate`와 `LoginActivity.onCreate`의 직접 거절은
`callee exceptions not declared by caller`다. original analysis bundle 기준 각각
256개와 204개의 invoke가 있다. 더 큰 CFG structurer만으로 이 거절은 해결되지 않는다.

`type_compatibility.rs::narrow_register_widening`은 현재 byte/short/char→int만
허용한다. `Emitter::operand`도 이 경로에서 descriptor 고정을 위해 int cast를
하드코딩한다. Byte→Short를 지원할 때 compatibility predicate 하나만 바꾸면 부족하다.

## 2. 제안하는 공통 분석 계층

```text
immutable original CFG / SSA / metadata snapshot
                  ↓
              MethodFacts
       ┌──────────┼───────────┐
       ↓          ↓           ↓
   RegionPlan ExpressionPlan Contract/FailureLedger
       └──────────┼───────────┘
                  ↓
          typed statements / expressions
                  ↓
       readability rewrite → Java rendering
```

### MethodFacts

한 메서드에서 다음 증거를 공유하고, 필요한 분석은 lazy로 계산한다.

- block/edge/instruction index와 **Phi-edge를 포함한** def-use index.
- SCC, dominance/post-dominance, loop nesting, natural-loop latches/exits.
- normal-flow view와 exception-flow view. original CFG와 provenance는 변경하지 않는다.
- live-in/live-out과 stable typed local slots.
- 각 instruction의 ordered handler signature, cleanup/monitor state.
- 연산의 `TotalPure / MayThrow / HeapRead / Volatile / Write / Invoke /
  Allocate / Monitor` 속성과 실제 입력 slot 의존성.

normal graph를 구조화할 때 exception edges를 무시한 결과를 전체 의미 증거로
사용하지 않는다. exception-scope 경계를 넘는 region에는 별도의 적합성 증명이 필요하다.
cache key에는 immutable IR identity와 resolver/metadata snapshot, source mode 및
선언 타입이 포함되어야 한다. 원래 object-planning 캐시를 재사용하고 중복 보관은 피한다.

## 3. 지역 단위의 통합 구조화

### 문법

```text
Region = Sequence | If | Switch | Loop | Transfer | DispatchRegion | Instruction
Transfer = Continue(loop) | Break(boundary) | Return | Throw | RegionExit(exit_id)
```

Switch와 Loop를 같은 region tree에 넣는다. 기존의 switch/while/general-loop
문법과 edge 검증을 통합하며, 이미 성공한 증거를 폐기하고 재분석하지 않는다.

### 처리 순서

1. semantic instruction/type/availability validation을 수행한다.
2. SCC와 loop tree를 구성한다. reducible loop는 inner-first로 region으로 요약한다.
3. 요약된 그래프에서 join/post-dominance를 이용해 if/switch/sequence를 구성한다.
4. shared tail은 각 arm에 복제하지 않고, 공통 continuation에 한 번 둔다.
5. 여러 exit가 있는 loop는 직접 return/throw/외부 labeled transfer를 먼저 사용한다.
   서로 다른 continuation을 Java 문법으로 직접 표현하기 어려운 경우에만 작은
   region-local `exit_id`와 exit switch를 사용한다.
6. 아직 구조화할 수 없는 **최소 영역**에만 DispatchRegion을 둔다.

### DispatchRegion의 계약

- 원본 entry edges, original blocks, exits와 live-in/live-out slots를 명시한다.
- entry/exit Phi copies는 해당 original edge에서 한 번 수행한다.
- region의 PC는 그 영역 안에서만 사용한다. SCC 밖의 직선 코드와 복원된 if/loop는
  PC switch의 case로 만들지 않는다.
- 초기 버전은 original effectful block을 복제하지 않는다. 모든 원본 normal edge와
  block의 소유/실행 위치를 provenance validator로 확인한다.
- Java definite assignment 때문에 default local initializer가 필요하다면,
  모든 실제 read가 원본 정의 또는 올바른 incoming edge copy에 도달한다는 증거가
  먼저 있어야 한다. default 값으로 누락된 SSA 정의를 대신하지 않는다.

### 도달 조건 기반 구조화는 2차 단계

기본 문법으로 남은 acyclic overlapping region에만 DREAM 계열 접근을 적용한다.

- 조건은 원본 평가 site별 symbolic atom이다. 같은 텍스트의 두 호출도 서로 다른
  atom이며, loop iteration의 평가 횟수를 합치지 않는다.
- effects가 있는 조건은 원래 위치에서 한 번 평가하여 capture한다.
- reaching conditions는 공유 Boolean DAG로 표현하고, path enumeration을 하지 않는다.
- `&&`/`||` 순서와 실행 guard를 증명한 경우만 effectful RHS를 직접 넣는다.
- float 비교의 `!(a < b)`를 `a >= b`로 바꾸는 등 NaN 의미를 바꾸는 변환은 하지 않는다.
- Boolean DAG 크기·rewrite work에 예산을 두고, 초과하면 해당 region만 fallback한다.

이 단계의 목표는 무조건 goto/PC 0개가 아니라, **더 읽기 쉬운 표현을 의미 증거와
비용 예산 안에서 선택하는 것**이다.

## 4. ValueId 집합 대신 use-site ExpressionPlan

```text
UseSite = InstructionOperand(insn, operand)
        | PhiEdge(edge, phi, input)

Placement = Materialized(slot)
          | Inline(expression_dag, use_site, region)
          | Captured(original_definition_site, slot)
```

### 핵심 변경

- `value_slot(ValueId)`는 항상 stable local/parameter binding을 가리킨다.
- `expression_at(UseSite, ValueId)`가 해당 소비 위치의 expression DAG를 렌더링한다.
- 표현식을 얻기 위해 producer가 먼저 문자열 emitter를 통과해야 하는 의존성을 없앤다.
- local hoisting은 계획을 완성한 다음 결정한다. 한 정의를 제거하려면 모든 original
  read가 Inline/Captured 계획으로 정확하게 대체되어야 한다.
- Phi sources도 최초 use census에 포함한다. Phi-edge 인라인은 초기 단계에서
  literal rematerialization 외에는 보수적으로 유지한다.
- compatibility/type evidence와 availability evidence는 별개로 확인한다.

### 루프 안에서의 적용 조건

메서드 전체가 cyclic이라는 이유로 포기하지 않고, producer→consumer의 지역 경로를 검증한다.

1. dominance와 original execution guard를 확인한다.
2. nonliteral producer와 consumer가 같은 loop-nesting/iteration domain에 있어야 한다.
3. 그 사이에 backedge/phi update나 참조하는 mutable slot의 write가 없어야 한다.
4. 초기 지원은 유일한 unconditional 경로의 TotalPure primitive 연산으로 제한한다.
5. loop 밖의 계산을 loop 안에 넣어 반복 횟수가 늘어나는 이동은 초기 단계에서 거절한다.
6. 현재의 깊이/노드 예산에 더해 expression bytes, duplication count, 소비 위치의
   총 비용을 제한한다. limits는 typed DAG에서 계산한다.

### try/catch 안에서의 적용 조건

- 1차: 예외를 던지지 않는 primitive 연산을 동일 scope/iteration의 local chain에서 복원.
- 2차: MayThrow 연산은 producer와 consumer의 execution guard 및 ordered handler
  signature가 같고, 원래 operand 평가 순서·cleanup/monitor 상태가 보존되는 경우에만 이동.
- 단순 dominance만으로는 부족하다. `q=a/b; if(flag) return q;`를
  `if(flag) return a/b;`로 바꾸면 flag=false 경로의 예외가 사라지므로 거절한다.
- volatile/read/write/invoke/allocate는 초기 expression movement의 경계로 남긴다.
  이후 효과 분석을 확장할 때도 원래 평가 site에서 capture하는 방법을 우선한다.

## 5. 미지원 메서드 복원은 원인별로 진행

### 5.1 FailureLedger

동일 original method identity에 다음 정보를 저장한다.

- 최초 거절 stage와 instruction offset/id, expected/actual type 또는 contract.
- 검사한 planner/bridge와 각각의 rejection reason. 마지막 error만 남기지 않는다.
- original invoke reference, receiver-static source target, inferred/declared throws,
  active original handlers, override ceiling.
- root rejection / dependency propagation / initialization-unit publication rejection을 구분.
- dependency rejection은 원인 callee와 root까지의 경로를 기록한다.

method-pool index만으로 연결하지 않고 APK hash, DEX entry, original owner/name/descriptor
identity를 함께 사용한다. annotation/string error 표현 대신 machine-readable enum을 둔다.

### 5.2 Byte→Short: 좁고 검증하기 쉬운 첫 복원 후보

- original value가 실제 Byte domain임을 확인한 후 Short consumer의 lossless widening을 지원한다.
- `operand`, `compatible`, result definition typing 및 contextual projection을 함께 검토한다.
- invoke에서는 `(short)` cast 또는 Short-typed capture로 **original descriptor의 overload**를
  고정한다. 단순 Java implicit widening만 믿으면 Byte overload를 호출할 수 있다.
- Byte→Char, Short→Byte, Int→Short와 임의 long/float 변환까지 같이 허용하지 않는다.
- byte 전체 범위, 배열/field read, Phi/Move, short/byte overload oracle로 확인한다.

### 5.3 Int→Object: 최초 잘못된 타입 부여 위치를 찾는다

대표 root를 다음 provenance로 분리한다.

- 원본 const-zero가 reference use에서 Null이어야 했는지.
- Move/Phi에서 category-one numeric/reference projection이 잘못 결합되었는지.
- field/array/invoke descriptor가 실제 reference 결과를 제공하는지.
- original boxing invoke가 존재하는지.

original const-zero 및 reference compatibility 증거가 있을 때만 reference use를 `null`로
표현한다. 계산 결과가 수학적으로 0이라는 이유로 null로 바꾸지 않는다. 임의
Integer boxing이나 Object cast도 만들지 않는다: identity·할당·OOM 의미가 달라질 수 있다.

### 5.4 Checked exceptions: Main/Login 진입점의 직접 병목

각 실패 invoke를 active original handler와 source contract에 연결한다.

1. exception inference가 raw annotation, derived source contract, metadata ambiguity,
   actual escaping throw를 구분하도록 provenance를 기록한다.
2. 보호된 invoke의 original handler coverage는 site별로 검사한다. 메서드 안에
   같은 catch type이 있다는 사실만으로 보호되었다고 판단하지 않는다.
3. Android lifecycle override에는 임의 checked `throws`를 추가하지 않는다.
4. 실제로 checked Throwable이 빠져나갈 수 있는 DEX 코드라면 기존
   `checked_source_bridge`의 caller-local exact-site recipe를 확장한다.
   같은 Throwable 객체를 같은 outer handler/cleanup 경로로 전달하고 wrapping하지 않는다.
5. pre-super constructor 영역, uninitialized object, monitor/cleanup crossing은
   별도 증명이 없으면 기존 boundary를 유지한다.

Main/Login onCreate의 256/204 invoke 중 어떤 site가 막는지부터 좁힌다.
여기서 직접 root를 해결하면 dependency closure로 withheld된 caller도 다시 검사할 수 있다.
7,914건을 모두 자동 복원할 수 있다는 뜻은 아니다.

## 6. 비용과 성능 설계

- SCC는 O(V+E). dominance/post-dominance는 기존 구현의 실제 비용을 그대로 측정한다.
  전체 구조화를 무조건 O(V+E)라고 주장하지 않는다.
- 여러 planner가 같은 CFG/type/def-use를 다시 준비하는 비용을 MethodFacts로 공유한다.
- type/instruction 불가능과 shape 불가능을 구분한다. 같은 semantic rejection을
  formatter를 바꾸며 반복 검사하지 않는다.
- 대부분의 작은 메서드는 cheap shape/operation gate만 거쳐 현재 fast path를 사용한다.
- reaching-condition/complexity rewriting은 candidate region에만 적용하며 work budget을 기록한다.
- stage wall/worker work를 별도로 기록: validation, SSA/type, contracts, facts,
  region planning, expression planning, rendering, file writes.
- 같은 input/scope/metadata/workers와 보존된 before/after binary로 순서를 교대하여
  paired 측정한다. 50.83초와 58.26초의 서로 다른 시점 단일 관측을 regression 증거로 쓰지 않는다.

## 7. 구현 순서와 완료 조건

| 단계 | 산출물 | 완료 조건 |
|---|---|---|
| A | FailureLedger, CFG census, stage profile | root/전파 구분; switch+loop, nested reducible, irreducible 분류; 재현 fixture 선택 |
| B | Byte→Short projection, checked-call root trace | byte 범위·overload oracle; Main/Login 실패 site와 bridge 거절의 구체적인 원인 확보 |
| C | MethodFacts + typed RegionPlan | 기존 성공 입력의 원본 block/edge coverage 동일; 분석 준비의 중복 제거 |
| D | switch+loop 통합, nested/shared-tail, local DispatchRegion | 복원 가능한 영역의 PC 제거; residual region의 정확한 entry/exit/copy 계약 |
| E | use-site ExpressionPlan + live slots | cycle gate 없이 안전한 local loop-chain 복원; undefined local/Phi read 없음 |
| F | scope-aware exception expressions/bridges, Int→Object provenance 수정 | 예외 객체·handler·cleanup 순서 oracle와 원본 descriptor 검증 |
| G | bounded reaching conditions와 구조 품질 tuning | 예산 초과 지역 fallback; 분기/호출 재평가 없음; paired 비용 측정 |

각 단계에서 Java17 컴파일/실행, IR edge/provenance validation, targeted oracle를 수행한다.
루프 fixture는 switch fallthrough, 복수 latch/exit, nested outer continue, shared terminal,
Phi cycle을 포함한다. expression fixture는 overflow, NaN/signed-zero, zero divisor,
same-text effectful predicates, protected/unprotected invoke, allocation/monitor ordering을 포함한다.
bounded CFG interpreter와 compiled JVM fixture의 결과/예외/관측 가능한 effect trace를 비교한다.
실제 Absher 전체 앱 실행의 동일성을 이 fixture 결과로 대체하지 않는다.

현재 기준선은 compact 92,388,317 bytes, dispatch cases 236,137, generated locals
741,804, complete ordinary methods 10,697이다. 모든 품질 측정은:

- 동일 original method identity의 **matched-method 비교**와 전체 coverage 비교를 분리한다.
- compact bytes, PC transfers, local declarations, nesting과 expression-size tail을 함께 본다.
- 새로 복원된 메서드 때문에 source 총량이 증가하는 것은 가독성 회귀로 단정하지 않는다.
- 메서드를 생략해 얻은 크기 감소는 성공으로 인정하지 않는다.
- regional budget exhaustion과 root rejection은 report에 명시한다.

## 권장 첫 구현

**FailureLedger와 Byte→Short projection을 먼저 만들고, 병행하여 switch+loop의
통합 RegionPlan을 구축한다.** 그 다음 use-site ExpressionPlan으로 cycle gate를
제거한다. Main/Login lifecycle 복원은 checked-call root trace에서 확인한 원인을
별도의 exact-site bridge/contract 작업으로 해결한다.

근거 영수증: `/tmp/opencode/readability-design-inventory.json`와
`/tmp/opencode/absher-readable-verified/readability.json`.
