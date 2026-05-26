# Phase 3 — Self Review

| | |
|---|---|
| **Phase** | 3 — relationships.yaml + 자동 dispatch |
| **Status** | ✅ 4-게이트 통과 |
| **Date** | 2026-05-26 |
| **Reviewer** | Luida 본인 + backend-engineer 1 sub-agent |

---

## ① 변경 요약

### 새 패키지: `@luida/brain`
- `src/rules.ts` — `evaluatePostQuest`, `applyFollowUps`, `syncRelationshipsFromYaml`
- `src/rules.test.ts` — 10건
- `src/index.ts` — public surface

### Core 확장
- `migrations/0002_quest_source_inmail.sql` — `quests.source_inmail_id` 컬럼 + partial UNIQUE 인덱스
- `src/schema.ts` — Quest.source_inmail_id 필드
- `src/repo/quest.ts` — `insertIdempotent`, `findBySource` 메서드
- `src/repo/relationship.ts` — `upsertByName`, `findByName` 메서드
- `src/relationships.ts` — YAML 파서(Bun.YAML 우선, fallback 자체 파서 + flow style guard) + glob matcher (`**`, `*`, `?`)
- `src/relationships.test.ts` — 13건

### Sidecar 확장
- `src/dispatch.ts` — `insertIdempotent` 사용, `source_inmail_id` 설정, PR 후 brain 평가 호출 (auto_dispatch는 `status==='completed'`에서만, proposal은 needs_approval에서도 OK)
- `src/git.ts` — `changedFiles({cwd})` (`git diff --name-only base...HEAD`)
- `src/run.ts` — onMessage에서 repos + getChangedFiles 명시 전달 (Phase 3 실제 활성화)

### 예시·문서
- `docs/examples/relationships.yaml` — 자동화 룰 작성 가이드 + agora→admin schema 예시

### 테스트 총 135건 (Phase 2 116 → +19)
- relationships parser·matcher 13건
- brain rules 10건 (path_changed, quest_completed, propose, disabled, dedupe)
- core/db 마이그레이션 테스트 0002 반영

---

## ② 설계 의사결정

### dispatch 멱등성 = `source_inmail_id`
- 같은 inmail 두 sidecar 또는 같은 sidecar의 두 polling tick에서 처리돼도 quest는 1건만
- `insertIdempotent`가 race-safe: findBySource → insert → catch on UNIQUE → re-findBySource
- partial UNIQUE 인덱스 (`WHERE source_inmail_id IS NOT NULL`) — 비 dispatch 경로(수동 quest)는 영향 없음

### YAML 파서 전략
- 1차: Bun.YAML.parse (Bun 1.3+에서 사용 가능, 표준 YAML 1.2 지원)
- 2차 fallback: 자체 파서 (block 스타일만 지원)
  - inline flow(`{...}`, `[...]`) 감지 시 명시적 throw → silent breakage 방지

### Glob matcher — 자체 구현
- 외부 의존성 0. `**`, `*`, `?` + regex 특수문자 escape
- Phase 4에서 picomatch 도입 검토 (negation, brace expansion 필요할 때)

### 룰 평가 위치 = brain
- Phase 0 결정 유지: sidecar는 quest 완료만 알림, brain이 룰 매칭과 후속 inmail 발행
- brain은 sidecar에서 호출되지만 코드는 brain 패키지에 격리 → Phase 4 headless brain에서 같은 함수 재사용

### dedupe_key 안정성 — rule.name 우선
- `chain:quest-N-rule-<name|id>`. yaml 재싱크로 rel.id가 바뀌어도 name 기반 키는 안정
- name 없는 룰은 `id<N>` fallback

### auto_dispatch 발화 시점 정책
- **PR 만들어진 completed 상태에서만 auto_dispatch 발사**
- needs_approval 상태는 변경이 main에 반영되지 않은 상태라 chain dispatch 안전하지 않음
- proposal은 needs_approval에서도 OK (사용자 판단 보조)

### yaml SOT 보장 — upsertByName
- 같은 name 재싱크 시 update로 SOT 유지
- 이전 sync 결과: `{added, updated, failed}`로 운영자 가시화

---

## ③ 발견 사항 · 이슈

### Critical (sub-agent 2건 → **모두 처리**)

| # | 이슈 | 처리 |
|---|---|---|
| C1 | run.ts에서 handleDispatch에 repos/getChangedFiles 미전달 → brain 평가가 production에서 실제로 발화 안 됨 | **수정** — onMessage에서 repos + getChangedFiles 명시 전달. changedFiles 실패는 console.warn 후 빈 배열로 흡수 |
| C2 | dedupe_key가 rel.id에 묶여 yaml 재싱크 시 중복 dispatch 위험 | **수정** — `rule.name` 우선, 없으면 `id<N>` fallback |

### Major (5건 → **모두 처리**)

