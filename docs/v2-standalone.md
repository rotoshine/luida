# Luida v2 — Standalone Architecture (Design)

| | |
|---|---|
| **Status** | Draft v0.1 (설계 — 미구현) |
| **Owner** | Roto |
| **Last updated** | 2026-05-28 |
| **선행** | Phase 0~5 + Web Track A/B + 운영(A~E) 완료 |
| **목표** | cmux 의존 제거 → self-contained 오케스트레이터. 다중 프로젝트 자동 계획(원정) + 행위별 멀티 런타임/모델 |

---

## 1. 배경 · 목표

v1은 **cmux pane에 붙는 sidecar** 모델이었다. 사용자가 pane을 직접 띄우고, sidecar가 거기 붙어 `cmux send-key`로 prompt를 주입했다.

v2는 **cmux 의존을 완전히 제거**하고 Luida가 스스로:
1. 모험지(프로젝트)를 등록받고
2. 사용자 프롬프트 1개를 다중 프로젝트 **원정(Campaign)** 계획으로 분해하고
3. worktree를 만들어 worker를 돌리고
4. 진행을 모니터링하고, 문제 시 비방해적으로 사용자에게 묻고
5. 끝나면 보고서를 **모험의 서**에 남긴다.

### 비목표 (명시적으로 안 함)
- **Ghostty 직접 제어 ❌** — cmux가 `libghostty` 위에 쌓은 surface/socket 레이어를 재구현하는 것이라 거대한 우회로. 대신 **PTY를 직접 소유**(node-pty)하고 우리 UI(Ink/xterm.js)에 렌더한다.
- v1 호환성 유지 ❌ — adventurer(=cmux pane) 개념은 폐기.

---

## 2. 아키텍처 개요

```
┌─ Luida 앱 (Tauri 데스크탑 / TUI) ──────────────────────────┐
│                                                            │
│  모험지 등록 메뉴   원정 입력   진행 모니터   모험의 서          │
│        │              │            │            │            │
│        ▼              ▼            ▼            ▼            │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Luida Core (Bun) — tavern.db + agents.json          │  │
│  │   projects · campaigns · quests · events · memory    │  │
│  └───────────────┬──────────────────────────────────────┘  │
│                  │ Agent Resolver (행위 → 런타임/모델)        │
│        ┌─────────┼──────────┬───────────┬─────────────┐     │
│        ▼         ▼          ▼           ▼             ▼     │
│   Planner    Worker     Reviewer   Escalator     Reporter  │
│   (opus)     (sonnet)   (opus)     (opus)        (sonnet)  │
│        └─────────┴──────────┴───────────┴─────────────┘     │
│                  │ AgentRuntime adapter                      │
│        ┌─────────┴──────────┬───────────┐                   │
│        ▼                    ▼           ▼                   │
│   ClaudeRuntime       CodexRuntime  DeepseekRuntime         │
│   (claude -p)         (codex)       (API/CLI)               │
│                  │                                           │
│                  ▼ 각 worker는 worktrunk worktree에서 PTY로 실행 │
│   ~/workspace/<project>/.worktrees/<branch>                 │
└────────────────────────────────────────────────────────────┘
```

핵심: **터미널 에뮬레이터를 제어하지 않는다.** worker는 (a) headless(`claude -p`)거나 (b) 우리가 소유한 PTY. 라이브 보기는 PTY 출력을 Ink/xterm.js에 렌더.

---

## 3. 에이전트 행위 분류 (Agent Action Taxonomy) ★

v2의 모든 LLM 호출 지점을 **행위(action)**로 분류한다. 각 행위는 복잡도 tier를 갖고, tier가 기본 모델을 결정한다. **행위별로 런타임·모델을 `agents.json`에서 override 가능.**

### 3.1 복잡도 tier
| tier | 정의 | 기본 모델 |
|---|---|---|
| `complex` | 판단·계획·교차맥락·위험평가가 필요 | `claude-opus-4-7` |
| `simple` | 범위가 정해진 작업 이행·요약 | `claude-sonnet-4-6` |

### 3.2 행위 목록

