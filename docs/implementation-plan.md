# Luida — Implementation Plan

> ⚠️ **v1 (TypeScript) 아카이브** — 이 문서는 v1 설계 로드맵입니다. v2(Rust)는 ADR-0001 후
> 전면 재작성됐습니다. **최신 설계 정본: [`docs/v2-standalone.md`](v2-standalone.md)** ·
> 진행 기록: [`docs/reviews/v2-p*.md`](reviews/) · v1 코드: `git tag v1-typescript`.

| | |
|---|---|
| **Status** | v1.0 (아카이브 — v2-standalone.md 로 대체) |
| **Owner** | Roto |
| **Last updated** | 2026-05-26 |
| **Goal command** | `/goal` 활성 — 각 Phase 4-게이트 필수 |

## 시스템 개요 (요약)

Luida는 cmux 위에서 동작하는 멀티 에이전트 오케스트레이션 시스템. 핵심 컴포넌트:

| 컴포넌트 | 역할 |
|---|---|
| `tavern.db` (SQLite) | 모든 상태·메시지의 단일 진실 |
| Sidecar (per cmux pane) | 자기 inmail polling → `cmux send-key`로 주입 |
| Worker (headless `claude -p`) | sidecar가 worktree에 띄운 작업 실행자 |
| Brain (headless 데몬) | 이벤트 기반 의사결정, 학습 reflect |
| MCP server | main pane Claude가 붙는 tools (quest.*, memory.* …) |
| TUI (Ink) | 술집 대시보드 (`luida ui`) |
| Web (Vite, Phase 외) | 별도 트랙. `docs/web-design-spec.md` |

자세한 데이터 모델은 §Phase 0의 스키마 SQL과 동일하게 `packages/core/src/schema.ts`에 TS로 표현.

---

## Phase별 정의

각 Phase는 [DoD] · [검증] · [산출물] 세 섹션을 가진다. 4-게이트(typecheck/test/리뷰doc/리뷰agent + 완료선언)는 모든 Phase 공통이라 여기서는 생략한다.

### Phase 0 — Monorepo 부트스트랩 + tavern.db 스키마

**DoD (Definition of Done)**
- Bun workspace로 `packages/core`, `packages/cli` 생성
- `packages/core/src/schema.ts`에 5개 엔티티 TS 타입 정의 (Adventurer/Quest/Inmail/Event/Relationship)
- `packages/core/migrations/0001_init.sql`에 모든 테이블 + 인덱스 정의
- `packages/core/src/db.ts`: `openDb()`, `migrate()` (bun:sqlite, WAL)
- `packages/cli/src/index.ts`: `luida db init` 명령으로 `~/.luida/tavern.db` 생성·마이그레이션
- `bun run typecheck` 0 error
- `bun test` 통과 (migration·insert·select round-trip)

**검증**
- `bun install` → 의존성 해소
- `bun run typecheck` → 0 error
- `bun test` → 단위 테스트 전체 pass
- `bun run --filter=cli start db init` → DB 파일 생성 확인, 5개 테이블 + indices 존재

**산출물**
- `package.json` (workspaces)
- `tsconfig.json` (base)
- `packages/core/{package.json, tsconfig.json, src/{schema.ts, db.ts, index.ts, db.test.ts}, migrations/0001_init.sql}`
- `packages/cli/{package.json, tsconfig.json, src/index.ts}`
- `.gitignore` (~/.luida/는 별개라 무관, node_modules·dist만)

---

### Phase 1 — agora sidecar + worker spawn 동작

**Prereq (Phase 0 리뷰에서 등록됨, Phase 1 시작 첫 작업)**
1. `tsconfig.base.json` + 패키지별 `tsconfig.json` 분리 (Ink/React 대비)
2. `packages/core/src/repo/{adventurer,quest,inmail,event,relationship}.ts` repository 계층 도입 — 각 repo는 자체 prepared statement 캐시 보유
3. `packages/core/src/integrations/`에 `CmuxBridge` / `Worktree` / `WorkerRunner` / `VcsHost` 인터페이스 + mock — 실제 구현은 sidecar에서, 인터페이스는 core에서 export
4. CLI에 가벼운 라우터 도입 (~50줄 자체 작성 또는 commander)

