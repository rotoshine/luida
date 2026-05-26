# Phase 4 — Self Review

| | |
|---|---|
| **Phase** | 4 — Luida brain 의사결정 + MCP server |
| **Status** | ✅ 4-게이트 통과 |
| **Date** | 2026-05-26 |
| **Reviewer** | Luida 본인 + backend-engineer 1 sub-agent |

---

## ① 변경 요약

### `@luida/brain` 확장
- `src/memory.ts` — `MemoryStore` (chronicle/projects/patterns markdown 관리, sanitize, pattern auto-suffix)
- `src/memory.test.ts` — 8건
- `src/daemon.ts` — `runBrain` (interval tick, stuck quest 감지 idempotent, in-flight cleanup)
- `src/daemon.test.ts` — 3건
- `src/index.ts` — re-export

### 신규 패키지: `@luida/mcp`
- `src/tools.ts` — 6개 tool 순수 함수
  - `quest.list` / `quest.get` / `quest.dispatch`
  - `adventurer.list`
  - `memory.recall` / `memory.record`
  - 각 tool은 inputSchema 선언 + runtime type guard
- `src/server.ts` — JSON-RPC stdio MCP (initialize / ping / tools/list / tools/call)
  - **stdin chunk 직렬화** (processing promise chain)으로 race 차단
  - SIGINT 핸들러는 라이브러리 안에서 등록하지 않음 (호출자 책임)
- `src/{tools,server}.test.ts` — 22건

### CLI 확장
- `luida brain start [--interval MS] [--once]` — headless brain 데몬
- `luida sync-rules <yaml-file>` — relationships.yaml DB 동기화
- `luida mcp start [--me NAME]` — MCP server 시작, handle.close() SIGINT 처리

### 테스트 총 166건 (Phase 3 135 → +31)

---

## ② 설계 의사결정

### MCP — 공식 SDK 미사용, 최소 JSON-RPC 자체 구현
- 4개 핵심 메서드(initialize, ping, tools/list, tools/call)만 직접 구현 (~180줄)
- 외부 의존성 0 — Bun 직접 stdio + JSON.parse
- 향후 더 많은 MCP 기능(resources, prompts)이 필요해지면 그때 `@modelcontextprotocol/sdk` 도입 검토

### Tools = 순수 함수
- `ToolDef<I, O>` 인터페이스에 name·description·inputSchema·handler
- handler는 ctx 주입 받고 sync/async 둘 다 지원
- 단위 테스트는 handler를 직접 호출 — MCP 프로토콜 우회

### Brain daemon stuck 감지 — idempotent
- 매 tick에서 cooldown 내에 이미 review_failed event를 기록한 quest는 skip
- 60초 tick × 1시간 stuck threshold = 같은 stuck quest당 최대 1건/시간 event
- `events.recentSince(now - stuck)`로 조회 → in-memory Set으로 dedup

### MemoryStore sanitize + auto-suffix
- 파일명에 path traversal 차단 (`[^A-Za-z0-9_\-가-힣]` 외 모두 `_`)
- pattern 같은 name이 존재하면 timestamp suffix 부여 (`name-NNNNNN`)

### CLI cleanup 일관성
- 모든 long-running 명령은 `handle.close()` 또는 `loop.stop()` SIGINT 핸들러
- `process.once + process.off`로 listener 누적 방지
- MCP 디버그 출력은 stderr (stdout = JSON-RPC 채널)

### Phase 5 prerequisite — reflect/promote stub
- Brain daemon은 stuck 감지까지만. reflect/패턴 분석/promote는 Phase 5에서 daemon에 cron 추가

---

## ③ 발견 사항 · 이슈

### Critical (sub-agent 2건 → **모두 처리**)

| # | 이슈 | 처리 |
|---|---|---|
| C1 | stdin data handler buffer race — 동시 chunk 도착 시 라인 중복/누락 가능 | **수정** — processing promise chain으로 직렬화. 라인 추출은 sync, 처리는 sequential await |
| C2 | `luida mcp start`가 handle.close() 미호출 | **수정** — handle 캡처, SIGINT에서 close() 호출 + listener off |

### Major (6건 → **5건 수정 / 1건 deferral**)