| action id | 설명 | tier | 기본 |
|---|---|---|---|
| `campaign.plan` | 사용자 프롬프트 → 다중 프로젝트 원정 DAG 계획. 어느 모험지에서 무엇을, 어떤 핸드오프로 | **complex** | claude · opus-4-7 |
| `quest.execute` | 계획된 brief대로 한 worktree에서 실제 코딩 수행 | simple | claude · sonnet-4-6 |
| `quest.review` | quest 변경을 PR 전 검토 (위험·설계 적합성 판단) | **complex** | claude · opus-4-7 |
| `escalation.triage` | 모험 중 문제를 분류(system_error/design_mismatch/dangerous_op/ambiguous_spec)하고 사용자에게 물을지 결정 | **complex** | claude · opus-4-7 |
| `learning.reflect` | events 분석 → 패턴 발굴 → 관계 제안 | **complex** | claude · opus-4-7 |
| `merge.resolve` | 교차 프로젝트 변경 충돌 해소 판단 | **complex** | claude · opus-4-7 |
| `campaign.report` | 완료 원정 요약 → 모험의 서 | simple | claude · sonnet-4-6 |
| `project.ingest` | 등록된 모험지의 README/schema/구조 읽어 맥락 요약 → memory/projects/ | simple | claude · sonnet-4-6 |
| `pr.describe` | PR title/body 생성 | simple | claude · sonnet-4-6 |
| `inmail.summarize` | (선택) inmail payload를 사람 읽기 좋게 변환 | simple | claude · sonnet-4-6 |

> 추가 행위는 같은 표에 한 줄 추가 + `agents.json`에 매핑 추가만 하면 됨.

### 3.3 분류 원칙
- "이 결정이 틀리면 비싸다 / 교차 맥락이 필요하다" → **complex** (opus)
- "이미 좁혀진 범위를 이행하거나 요약한다" → **simple** (sonnet)
- `quest.execute`는 기본 simple이지만 **프로젝트별·원정별 override** 가능 (어려운 리팩터는 opus로). 자세히는 §5.

---

## 4. 런타임 · 모델 설정 시스템 (`agents.json`)

### 4.1 위치
`~/.luida/agents.json` (런타임 데이터). `luida db init` 시 기본값으로 생성. 예시는 `docs/examples/agents.json`.

### 4.2 스키마
```jsonc
{
  "version": 1,
  "defaults": { "runtime": "claude", "tier": "simple" },

  // 런타임 정의 — 각 런타임이 어떤 CLI/API로 호출되는지 + tier별 기본 모델
  "runtimes": {
    "claude": {
      "kind": "claude-cli",
      "command": "claude",
      "models": { "complex": "claude-opus-4-7", "simple": "claude-sonnet-4-6" }
    },
    "codex": {
      "kind": "codex-cli",
      "command": "codex",
      "models": { "complex": "gpt-5.1-codex-max", "simple": "gpt-5.1-codex-mini" }
    },
    "deepseek": {
      "kind": "openai-compatible",
      "baseUrl": "https://api.deepseek.com",
      "apiKeyEnv": "DEEPSEEK_API_KEY",
      "models": { "complex": "deepseek-reasoner", "simple": "deepseek-chat" }
    }
  },

  // 행위별 매핑 — runtime/model/tier override. 생략 필드는 defaults + tier 기본 모델로 해소
  "actions": {
    "campaign.plan":     { "runtime": "claude", "tier": "complex" },
    "quest.execute":     { "runtime": "claude", "tier": "simple" },
    "quest.review":      { "runtime": "claude", "tier": "complex" },
    "escalation.triage": { "runtime": "claude", "tier": "complex" },
    "learning.reflect":  { "runtime": "claude", "tier": "complex" },
    "merge.resolve":     { "runtime": "claude", "tier": "complex" },
    "campaign.report":   { "runtime": "claude", "tier": "simple" },
    "project.ingest":    { "runtime": "claude", "tier": "simple" },
    "pr.describe":       { "runtime": "claude", "tier": "simple" },
    "inmail.summarize":  { "runtime": "claude", "tier": "simple" }
  },

  // (선택) 프로젝트별 override — 특정 모험지의 quest.execute는 다른 런타임/모델
  "projectOverrides": {
    "community-web-agora": {
      "quest.execute": { "runtime": "codex", "tier": "complex" }
    }
  }
}
```

### 4.3 해소(resolution) 규칙 — 우선순위
1. `projectOverrides[project][action]` (있으면)
2. `actions[action]`
3. `defaults`
4. 최종 모델 = action/override에 `model`이 명시되면 그것, 아니면 `runtimes[runtime].models[tier]`

