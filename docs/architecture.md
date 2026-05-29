# Luida 아키텍처 (v2 · Rust) — 구현 기준 설명서

> 이 문서는 **현재 구현된 코드의 전체 구조**를 Rust에 익숙하지 않아도 따라올 수 있게 설명합니다.
> 설계 정본(왜 이렇게 설계했나)은 [`v2-standalone.md`](v2-standalone.md), 운영법은 [`operations.md`](operations.md).
> 기능을 추가할 때마다 이 문서도 함께 갱신합니다. (마지막 갱신: TUI 진행 바(Gauge) + 원정 목록 완료/전체 진행도)

---

## 1. 한 문장 요약

> 사용자가 "원정(campaign)"을 의뢰하면 → LLM 플래너가 여러 프로젝트에 걸친 "모험(quest)" 묶음으로 쪼개고 → 각 모험을 격리된 worktree에서 worker(claude/codex)로 실행하고 → 모든 상태·진행을 **하나의 SQLite(`tavern.db`)에 기록**하며 → 완료되면 보고서를 남기고 반복 패턴을 학습한다.

핵심 원칙 **"모든 상태의 단일 진실은 `tavern.db`"** — CLI·TUI·서버는 전부 같은 DB를 읽고 쓰는 얇은 표면(front-end)일 뿐입니다.

---

## 2. crate(=패키지) 구조와 의존 방향

Rust는 프로젝트를 **crate** 단위로 나눕니다(JS의 워크스페이스 패키지와 비슷). 화살표 `A → B` 는 "A가 B를 사용(의존)한다"는 뜻이고, **화살표는 한 방향으로만** 흐릅니다(순환 금지).

```
                    ┌─────────────┐
   진입점(표면)      │  luida-cli   │   luida-tui    luida-server
   (사용자 대면)     └──────┬──────┘       │             │
                           │              │             │
        ┌──────────────────┼──────────────┼─────────────┘
        ▼                  ▼              ▼
   luida-planner ───► luida-sidecar ──► luida-runtimes ──┐
   (원정 계획·실행)     (모험 실행 배선)   (claude/codex 실행)│
        │                  │                              │
        └────► luida-brain ┘                              │
        │      (학습·기억·보고)                            │
        ▼                                                 ▼
   ┌─────────────────────────────────────────────────────────┐
   │  luida-core  ── 모두의 기반: 스키마·DB·모델·repository    │
   │               + agents.json 해소 + 공용 헬퍼            │
   └─────────────────────────────────────────────────────────┘
```

| crate | 한 줄 역할 | 대표 타입/함수 |
|---|---|---|
| **luida-core** | tavern.db 스키마·연결·모델·repository, agents.json 해소, 공용 헬퍼(`is_fake`/`open_ready`) | `Connection`, `ProjectRepo`·`QuestRepo`·`EventRepo`…, `resolve`, `AgentRuntime`(trait) |
| **luida-runtimes** | 실제 CLI(claude/codex) 실행 + 데모용 `FakeRuntime` + factory | `make_factory`, `runtime_for_kind`, `FakeRuntime` |
| **luida-sidecar** | 모험 1건 실행 배선: worktree 생성 → worker 실행 → events/상태 기록 → escalation triage → 관계 트리거 | `dispatch_quest`, `resume_quest`, `make_worktree`, `WorktreeProvider`(trait) |
| **luida-planner** | 원정 계획(LLM→DAG 파싱·검증·위상정렬)과 실행 루프 | `plan_campaign`, `run_campaign` |
| **luida-brain** | 모험의 서(Obsidian vault): 프로젝트 맥락 ingest, 원정 보고서, reflect(관계 학습), Memory Tree | `ingest_project`, `report_campaign`, `reflect`, `MemoryVault` |
| **luida-server** | Axum HTTP/SSE — GUI/웹 브리지 | `/api/health`·`/api/snapshot`·`/api/stream`·`POST /api/projects` |
| **luida-tui** | ratatui 터미널 대시보드 + 명령(plan/run/resume/triage) + 상세 뷰 | `run(db_path)`, `Dashboard`, `dispatch` |
| **luida-cli** | 단일 진입점 `luida` (clap) — 위 기능들을 명령으로 노출 | `main()` |