| # | 이슈 | 처리 |
|---|---|---|
| M1 | syncRelationshipsFromYaml이 insert-only라 yaml SOT 깨짐 | **수정** — `upsertByName` 도입, 반환값에 added/updated/failed 노출 |
| M2 | yaml fallback이 inline flow 미지원 silent | **수정** — flow 감지 시 명시적 throw + Bun.YAML 안내 |
| M3 | catch에서 에러 분류 미흡 | **부분 수정** — sync 실패 시 console.warn으로 가시화. SqliteError code 분류는 Phase 4에서 추가 (다른 helper와 함께 정착) |
| M4 | needs_approval에서 auto_dispatch 발화 위험 | **수정** — `quest.status === 'completed'`일 때만 auto_dispatch. proposal은 변함 없이 발화 |
| M5 | changedFiles가 base ref 부재 시 throw → silent degrade | **수정** — run.ts에서 try/catch + console.warn + 빈 배열 fallback |

### Minor (4건 → 일부 deferral)

- ⏸ m1 parsePayload free-form fallback — InmailRepo 경유 시 항상 JSON. 외부 직접 insert 케이스 보호용으로 유지
- ⏸ m2 renderTemplate 미존재 변수 — 운영자가 yaml typo 시 placeholder가 그대로 보임. Phase 4 brain 데몬 도입 시 warn 추가 검토
- ⏸ m3 trigger_config: unknown 타입 — Zod 도입 시점(Phase 4)에 좁힘
- ⏸ m4 tag_pushed silent skip — `parseRelationshipsYaml` 입력 시 warn 추가 검토 (Phase 4 brain 데몬)

---

## ④ 잔여 위험

| # | 위험 | 영향 | 완화 |
|---|---|---|---|
| R1 | brain은 sidecar 프로세스 내부에서 호출됨 — Phase 4 headless brain 데몬으로 분리 시 호출 위치 변경 필요 | 정보 | Phase 4 prereq로 등록 |
| R2 | `git fetch origin main`을 호출하지 않으므로 base ref가 오래되면 changedFiles 결과가 좁아질 수 있음 | 중 | Phase 4에서 baseRef 정책 옵션화 + 주기적 fetch 검토 |
| R3 | yaml 룰 비활성화 시 enabled=false로 표시되지만 DB에서 row가 남아 있음 — listEnabled에서 자동 필터됨 | 저 | OK. 향후 yaml에서 룰을 통째로 제거할 때 DB row 정리 정책 필요 (Phase 4) |
| R4 | tag_pushed kind는 선언만, 평가 없음 | 저 | Phase 4 brain 데몬에서 git 후크와 함께 본격화 |
| R5 | Phase 0~2의 R1·R2·R5·M3·M11·M16 등 잔여 위험 누적 | 정보 | 각 phase review 참조 |

---

## ⑤ 다음 Phase 영향

### Phase 4 prereq (Phase 0~3 누적)
1. **`packages/mcp` 신규 패키지** — main pane Claude가 붙는 MCP server
   - tools: `quest.{dispatch,list,get,log}`, `adventurer.{list,status}`, `memory.{recall,record}`
2. **brain headless 데몬** — 현재는 sidecar 내부 호출. brain을 별도 프로세스로 분리해 cron + reflect + proposal 생성
3. **마이그레이션 SQL 패키징 정책** — MCP가 외부 노출 시 필요
4. **payload Zod schema** — 사용자/외부 입력 검증
5. **adaptive polling backoff** — UI/sidecar 모두 적용

### Phase 5 prereq (Phase 3에서 등록)
- yaml 룰 row 정리 정책 (yaml에서 제거된 룰을 DB에서 어떻게 처리할지)
- learned-promoted source 룰의 신뢰도 가시화 (chronicle 연동)

### 즉시 사용 가능한 API surface (Phase 3 추가)
```typescript
import {
  // brain
  evaluatePostQuest, applyFollowUps, syncRelationshipsFromYaml,
} from '@luida/brain';

import {
  // core relationships
  parseRelationshipsYaml, globToRegex, pathMatchesAny, pathsMatchingAny,
} from '@luida/core';

import {
  // sidecar git
  changedFiles,
} from '@luida/sidecar';
```

---

## 4-게이트 통과 증거

| 게이트 | 통과 증거 |
|---|---|
| [1] 검증 실행 | `bun run typecheck` → 0 error / `bun test` → **135 pass · 0 fail · 303 expect** (16 files, 1056ms) |
| [2] 셀프 리뷰 문서 | 본 문서 `docs/reviews/phase-3.md` 작성. 5개 섹션 + 4-게이트 증거 |
| [3] 셀프 리뷰 에이전트 | backend-engineer 1 sub-agent 리뷰 → Critical 2 · Major 5 · Minor 4 식별 → 처리 후 **잔여 Critical 0 · 잔여 Major 0 (전부 수정 또는 Phase 4 deferral 명시)** |
| [4] 완료 선언 | 다음 메시지에 `## ✅ Phase 3 — 4-게이트 모두 통과` 헤더 출력 |