```
resolve(action, project?) → { runtime, model, kind, command|baseUrl }
```

### 4.4 검증
- `agents.json` 로드 시 Zod-lite validator(`@luida/core/validators` 확장)로 스키마 검증
- 알 수 없는 runtime 참조 / 모델 누락 시 명확한 에러 + 기본값 fallback

---

## 5. 런타임 추상화

### 5.1 인터페이스 (core)
```ts
// @luida/core/src/agents/types.ts
export type AgentTier = 'complex' | 'simple';

export type ResolvedAgent = {
  action: string;
  runtime: string;        // 'claude' | 'codex' | 'deepseek' | ...
  model: string;          // 'claude-opus-4-7' 등
  kind: RuntimeKind;      // 'claude-cli' | 'codex-cli' | 'openai-compatible'
};

export type AgentInvocation = {
  prompt: string;
  cwd?: string;           // worktree path (quest.execute 등)
  systemContext?: string; // 학습 맥락 주입
  sessionId?: string;
  stream?: boolean;
};

export type AgentEvent =
  | { kind: 'text'; text: string }
  | { kind: 'tool_use'; name: string; input: unknown }
  | { kind: 'escalation'; category: string; message: string }  // v2 신규
  | { kind: 'result'; success: boolean; summary?: string }
  | { kind: 'error'; message: string };

export interface AgentRuntime {
  readonly kind: RuntimeKind;
  run(model: string, inv: AgentInvocation): AsyncIterable<AgentEvent>;
}
```

### 5.2 구현 (신규 패키지 `@luida/runtimes`)
| kind | 호출 방식 |
|---|---|
| `claude-cli` | `claude -p --model <model> --output-format stream-json` (기존 ClaudeWorkerRunner 일반화) |
| `codex-cli` | `codex exec --model <model> ...` (stream 파싱) |
| `openai-compatible` | `POST {baseUrl}/chat/completions` (deepseek 등). stream=SSE 파싱 |

→ v1의 `WorkerRunner`는 이 인터페이스의 `claude-cli` 케이스로 흡수.

### 5.3 escalation 이벤트
worker가 판단이 필요할 때 `{ kind: 'escalation', category, message }` 이벤트를 emit:
- claude-cli: worker가 특정 마커(예: `<<LUIDA_ASK category=design_mismatch>>...<<END>>`)를 출력하거나 ask 도구 호출 → 파서가 escalation 이벤트로 변환
- 그 외 런타임: stdout 패턴 매칭 또는 도구 규약

---

## 6. 데이터 모델 (신규/변경)

```sql
-- 신규: 모험지 (등록 메뉴가 채움)
CREATE TABLE projects (
  name          TEXT PRIMARY KEY,
  repo_path     TEXT NOT NULL,
  base_branch   TEXT NOT NULL DEFAULT 'main',
  description   TEXT,
  context_path  TEXT,            -- memory/projects/<name>.md
  registered_at INTEGER NOT NULL,
  last_ingested_at INTEGER
);

-- 신규: 원정 (플래너 산출물)
CREATE TABLE campaigns (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  title         TEXT NOT NULL,
  prompt        TEXT NOT NULL,   -- 사용자 원본 프롬프트
  plan_json     TEXT NOT NULL,   -- DAG (quests + 의존성)
  status        TEXT NOT NULL,   -- planning|confirmed|running|needs_input|completed|failed|aborted
  report_path   TEXT,            -- 완료 보고서 (모험의 서)
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  completed_at  INTEGER
);

-- 변경: quests
--   + campaign_id (어느 원정 소속)
--   + project (dispatched_to → project 참조)
--   + depends_on_quest_id (DAG 의존성)
--   + status에 'needs_input' 추가
ALTER TABLE quests ADD COLUMN campaign_id INTEGER REFERENCES campaigns(id);
ALTER TABLE quests ADD COLUMN project TEXT REFERENCES projects(name);
ALTER TABLE quests ADD COLUMN depends_on_quest_id INTEGER REFERENCES quests(id);
-- needs_input은 CHECK 제약 확장 마이그레이션

-- adventurers 테이블은 deprecated (v1 cmux pane 개념).
--   호환을 위해 남기되 'main'/'brain' 역할만 사용, worker는 ephemeral.
```