> 왜 이렇게 나누나? **표면(cli/tui/server)을 바꿔도 핵심 로직(planner/sidecar/brain)은 그대로 재사용**하기 위해서입니다. 그래서 "명령 디스패치 자원"(런타임 factory, worktree, DB 열기)을 `core/runtimes/sidecar`의 공용 함수로 빼서 cli와 tui가 똑같이 씁니다.

---

## 3. 데이터 모델 — tavern.db의 테이블 = Rust 구조체

`tavern.db`(SQLite)의 각 테이블은 `luida-core/src/models.rs`의 구조체와 1:1 대응합니다.

| 테이블/구조체 | 의미 | 핵심 필드 |
|---|---|---|
| `Project` (모험지) | 등록된 git 프로젝트 | name, repo_path, base_branch |
| `Campaign` (원정) | 사용자 요청 1건 = quest 묶음 | id, title, prompt, plan_json, status |
| `Quest` (모험) | 프로젝트 1곳에 대한 작업 1건 | id, campaign_id, project, brief, status, progress, depends_on |
| `Event` (사건 로그) | 진행의 모든 단계 기록 | campaign_id, quest_id, actor, **kind**, payload(JSON), occurred_at |
| `Inmail` (편지) | 세션 간 메시지·escalation 알림 | from/to_session, kind, payload |
| `Relationship` (관계) | 프로젝트 간 자동화 규칙 | from_project, trigger_kind, to_project, action, enabled |
| `MemoryChunk` | Memory Tree(요약 트리) 노드 | — |

**`Event.kind` 값이 곧 "진행 타임라인"**입니다 (TUI 상세 뷰가 이걸 읽습니다):
`campaign_planned` → `quest_dispatched` → `tool_use` → `quest_completed` / `quest_needs_input` / `quest_failed` → `trigger_dispatched`

### Repository 패턴
DB 접근은 **repository 구조체**를 거칩니다. 날 SQL을 여기저기 흩지 않고 한곳에 모으는 패턴입니다.
```rust
// 항상 이 형태: Repo::new(&conn) 로 만들고 메서드 호출
ProjectRepo::new(&conn).list()?;                 // 프로젝트 전체
QuestRepo::new(&conn).ready_in_campaign(cid)?;   // 실행 가능한 quest
EventRepo::new(&conn).for_campaign(cid, 200)?;   // 원정 타임라인 (이번에 추가)
```

---

## 4. 핵심 흐름: 원정 계획 → 실행 → 보고

```
luida campaign plan "agora·admin에 검색 필터 추가"
   │  plan_campaign (luida-planner)
   │   ├─ resolve("campaign.plan") → 어떤 런타임/모델 쓸지 결정 (agents.json)
   │   ├─ make_factory()() → AgentRuntime 생성 → LLM 호출 → quest DAG(JSON)
   │   ├─ CampaignPlan::parse + validate(위상정렬, 사이클 검사)
   │   └─ campaigns/quests 테이블에 영속 + "campaign_planned" event
   ▼
luida campaign run <id>
   │  run_campaign (luida-planner) — 의존성 순 루프
   │   for 준비된 quest:
   │     dispatch_quest (luida-sidecar)
   │       ├─ make_worktree() → 격리 작업공간 (실모드 wt / 데모 temp)
   │       ├─ "quest_dispatched" event
   │       └─ runtime.run(on_event) → worker 실행
   │            on_event: ToolUse/Text/… → events 테이블에 실시간 기록
   │       └─ 결과 → "quest_completed"/"needs_input"/"failed"
   │     needs_input?  → triage_escalation → 자동 해소 or 사용자 대기
   │     completed?    → fire_quest_completed → 관계 트리거(후속 quest 자동 생성)
   ▼
luida campaign report <id>   → 모험의 서(vault) markdown  (luida-brain)
luida reflect                → 최근 events 분석 → 관계 제안(학습)
```