**DoD**
- `packages/sidecar` 생성, entry: `luida sidecar --me <name>`
- 시작 시 `$CMUX_WORKSPACE_ID`/`$CMUX_SURFACE_ID` 읽어 `adventurers` 테이블에 upsert
- 10초 polling 루프: `inmail WHERE to_session=me AND delivered_at IS NULL`
- 수신 시 `cmux send-key`로 자기 surface에 prompt 주입 + `delivered_at` 마킹
- `dispatch` kind 수신 시:
  - `wt c "<branch>"`로 worktree 생성 (worktrunk 표준 준수)
  - `claude -p --cwd <worktree> --output-format stream-json "<brief>"` headless worker spawn
  - stdout stream-json 파싱 → `quests.progress` 갱신 + `events` row 적재
  - worker 종료 후 `git status` 확인, `gh pr create` (mock 모드 토글 제공)
  - `inmail` (kind=ack, to=requester) + `quests.status='completed'`
- Stop hook 스크립트 (`packages/sidecar/src/stop-hook.ts`): 옵션, Phase 1에선 stub

**검증**
- 단위 테스트: polling, inject 로직 (cmux send-key는 mock)
- 통합 테스트: 인메모리 DB로 dispatch→worker(faked)→ack 흐름
- 수동 검증 (선택): 실제 cmux pane에서 dummy inmail 1건 처리

**산출물**
- `packages/sidecar/{package.json, src/{index.ts, poll.ts, inject.ts, worker.ts, stop-hook.ts}, tests/}`
- `packages/cli`에 `luida sidecar` 서브명령 추가
- README에 sidecar 띄우는 법 명시

---

### Phase 2 — TUI 대시보드 (Ink)

**Prereq (Phase 0 리뷰에서 등록됨)**
- 패키지별 tsconfig 분리가 이미 Phase 1에서 완료된 상태 가정. `packages/ui`는 `jsx: "react-jsx"`, `lib`에 DOM 제외, React 18 typings는 ui 패키지 한정.

**DoD**
- `packages/ui` 생성. Ink + React 18 + TypeScript
- 컴포넌트: `<Window>`, `<HpBar>`, `<AdventurerCard>`, `<QuestRow>`, `<EventLogLine>`
- 화면 4분할: 모험가/의뢰서/술집게시판/연감(stub)
- tavern.db에서 1초 간격 polling으로 라이브 갱신 (Phase 4에서 SSE로 대체 예정)
- 키바인딩: `q` 종료, `j/k` 항목 이동, `Tab` 패널 전환
- `luida ui`로 기동

**검증**
- `bun test` (스냅샷 또는 ink-testing-library)
- `bun run typecheck`
- 수동: dummy seed로 화면 확인 (스크린샷 1장 review 문서에)

**산출물**
- `packages/ui/{package.json, src/{App.tsx, panels/*.tsx, components/*.tsx}, tests/}`
- `luida ui` CLI 서브명령

---

### Phase 3 — relationships.yaml + 자동 dispatch

**결정 (Phase 0 리뷰에서 확정)**
- **룰 평가기 위치 = brain**. sidecar는 quest 완료 시 inmail만 발행. brain이 룰을 평가해 자동 dispatch 또는 proposal 발급.
- 따라서 Phase 3 시작 시 `packages/brain`을 먼저 생성하고, Phase 4에서 의사결정·MCP를 그 위에 쌓는다.

**DoD**
- `~/.luida/relationships.yaml` 스키마 정의 + 파서
- `packages/brain`에 룰 평가기
- Stop hook이 worker 종료 후:
  1. 변경 파일·status 수집
  2. enabled relationships 중 trigger 매칭
  3. action=`auto_dispatch`면 즉시 `quests` + `inmail` row 작성 (parent_quest_id 연결)
  4. action=`propose`면 사용자에게 inmail kind=`proposal` 전송
- 기본 룰 1개 seed: `agora-schema-to-admin`

**검증**
- 룰 평가 단위 테스트 (다양한 trigger_config로)
- 통합 테스트: agora quest 완료 → admin quest 자동 생성 확인