> `inmail`/`events`/`relationships`/`memory`는 그대로 재사용. inmail은 escalation·ack에 계속 쓰임.

---

## 7. 주요 흐름

### 7.1 첫 기동
```
TUI/Web 진입 → tavern.db 없으면 자동 init → agents.json 없으면 기본값 생성
→ "모험지가 없습니다. 등록해주세요" empty state
```

### 7.2 모험지 등록
```
등록 메뉴 → name, repo_path, base_branch 입력
→ projects INSERT
→ (백그라운드) project.ingest 행위 실행 → README/schema 요약 → memory/projects/<name>.md
```

### 7.3 새 원정
```
프롬프트 입력
→ campaign.plan 행위 (opus): projects 맥락 + chronicle 학습 → DAG 계획 (plan_json)
→ campaigns INSERT (status=planning) → 사용자에게 계획 미리보기
→ 사용자 확정 → status=confirmed
→ DAG 위상정렬 → 의존성 없는 quest부터:
     wt c "<branch>" (각 프로젝트) → quest.execute 행위 (sonnet)
→ 진행 모니터링 (quests/events + PTY 로그 tail)
```

### 7.4 escalation (모험 중 사건)
```
worker가 escalation 이벤트 emit
→ escalation.triage 행위 (opus): 카테고리 분류 + 사용자에게 물을지 결정
→ 물어야 하면:
     quest.status = needs_input
     inmail kind='escalation' → UI 비방해 토스트
     "🍺 모험 중 사건 — <project>가 <category>로 판단을 기다립니다"
→ 사용자 결정 → inmail 회신 → worker resume 또는 abort
```

### 7.5 완료 + 보고
```
campaign의 모든 quest 종료
→ campaign.report 행위 (sonnet): 원정 요약 작성
→ report_path 저장 + memory/chronicle.md append ("모험의 서")
→ campaigns.status = completed
→ UI에 "원정 완료 🍺" + 보고서 링크
```

---

## 8. Phase 분해

| Phase | 내용 | 의존 |
|---|---|---|
| **V2-P0** | `projects` 테이블 + 등록 메뉴(TUI/Web) + `luida project add/list/remove`. cmux send-key 경로 제거 | — |
| **V2-P1** | `agents.json` 스키마 + 로더 + validator + **Agent Resolver** (`@luida/core/agents`) | P0 |
| **V2-P2** | `@luida/runtimes` — ClaudeRuntime(기존 흡수) + CodexRuntime + OpenAI-compatible(deepseek). 행위별 호출 배선 | P1 |
| **V2-P3** | `@luida/planner` — campaign.plan 행위 + `campaigns` 테이블 + DAG 실행기 | P2 |
| **V2-P4** | escalation 흐름 (needs_input + triage 행위 + 비방해 알림 UI) | P2 |
| **V2-P5** | campaign.report + 모험의 서 통합 | P3 |
| **V2-P6** | PTY 직접 관리(node-pty) + 라이브 로그 tail (TUI/Web) | P0 |
| **V2-P7** | xterm.js 인터랙티브 세션 (선택) | P6 |
| **V2-P8** | project.ingest (등록 시 맥락 요약) + 지속 학습 강화 | P2, P3 |
| **V2-P9** | **Tauri 데스크탑 패키징** — TUI 화면(등록/원정/모니터)이 정착된 뒤, 동일 기능을 Web(React)로 맞춘 다음 Tauri로 래핑해 `Luida.app` 빌드. 자세히는 §12 | P0~P5 (TUI 흐름 완성), Web Track B |

각 Phase는 기존 4-게이트(typecheck/test/리뷰doc/리뷰agent) 규약을 그대로 따른다.

### UI 표면 순서
v2 UI는 **TUI 먼저 → Tauri 데스크탑 나중**의 순서로 간다:
1. **TUI(Ink)에서 v2 흐름을 먼저 구현** — 등록 메뉴, 원정 입력·계획 미리보기, 진행 모니터, escalation 알림, 모험의 서. 터미널 네이티브라 빠르게 반복 가능.
2. TUI에서 흐름이 검증되면 **동일 기능을 Web(React)로 포팅** (Web Track A/B의 디자인 자산 재사용).
3. **Tauri로 래핑**해 네이티브 `Luida.app` 데스크탑 앱으로 배포 (V2-P9).

