# decomphify 기술 문서

- [codepuri 마스터 플랜](codepuri-plan.md): 포크 목표, 기준선(jadx 26.26s 대결),
  Phase 0~4 (레드 베이스라인 정상화 → 속도 → 가독성 → 패리티 → 멀티 타겟).
- [기능별 시간·공간 복잡도](algorithm-complexity.md): 현재 구현의 알고리즘 비용,
  캐시와 병렬 실행의 한계, 남은 비선형 처리 및 개선 우선순위.
- [원본 메서드 비교 데이터셋](../bench/METHOD_COMPARISON.md): 17개 APK의 입력 범위,
  identity 매칭, 도구별 관측 결과와 중복 코드 후보를 조회하는 방법.
- [RDX 리뷰와 실제 앱 증거 대조](rdx-dataset-followup.md): dominance 분석 재사용,
  초기화·enum 복원, 선택적 클래스 조회의 개선 우선순위.
- [반복 질의 인덱스·캐시 개선](../bench/QUERY_CACHE_OPTIMIZATION.md): 공통 dominance,
  메서드별 검색·분석 재사용, 성공 경로 질의의 복잡도와 전후 측정.
- [앱 중심 export와 LLM context](../bench/FOCUSED_LLM_EXPORT.md): 라이브러리 source
  출력 범위, 의존성 중심 계약 분석, 주석 정리·원본 DEX 보존·페이지 단위 조회.
- [의미를 보존하는 출력 재구성](decompiled-readability.md): SSA 표현식, 조건·루프·guard,
  Phi copy 및 dispatch region 복원과 Java 실행 검증·원본 앱 전후 측정.
- [다음 복원 단계 설계](readability-recovery-design.md): switch+loop 통합·지역 fallback,
  use-site 표현식 배치, 타입·checked-exception·의존성 거절의 원인 중심 복원 계획.
- [Absher 라이브러리 탐지 실험](../bench/LIBRARY_IDENTIFICATION.md): LibScout 실제 실행,
  전체 DEX 로딩 확인, 라이브러리 후보와 작은 시그니처 프로필의 오탐 검증.
- [점수 인자로 제어하는 라이브러리 분리](../bench/LIBRARY_SEPARATION.md): 최신 artifact,
  클래스 구현 지문·원본 hash 검증, `--library-min-score`와 실제 분리 결과.