**산출물**
- `packages/core/src/relationships.ts` (룰 schema·파서)
- `packages/brain/src/rules.ts` (평가기)
- `packages/sidecar/src/post-quest.ts` (quest 완료 inmail 발행만)
- `~/.luida/relationships.yaml.example`
- 통합 테스트

---

### Phase 4 — Luida brain 의사결정 (제안 생성) + MCP server

**Prereq (Phase 0 리뷰에서 등록됨)**
- 마이그레이션 SQL 패키징 정책 결정 (text import vs `files` 필드) — MCP server를 외부에 배포 형태로 노출하기 시작하므로
- payload JSON Zod schema validation 도입 검토

**DoD**
- `packages/brain` 헤드리스 데몬 생성. entry: `luida brain start`
- 3종 트리거: 이벤트 기반, cron, 명시 호출
- 이벤트 기반: quest 종료 시 chronicle/패턴 참조해서 "제안" inmail 생성
- `packages/mcp` MCP server 생성. tools:
  - `quest.dispatch`, `quest.list`, `quest.get`, `quest.log`
  - `adventurer.list`, `adventurer.status`
  - `memory.recall`, `memory.record`
- main pane Claude가 MCP로 붙어 사용자 대화 중 즉시 호출

**검증**
- brain 단위 테스트: 제안 생성 로직
- MCP server 통합 테스트: tools 호출 → 올바른 DB 변경
- 수동: main pane에서 `quest.list` 호출 → 표 반환 확인

**산출물**
- `packages/brain/{...}`
- `packages/mcp/{...}`
- MCP 등록 가이드 (README)

---

### Phase 5 — 학습 루프 (reflect + 패턴 승격)

**DoD**
- brain에 reflect job (cron 또는 quest N개마다)
- 최근 N일 events를 분석 → 패턴 후보를 `~/.luida/memory/patterns/YYYY-MM-DD-<topic>.md`로 출력
- 사용자에게 push inmail (kind=`proposal`): "패턴 후보 N건, 검토하세요"
- 사용자 승인 시 `relationships` row 자동 INSERT (source=`learned-promoted`, confidence 점수)
- chronicle.md에 학습 활동 누적 append

**검증**
- 패턴 분석 단위 테스트 (synthetic events로)
- end-to-end: events 시뮬레이션 → 패턴 markdown 생성 → 승급 → relationships 활성화

**산출물**
- `packages/brain/src/reflect.ts`
- `packages/brain/src/promote.ts`
- chronicle 템플릿

---

## 디렉터리 최종 형태 (Phase 5 종료 시점 기준)

```
roto-ai-agent/                        ← 디렉터리는 그대로 유지, 패키지명만 @luida/*
├── docs/
│   ├── implementation-plan.md        ← 본 문서
│   ├── web-design-spec.md
│   └── reviews/
│       ├── phase-0.md
│       ├── phase-1.md
│       ├── phase-2.md
│       ├── phase-3.md
│       ├── phase-4.md
│       └── phase-5.md
├── packages/
│   ├── core/        # 스키마, 타입, DB
│   ├── cli/         # luida 단일 진입점
│   ├── sidecar/     # cmux pane별 데몬
│   ├── ui/          # Ink TUI
│   ├── brain/       # 헤드리스 의사결정·학습
│   └── mcp/         # main pane Claude용 MCP server
├── package.json
├── tsconfig.json
├── bun.lockb
└── .gitignore
```

> ⚠️ 디렉터리 rename(`roto-ai-agent` → `luida`)은 작업 중 CWD 무효화 위험이 있어 Phase 작업이 모두 끝난 뒤 사용자가 수동으로 수행하거나 최종 Phase 5의 마지막 단계에서 처리한다.

---

## 공통 제약 (모든 Phase)

- **Worktree**: `wt c "<name>"` 표준. raw `git worktree` 금지.
- **경로 분리**: 코드·문서는 프로젝트, 런타임 데이터는 `~/.luida/`.
- **시크릿**: `.env` 등 생성·커밋 금지.
- **메모리 준수**: `~/.claude/projects/-Users-roto-workspace-roto-ai-agent/memory/`의 모든 feedback 메모리.
- **언어**: 사용자 대화·문서·UI는 한국어 우선. 코드 식별자는 영문.