두 표면은 같은 `tavern.db` + `agents.json` + Agent Resolver를 공유하므로 로직 중복은 없다. TUI는 터미널 사용자용, Tauri는 GUI·알림·모바일(PWA) 확장용.

---

## 9. 미해결 질문 (구현 전 결정)

1. **codex/deepseek 실제 호출 규약**: codex CLI의 stream 포맷, deepseek tool-use 지원 범위 확인 필요. 초기엔 claude-cli만 완전 지원, 나머지는 text-only로 시작?
2. **escalation 마커 규약**: claude worker가 "물어봐야 함"을 어떻게 신호할지. 전용 도구 vs 출력 마커. Claude Code의 ask 패턴과 정합성.
3. **DAG 동시 실행 한도**: 의존성 없는 quest를 몇 개까지 병렬로? (자원·비용·머지 충돌)
4. **계획 미리보기 UX**: campaign.plan 결과를 사용자가 어떻게 검토·수정하는지 (전체 수락 vs quest별 편집)
5. **모델 비용 추적**: 행위별 토큰/비용을 events에 기록해 대시보드에 노출할지
6. **머지 전략**: 여러 프로젝트 PR을 묶을지, 프로젝트별 독립 PR인지 (v1 review의 머지 큐 deferral과 연결)
7. **PTY 라이브러리**: node-pty(네이티브 빌드) vs Bun 자체 PTY 지원 여부 확인

---

## 10. v1 → v2 마이그레이션

- v1 코드(`@luida/sidecar`)의 worker spawn·integrations는 대부분 재사용 (claude-cli 런타임으로 흡수)
- `adventurers`(cmux pane) 개념만 폐기 → `projects` + ephemeral worker
- tavern.db는 마이그레이션 0003_* (projects/campaigns + quests 컬럼 추가)으로 in-place 업그레이드
- Web/TUI는 등록 메뉴 + 원정 화면 추가, 기존 대시보드 패널 재배치

---

## 12. Tauri 데스크탑 패키징 (V2-P9)

TUI에서 v2 흐름이 검증된 뒤 진행. Web Track B에서 만든 `packages/web/src-tauri/` shim(Option α)을 v2 화면에 맞춰 확장한다.

### 12.1 구성
- **frontend**: `packages/web/` (React + Vite). v2 화면 — 등록 메뉴, 원정 입력/계획 미리보기, 진행 모니터(xterm.js 로그), escalation 토스트, 모험의 서
- **backend**: Bun.serve (`/api/snapshot`, `/api/stream` SSE) — 이미 있음. v2 엔드포인트 추가:
  - `POST /api/projects` (등록), `POST /api/campaigns` (원정 생성→plan)
  - `POST /api/campaigns/:id/confirm`, `POST /api/quests/:id/decision` (escalation 응답)
- **Tauri shim**: `src-tauri/` — 윈도우 + native notification(escalation을 OS 알림으로) + 트레이 아이콘
- **데몬 통합**: Tauri 앱 시작 시 Bun backend + brain daemon을 sidecar 프로세스로 함께 기동 (`tauri.conf.json`의 외부 바이너리 또는 `Command::sidecar`)

### 12.2 빌드
```bash
cd packages/web
bun run build           # Vite → dist/
cargo tauri build       # → Luida.app + .dmg
```

### 12.3 native 알림 (escalation)
- escalation inmail 발생 → Tauri `notification` API로 OS 알림: "🍺 모험 중 사건 — agora가 판단을 기다립니다"
- 클릭 → 앱 포커스 + 해당 quest 상세로 deep-link
- 비방해 톤: 일반 progress는 알림 안 함, `needs_input`/`failed`만

### 12.4 순서 재확인
TUI(V2-P0~P5) → Web 포팅 → **Tauri 래핑(V2-P9)**. TUI 없이 Tauri부터 가지 않는다 (TUI가 흐름 검증 + 빠른 반복 수단).

---

## 11. 변경 이력
| 날짜 | 버전 | 변경 |
|---|---|---|
| 2026-05-28 | 0.1 | 최초 draft — 행위 분류 + agents.json + Phase 분해 |
| 2026-05-28 | 0.2 | UI 표면 순서(TUI→Tauri) + V2-P9 Tauri 패키징 §12 추가 |