---

## 5. 가장 중요한 추상화 2개 (Rust의 `trait`)

`trait`은 다른 언어의 **인터페이스**입니다. "이 메서드들을 구현하면 이 trait이다"라는 약속이고, 실제 구현체를 바꿔 끼울 수 있습니다.

### (1) `AgentRuntime` — "에이전트를 실행하는 무언가"
```rust
pub trait AgentRuntime {
    fn run(&self, model: &str, inv: &AgentInvocation,
           on_event: &mut dyn FnMut(&AgentEvent)) -> Result<AgentOutcome>;
}
```
- 구현체: `ClaudeCliRuntime`(실제 `claude -p` 실행), `CodexCliRuntime`, **`FakeRuntime`**(데모/테스트용 — 외부 LLM 없이 정해진 이벤트를 냄).
- `on_event` 콜백으로 진행 이벤트(도구 사용, 텍스트, 결과)를 흘려보냅니다 → sidecar가 이걸 받아 events 테이블에 기록.
- **factory 패턴**: 어떤 구현체를 쓸지는 `make_factory()`가 `LUIDA_FAKE` 환경변수를 보고 결정합니다. 그래서 같은 코드가 데모/실모드 양쪽에서 동작.

### (2) `WorktreeProvider` — "격리 작업공간을 만드는 무언가"
```rust
pub trait WorktreeProvider {
    fn create(&self, repo_path: &Path, codename: &str) -> Result<Worktree>;
}
```
- 구현체: `WorktrunkProvider`(실제 `wt`로 git worktree), **`TempWorktree`**(데모용 임시 폴더). `make_worktree()`가 선택.

> 이 두 trait 덕분에 **외부 LLM이나 git 없이도 전체 파이프라인을 테스트·시연**할 수 있습니다 (`LUIDA_FAKE=1`).

---

## 6. 표면(진입점)들이 핵심을 호출하는 법

세 진입점 모두 **같은 함수**(`run_campaign` 등)를 호출합니다. 차이는 "어떻게 띄우고 결과를 보여주냐"뿐입니다.

- **CLI** (`luida-cli/main.rs`): clap이 명령을 파싱 → 함수 호출 → `println!`로 출력. 동기·1회성.
- **TUI** (`luida-tui/lib.rs`): ratatui 렌더 루프. 명령은 **백그라운드 스레드**에서 실행(UI 안 멈추게)하고, `mpsc` 채널로 결과를 받습니다. 진행 상세는 **events 테이블을 폴링**해서 표시(작업 중).
- **Server** (`luida-server`): Axum이 HTTP 요청 → `spawn_blocking`으로 DB 작업 → JSON/SSE 응답.

### TUI 동시성 한눈에 (이번 작업 영역)
```
[메인 스레드] ratatui 루프            [워커 스레드] (명령 1건)
  draw() ──────────────►              dispatch(db_path, cmd)
  event::poll(150ms)                    ├ open_ready(db_path) (자기 conn)
  키 입력 → spawn_worker ──────────►     ├ run_campaign(...) → events 기록
  rx.try_recv() ◄──── mpsc 채널 ────────┘ 완료 시 결과 송신
  detail 폴링: 별도 read conn 으로 events 재조회 → 타임라인 갱신
```
- 워커는 자기 DB 연결을 따로 엽니다. 메인은 읽기 전용 연결. SQLite WAL 모드라 **동시에 읽고 써도 안전**.

