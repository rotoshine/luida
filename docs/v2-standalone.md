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
    // ── backlog (v2 범위 밖, 인터페이스 자리만) ──────────────
    // API 기반 런타임. 실제 필요 생기면 승격. 기본 비활성.
    "deepseek": {
      "kind": "openai-compatible",
      "enabled": false,
      "baseUrl": "https://api.deepseek.com",
      "apiKeyEnv": "DEEPSEEK_API_KEY",
      "models": { "complex": "deepseek-reasoner", "simple": "deepseek-chat" }
    },
    "ollama": {
      "kind": "openai-compatible",
      "enabled": false,
      "baseUrl": "http://127.0.0.1:11434/v1",
      "models": { "complex": "qwen2.5-coder:32b", "simple": "qwen2.5-coder:7b" }
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

**전제: claude·codex는 해당 PC에 설치된 로컬 CLI를 그대로 사용한다. API 기반 호출(openai-compatible)은 backlog.**

| kind | 상태 | 호출 방식 |
|---|---|---|
| `claude-cli` | **v2 1급 지원** | `claude -p --model <model> --output-format stream-json` (기존 ClaudeWorkerRunner 일반화). 로컬 `claude` CLI 전제 |
| `codex-cli` | **v2 1급 지원** | `codex exec --model <model> ...` (stream 파싱). 로컬 `codex` CLI 전제 |
| `openai-compatible` | **backlog** | `POST {baseUrl}/chat/completions` (deepseek/ollama). API 키·HTTP 클라이언트 필요. v2 범위 밖, 인터페이스 자리만 마련 |

이유:
- 로컬 CLI는 **사용자의 기존 인증·구독을 그대로 사용** (claude Pro/Max, codex 로그인). 별도 API 키 관리 불필요.
- CLI는 **stream-json/도구 호출/세션 재개**가 이미 구현돼 있어 어댑터가 얇음.
- API 경로는 인증·rate limit·tool-use 규약·SSE 파싱이 런타임마다 달라 비용이 큼 → 실제 필요가 생기면 backlog에서 승격.

→ v1의 `WorkerRunner`는 `claude-cli` 케이스로 흡수. `codex-cli`가 v2에서 두 번째로 추가되는 1급 런타임.

**런타임 가용성 체크**: AgentResolver는 호출 전 해당 CLI가 PATH에 있는지 확인 (`which claude` / `which codex`). 없으면 명확한 에러 + agents.json에서 다른 런타임으로 fallback 안내.

### 5.3 escalation 이벤트
worker가 판단이 필요할 때 `{ kind: 'escalation', category, message }` 이벤트를 emit:
- claude-cli: worker가 특정 마커(예: `<<LUIDA_ASK category=design_mismatch>>...<<END>>`)를 출력하거나 ask 도구 호출 → 파서가 escalation 이벤트로 변환
- 그 외 런타임: stdout 패턴 매칭 또는 도구 규약

### 5.4 TokenJuice — LLM 전단 압축 레이어 (OpenHuman 차용)
모든 AgentRuntime 호출 직전에 prompt/context를 압축하는 미들웨어. 비용·지연 절감(목표 최대 ~80%).

```ts
// @luida/core/src/agents/tokenjuice.ts
export function compressContext(input: string, opts?: CompressOpts): string;
```
규칙:
- HTML → Markdown 변환, 긴 URL 단축
- 중복 제거 + 장황한 도구 결과(stream-json, 스크랩, diff) 요약
- **CJK·이모지 등 멀티바이트는 grapheme 단위 보존** (이미 `firstLine`/`truncate`에서 codepoint-safe 구현 → grapheme까지 확장: `Intl.Segmenter`)
- 토큰 budget 초과분만 압축 (작은 입력은 그대로)

적용 지점: `project.ingest`(대용량 README/schema), `campaign.plan`(다중 프로젝트 맥락), `quest.review`(diff), `learning.reflect`(events 묶음)에서 특히 효과적.

### 5.5 실행 모드 (Execution Mode) — headless vs interactive

worker 실행은 **두 모드**를 모두 지원한다. 행위/프로젝트별로 `agents.json`의 `mode` 필드로 선택 (기본 `headless`).

> **공통**: 두 모드 모두 `wt c "<코드네임>"`으로 worktree를 먼저 만든 뒤 그 안에서 worker를 띄운다. 차이는 worker를 어떻게 띄우고 제어하느냐.

> **Ghostty 직접 제어 ❌**: Ghostty는 외부 제어 API가 없다(그게 cmux가 libghostty 위에 만든 레이어). "세션 살려두고 프롬프트 주입 + 출력 수신"은 **Luida가 소유한 PTY**(node-pty)로 구현하지 Ghostty를 조종하지 않는다. Ghostty는 사용자가 Luida 앱을 띄우는 환경일 뿐.

| 모드 | 띄우는 법 | 맥락 이어짐 | 추가 입력 | 출력 |
|---|---|---|---|---|
| **headless** (기본) | `claude -p --session-id <quest>` / `codex exec` | 디스크 transcript (`--resume` reload) | escalation→needs_input→`--resume` 사이클 (§5.6) | stream-json (구조화) |
| **interactive** | node-pty로 `claude`(REPL) spawn, Luida가 PTY 소유 | live 메모리 (reload 없음) | PTY stdin에 바로 주입 (실시간) | PTY stdout 캡처 → markdown/sqlite, Ink/xterm.js 렌더 |

선택 기준:
- **headless**: quest 1개가 범위 명확 / escalation·진행 파싱이 깔끔해야 함 / 비용 효율. 대부분의 `quest.execute`.
- **interactive**: 긴 모험에서 사용자가 자주 끼어듦 / 실시간 조종 / 여러 step 연속. 라이브 모니터링 화면에서 그대로 조작.

`agents.json` 확장:
```jsonc
"actions": {
  "quest.execute": { "runtime": "claude", "tier": "simple", "mode": "headless" }
}
```

### 5.6 headless의 추가 입력 — escalation → needs_input → resume 사이클

headless는 실행 도중 stdin을 못 받는다(`-p`는 프롬프트 1개 소비 후 실행). 따라서 추가 입력은 **"종료 → 입력 받음 → 재개"** 비동기 사이클로 처리:

```
1. claude -p --session-id quest-42 "<brief>" 실행
2. worker가 판단 필요 → escalation 마커 출력 후 멈춤/종료
     <<LUIDA_ASK category=design_mismatch>>질문 내용<<END>>
3. Luida가 stream-json에서 마커 감지 → quest.status = needs_input
4. escalation.triage(opus)가 분류: 진짜 사용자가 필요한가? (§7.4)
     - trivial → 기본값으로 자동 결정, 사용자 안 깨움
     - 진짜 필요 → 비방해 알림 "🍺 모험 중 사건 — agora가 판단을 기다립니다"
5. 사용자 결정 입력 (TUI/Web/알림)
6. claude -p --resume quest-42 "<사용자 답변>" → 직전 맥락 그대로 이어서 재개
```

핵심: `--session-id`/`--resume`이 "프로세스가 죽었다 살아나도" 맥락을 잇는다. 사용자 체감은 "물어봄 → 답하면 이어감"으로 자연스럽고, 실제로는 프로세스 재시작.

**escalation 마커 규약** (모든 런타임 공통, brief 프롬프트에 규약 주입):
```
<<LUIDA_ASK category=<system_error|ambiguous_spec|design_mismatch|dangerous_op> >>
사용자에게 물을 질문
<<END>>
```
- interactive 모드에선 마커 없이도 worker가 자연스럽게 멈춰 물을 수 있음 (PTY 대화). 마커는 headless의 신호 수단.

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

### 6.1 Memory Tree + Obsidian Vault (OpenHuman 차용)

현재 메모리는 flat (`chronicle.md` + `projects/<name>.md` + `patterns/*.md`). v2는 이를 **계층 요약 트리 + Obsidian 호환 vault**로 업그레이드한다.

**Memory Tree**
- 모든 메모(이벤트 묶음·원정 보고서·프로젝트 맥락)를 **≤3k 토큰 Markdown 청크**로 정규화·점수화
- 청크들을 **계층 요약 트리**로 묶음: leaf(원자 청크) → 중간 요약 → 루트 요약
- `reflect`와 `campaign.plan`은 raw events가 아니라 **요약 트리의 상위 노드**를 읽어 맥락 주입 → 토큰 효율 + 장기 기억
- 저장: `tavern.db`에 `memory_chunks(id, parent_id, level, score, token_estimate, path, summary, created_at)` 테이블 + 본문은 vault `.md`

**Obsidian Vault (로컬 우선 KB)**
- `~/.luida/memory/`를 **Obsidian 호환 vault**로 구성:
  - frontmatter(`---`)에 메타(type, score, links)
  - `[[wikilink]]`로 청크·프로젝트·원정 상호 연결 (chronicle ↔ campaign report ↔ project context)
  - 사용자가 Obsidian으로 직접 열람·편집 → 편집분은 다음 reflect에 반영
- "모험의 서" = vault의 chronicle 섹션. 원정 보고서가 wikilink로 엮인 지식 그래프
- Karpathy LLM-wiki 워크플로우 영감 (OpenHuman과 동일 계보)

```
~/.luida/memory/            ← Obsidian vault
├── chronicle/
│   └── 2026-05.md
├── campaigns/
│   └── 0042-schema-migration.md   (frontmatter + [[agora]] [[admin]] 링크)
├── projects/
│   ├── agora.md
│   └── admin.md
├── patterns/
│   └── 2026-05-28-agora-to-admin.md
└── .obsidian/              (vault 설정 — 선택)
```

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

### 7.4 escalation (모험 중 사건) — 두 모드 공통
```
worker가 escalation 신호 emit
  - headless: <<LUIDA_ASK category=...>> 마커 출력 후 종료 (§5.6)
  - interactive: PTY에서 멈춰 질문 (자연 대화)
→ escalation.triage 행위 (opus): 카테고리 분류 + 사용자에게 물을지 결정
   (trivial이면 자동 결정, 사용자 안 깨움)
→ 물어야 하면:
     quest.status = needs_input
     inmail kind='escalation' → UI 비방해 토스트
     "🍺 모험 중 사건 — <project>가 <category>로 판단을 기다립니다"
→ 사용자 결정:
     - headless: claude -p --resume <quest> "<답변>"으로 재개
     - interactive: PTY stdin에 답변 주입
   또는 abort
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
| **V2-P10** | **TokenJuice 압축 레이어** — AgentRuntime 전단 미들웨어. §5.4 | P2 |
| **V2-P11** | **Memory Tree + Obsidian Vault** — 청크화·계층 요약 트리 + vault 호환. §6.1 | P8 (project.ingest), 학습 |
| **V2-P12** | **모험 중단·재개 (Suspend/Resume)** — git handoff 브랜치로 미커밋 포함 통째 이전. single owner 잠금. §14 | P3 (campaigns) |

각 Phase는 기존 4-게이트(typecheck/test/리뷰doc/리뷰agent) 규약을 그대로 따른다.

### UI 표면 순서
v2 UI는 **TUI 먼저 → Tauri 데스크탑 나중**의 순서로 간다:
1. **TUI(Ink)에서 v2 흐름을 먼저 구현** — 등록 메뉴, 원정 입력·계획 미리보기, 진행 모니터, escalation 알림, 모험의 서. 터미널 네이티브라 빠르게 반복 가능.
2. TUI에서 흐름이 검증되면 **동일 기능을 Web(React)로 포팅** (Web Track A/B의 디자인 자산 재사용).
3. **Tauri로 래핑**해 네이티브 `Luida.app` 데스크탑 앱으로 배포 (V2-P9).

두 표면은 같은 `tavern.db` + `agents.json` + Agent Resolver를 공유하므로 로직 중복은 없다. TUI는 터미널 사용자용, Tauri는 GUI·알림·모바일(PWA) 확장용.

---

## 9. 미해결 질문 (구현 전 결정)

1. ~~codex/deepseek 실제 호출 규약~~ **[결정됨]** claude·codex는 **로컬 CLI 전제**로 v2 1급 지원. deepseek/ollama(openai-compatible)는 **backlog** (`enabled:false`). codex CLI의 stream 포맷·tool-use 규약 확인은 codex-cli 어댑터 구현 시점(V2-P2)에 진행.
2. ~~escalation 마커 규약~~ **[결정됨]** `<<LUIDA_ASK category=...>>질문<<END>>` 출력 마커 (headless 신호 수단, brief에 규약 주입). interactive는 PTY 자연 대화. 카테고리: system_error/ambiguous_spec/design_mismatch/dangerous_op. 자세히는 §5.6.
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

## 14. 모험 중단 · 재개 (Suspend / Resume)

여러 노트북(집·회사)을 오가며 같은 작업을 이어가는 시나리오. **커밋·푸시되지 않은 진행물까지** 통째로 다른 기기로 넘긴다.

### 14.1 메타포 — 게임의 "중단 세이브"
고전 게임 모바일 포팅의 suspend save와 동일: 세이브포인트(정식 저장)와 별개로, "지금 이 순간 상태"를 봉인했다 다른 기기에서 그 자리에서 재개.

| 게임 | Luida |
|---|---|
| 일반 세이브 (세이브포인트) | git commit/push (worklog, 정식 저장) |
| **중단 세이브 (suspend)** | `luida adventure suspend` (미커밋 포함 통째 봉인) |
| 다른 기기에서 이어하기 | `luida adventure resume` |
| 한 번에 한 기기만 플레이 | single owner 잠금 |
| 재개하면 중단 세이브 소멸 | resume 후 WIP 커밋 폐기 |

### 14.2 Transport — git handoff 브랜치 (확정: A안)
- 미커밋 변경을 `luida/handoff/<campaign>` 브랜치에 **WIP 커밋**으로 임시 봉인
- 이 브랜치는 운반용 봉투 — resume 시 patch로 펼친 뒤 WIP 커밋은 폐기. **main/feature 히스토리 안 더럽힘.**
- 인프라 추가 0 (이미 쓰는 git remote 활용)

> 향후 외부 스토리지(S3/R2)·P2P가 필요해지면 `HandoffTransport` 인터페이스로 추상화 가능. 기본은 git.

### 14.3 무엇이 봉인되나
| 항목 | 방법 |
|---|---|
| 미커밋 코드 (tracked) | `git diff` patch |
| 미커밋 파일 (untracked) | 번들에 파일 내용 포함 (`git status --porcelain` 기준) |
| campaign/quest 진행 상태 | `.luida-handoff.json` (resume 시 id 재매핑하며 import) |
| 대화 맥락 | 관련 memory 청크 export → resume 시 머지 |
| (선택) Claude transcript | `--include-transcript` |

`node_modules`·build 산출물은 봉인 안 함 → resume 기기에서 재설치.

### 14.4 프로토콜
```
A 노트북                                  B 노트북
luida adventure suspend <campaign>        luida adventure resume <campaign>
  1. worktree 미커밋 수집                    1. handoff 브랜치 fetch
     (tracked diff + untracked 파일)        2. wt c "<원래 branch>" (origin/main 기준)
  2. WIP 커밋 → luida/handoff/<campaign>     3. patch 펼침 → A 미커밋 상태 바이트 복원
  3. .luida-handoff.json 첨부 커밋            4. .luida-handoff.json import (id 재매핑)
     (campaign+quests+inmail+events          5. owner = B 로 이전, handoff_state=resumed
      +memory 청크 +worklog 요약)             6. WIP 커밋 폐기 → 이어서 진행
  4. git push                               
  5. owner=suspended (A는 읽기전용 잠금)        마무리: 정식 commit/push 또는 다시 suspend
```

### 14.5 single owner 잠금
- `campaigns.owner_machine` + `campaigns.handoff_state` (`active` | `suspended` | `resumed`)
- `machine_id`: hostname 또는 `~/.luida/machine-id` 파일
- suspend 안 한 campaign을 다른 기기가 resume 시도 → 경고 + `--force` 옵션
- 동시 진행 사고 차단: 한 시점에 한 머신만 `active`

### 14.6 데이터·행위·CLI
```sql
ALTER TABLE campaigns ADD COLUMN owner_machine TEXT;
ALTER TABLE campaigns ADD COLUMN handoff_state TEXT DEFAULT 'active';
  -- 'active' | 'suspended' | 'resumed'
```
- **행위** `handoff.bundle` (simple tier): suspend 시 "어디까지 했는지" worklog 요약 자동 작성 → 기존 수동 worklog 관습의 자동화
- **CLI** (사용자 노출은 모험 메타포):
  ```
  luida adventure suspend <campaign>   # 내부: handoff push
  luida adventure resume <campaign>    # 내부: handoff pull
  luida adventure status               # 어느 기기에 중단 세이브가 있는지
  ```
- **UI 카피**:
  - 중단: "🏕 집 노트북에서 모험을 잠시 멈췄습니다. 다른 곳에서 이어갈 수 있어요."
  - 재개: "⚔ 회사 노트북에서 중단된 모험을 이어받았습니다. (집 노트북은 이 모험이 잠깁니다)"
  - 충돌: "⚠ 이 모험은 집 노트북에서 아직 진행 중입니다. 먼저 그쪽에서 중단(suspend)해주세요."

### 14.7 안전·엣지케이스
- **tavern.db 직접 동기화 금지** — SQLite WAL 손상 위험. 항상 번들(.luida-handoff.json)만 운반.
- resume 시 worktree가 이미 있으면 (같은 branch) → 사용자 확인 후 덮어쓰기 or 별도 worktree
- suspend 중 네트워크 실패 → WIP 커밋은 로컬에 남으므로 재시도 가능 (멱등)
- transcript 옵션은 민감정보 포함 가능 → 기본 off, 명시 opt-in + 시크릿 마스킹(`maskSecrets`) 통과

### 14.8 Phase
**V2-P12** (의존: V2-P3 campaigns) — suspend/resume 프로토콜 + owner 잠금 + 번들 import/export. transport는 git 고정, 인터페이스로 추상화 여지만 남김.

---

## 13. 외부 참고 — OpenHuman ([github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman), GPL-3.0)

차용한 개념과 Luida 매핑:

| OpenHuman | Luida 반영 | Phase |
|---|---|---|
| **Memory Tree** (≤3k 토큰 청크 + 계층 요약 트리, 로컬 SQLite) | §6.1 — `memory_chunks` 테이블 + 요약 트리. reflect/plan이 상위 노드 참조 | V2-P11 |
| **Obsidian Wiki 로컬 우선 KB** (Karpathy LLM-wiki) | §6.1 — `~/.luida/memory/`를 vault화 (frontmatter + wikilink). "모험의 서" 지식 그래프 | V2-P11 |
| **TokenJuice** (LLM 전단 토큰 압축, grapheme-safe, ~80%↓) | §5.4 — AgentRuntime 전단 압축 미들웨어 | V2-P10 |
| **Model routing** (작업별 LLM + Ollama 로컬) | §4 — agents.json 행위별 매핑 (이미 설계). ollama runtime 추가 | V2-P1~P2 |
| agentmemory 공유 저장소 (Claude Code/Cursor/Codex 공유) | 후순위 — tavern.db/vault를 외부 도구와 공유하는 통합. v3 후보 | (미정) |
| 118+ OAuth 연동 / 데스크탑 마스코트 / 음성 | Luida 범위 밖 (참고만) | — |

차용 원칙: **로컬 우선·markdown·계층 요약·토큰 절감**이 Luida 철학(`~/.luida/`, memory markdown, brain 학습)과 정확히 일치. GPL-3.0이므로 코드 직접 복사 대신 **개념·구조만 재해석**해 자체 구현.

---

## 11. 변경 이력
| 날짜 | 버전 | 변경 |
|---|---|---|
| 2026-05-28 | 0.1 | 최초 draft — 행위 분류 + agents.json + Phase 분해 |
| 2026-05-28 | 0.2 | UI 표면 순서(TUI→Tauri) + V2-P9 Tauri 패키징 §12 추가 |
| 2026-05-28 | 0.3 | OpenHuman 차용 — Memory Tree·Obsidian vault(§6.1) + TokenJuice(§5.4) + ollama runtime + V2-P10/P11 + §13 |
| 2026-05-28 | 0.4 | 모험 중단·재개(Suspend/Resume) §14 + V2-P12 — git handoff 브랜치, single owner 잠금, 미커밋 통째 이전 |
| 2026-05-28 | 0.5 | 런타임 정책 확정 — claude·codex 로컬 CLI 전제(1급), openai-compatible(deepseek/ollama)은 backlog(`enabled:false`) + CLI 가용성 체크 |
| 2026-05-28 | 0.6 | 실행 모드 §5.5 (headless/interactive 둘 다 지원) + headless 추가입력 사이클 §5.6 + escalation 마커 규약 확정 + §7.4 두 모드 커버 |