| # | 이슈 | 처리 |
|---|---|---|
| M1 | brain stuck quest event 중복 기록 | **수정** — recentSince + Set 기반 idempotent 가드 + TickResult.recorded 노출 |
| M2 | MCP tool input validation 부재 | **수정** — questDispatch / questGet / memoryRecord에 runtime type guard 추가 |
| M3 | quest.list `status: 'all'`이 active만 반환 | **부분 수정** — description은 active 명시, 본격적 listAll 메서드는 Phase 5 deferral |
| M4 | pattern memory 같은 name 덮어쓰기 | **수정** — `writePattern`이 존재 시 timestamp suffix 자동 부여 + 반환값에 final name |
| M5 | brain SIGINT 시 in-flight tick race | **수정** — stop()에서 inFlight promise 추적, finally cleanup |
| M6 | chronicle 무한 증가 + 매 호출 전체 read | **deferral** — recall에 tail limit은 이미 있음. 진정한 rotation은 Phase 5 reflect 도입 시 함께 작업 (chronicle.YYYY-MM.md) |

### Minor (5건 → 일부 수정 / 일부 deferral)
- ✅ JSON.stringify try/catch (m4)
- ⏸ m1 SIGINT 라이브러리 등록 — 이미 server.ts는 등록 안 함 (수정 완료)
- ⏸ m2 ctor default 함수화 — Phase 5 정착 시 LuidaConfig와 함께
- ⏸ m3 sanitize zero-width — 한국어 환경에서 영향 매우 적음
- ⏸ m5 sync-rules UX 메시지 — Phase 5 CLI 다듬을 때

---

## ④ 잔여 위험

| # | 위험 | 영향 | 완화 |
|---|---|---|---|
| R1 | brain의 reflect job이 아직 stub — 학습 패턴 자동 생성 안 됨 | 정보 | Phase 5에서 본격화 |
| R2 | chronicle rotation 미구현 — 1년 단위 운영 시 수십 MB markdown 가능 | 중 | Phase 5 reflect 도입 시 함께 작업. 임시는 recall limit 활용 |
| R3 | MCP server는 stdio만 지원 — 향후 SSE/HTTP 필요 시 `@modelcontextprotocol/sdk` 도입 | 정보 | Phase 4 외 |
| R4 | tools input validation은 핵심만 — quest.list params 등 일부 처리 안 됨 | 저 | Phase 5에서 Zod 도입 (다른 payload 검증과 함께) |
| R5 | brain daemon이 별도 cmux pane에 어떻게 띄울지 정책 미정 | 저 | 사용자가 `luida brain start &`로 띄우거나 launchd plist. Phase 5에서 startup 가이드 정리 |

---

## ⑤ 다음 Phase 영향

### Phase 5 prereq (Phase 1·2·3·4 누적)
1. brain daemon에 reflect job 추가 — recentSince(N일) → pattern 후보 markdown
2. **패턴 → 사용자 승인 → relationships 승격** 워크플로우
3. chronicle rotation (M6)
4. Zod schema validation (payload + MCP input)
5. quest.list `listAll` 메서드 (M3)
6. CLI startup 가이드 (`launchd` plist 예시, README)
7. Phase 2 deferred: `useStdout` 기반 단일 컬럼 fallback (선택)

### 즉시 사용 가능한 API surface (Phase 4 추가)
```typescript
import {
  MemoryStore, runBrain, getMemoryDir,
} from '@luida/brain';

import {
  ALL_TOOLS, handleMessage, runMcpServer,
  // 개별 tool
  questList, questGet, questDispatch,
  adventurerList, memoryRecall, memoryRecord,
} from '@luida/mcp';
```

### CLI 명령 누적
```
luida db init
luida sidecar --me <name> [...]
luida ui [--interval MS]
luida brain start [--interval MS] [--once]
luida sync-rules <yaml-file>
luida mcp start [--me NAME]
```

---

## 4-게이트 통과 증거

| 게이트 | 통과 증거 |
|---|---|
| [1] 검증 실행 | `bun run typecheck` → 0 error (1 syntax error fix 후 통과) / `bun test` → **166 pass · 0 fail · 353 expect** (20 files, 1185ms) |
| [2] 셀프 리뷰 문서 | 본 문서 `docs/reviews/phase-4.md` |
| [3] 셀프 리뷰 에이전트 | backend-engineer 1 sub-agent → Critical 2 · Major 6 · Minor 5 → **잔여 Critical 0 · 잔여 Major는 모두 수정 또는 Phase 5 deferral 명시** |
| [4] 완료 선언 | 다음 메시지에 `## ✅ Phase 4 — 4-게이트 모두 통과` 헤더 |