**상세 뷰 (events 폴링 = 실시간 진행)**: 목록에서 `Enter`/`d`로 선택 항목의 상세 뷰를 열면 본문이 좌(목록)/우(타임라인)로 갈라지고, 오른쪽에 그 원정/모험의 `events`(📋계획→⚙디스패치→🔧도구→✅완료…)가 시간순으로 표시됩니다. 메인 루프가 150ms마다 별도 read conn으로 `EventRepo::for_campaign`/`for_quest`를 재조회하므로, 실행 중이면 타임라인이 **실시간으로 늘어납니다 — 콜백/시그니처 변경 없이 폴링만으로.** `Esc` 닫기, `x` 원정 실행. (이 방식을 택한 이유: `run_campaign`에 진행 콜백을 넣으면 planner·sidecar 시그니처와 4개 호출처를 바꿔야 하지만, events는 이미 기록 중이라 TUI가 읽기만 하면 됨.)

**TUI 키**: 앱에서 `?` 를 누르면 전체 키맵·한글 IME 자모 매핑을 오버레이로 볼 수 있습니다. 핵심: `Tab` 탭 · `j/k` 이동 · `Enter`/`d` 상세 · `x` 실행 · `p` 계획 · `r`/`t` 재개·triage · `n` 다음 판단대기(needs_input)로 점프 · `Esc` 닫기 · `q` 종료. 결과 토스트는 ~5초 후 자동 소멸하고, `LUIDA_FAKE` 모드면 헤더에 🧪 데모 배지가 뜹니다.

**진행도**: 원정 실행 중(Running)에는 헤더 아래에 진행 바(`Gauge`)가 떠서 `완료/전체 · 실행중 · 대기 · 실패`를 보여주고, **Campaigns 목록**에는 각 원정의 완료/전체(`2/3`)가 항상 표시됩니다. 둘 다 `QuestRepo::list_for_campaign`(완료 포함)으로 집계 — 진행 바는 run_loop 폴링(150ms), 목록 수치는 `Dashboard::load`에서 채웁니다(`campaign_progress` 맵). `Dashboard.quests`(=`list_active`)는 완료 quest를 빼므로, 완료율 분모는 반드시 `list_for_campaign`을 써야 한다는 점이 포인트.

---

## 7. Rust 입문 메모 (코드 읽을 때 자주 보이는 것)

| 표기 | 뜻 |
|---|---|
| `Result<T>` + `?` | 실패할 수 있는 함수. `?`는 "에러면 즉시 반환, 아니면 값 꺼냄". try/catch 대신 쓰는 명시적 에러 처리 |
| `Option<T>` (`Some`/`None`) | 값이 있을 수도/없을 수도. null 대신 |
| `Box<dyn AgentRuntime>` | trait 객체 = "AgentRuntime을 구현한 무언가"를 힙에 담은 것. 런타임에 구현체 교체 가능 |
| `&self` / `&mut self` | 읽기 빌림 / 쓰기 빌림. `&mut conn`은 "이 함수가 conn을 변경할 수 있음" |
| `impl Fn(&X) -> Y` | 함수/클로저를 인자로. `make_factory()`가 이런 클로저를 돌려줌 |
| `enum` + `match` | 여러 경우 중 하나. `Mode::Normal`/`Input`/`Running` 처럼. `match`로 모든 경우를 빠짐없이 처리 |
| `mpsc::channel()` | 스레드 간 메시지 큐 (송신 `tx`/수신 `rx`). 워커→메인 결과 전달 |
| `Drop` (RAII) | 값이 사라질 때 자동 정리. TUI의 `TerminalGuard`가 종료 시 터미널 복원 |
| `#[cfg(test)] mod tests` | 같은 파일 안의 단위 테스트. `cargo test`로 실행 |

---

## 8. 테스트·검증 한눈에
- `cargo build` 빌드 / `cargo test` 단위 테스트 / `cargo clippy --all-targets` 린트(경고 0 유지).
- 핵심 로직은 `FakeRuntime` + 인메모리 DB(`open_memory`)로 외부 의존 없이 테스트.
- 전체 흐름 시연: `LUIDA_FAKE=1` + 임시 경로 env로 격리 ([operations.md](operations.md) 데모 섹션).
