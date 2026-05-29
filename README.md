# 🍺 Luida

> 루이다의 주점 — 여러 Claude/Codex 세션을 **모험가**로 등록하고, 프로젝트를 가로지르는 작업(quest)을 계획·실행·학습하는 멀티 에이전트 오케스트레이터. (v2 · Rust)

컨셉은 드래곤퀘스트 3의 「루이다의 주점」. 사용자(루이다)가 원정(campaign)을 의뢰하면 등록된 모험지(프로젝트)들에 걸친 quest DAG로 분해되어 의존성 순으로 실행되고, 완료된 작업은 모험의 서(memory vault)에 기록되며, 반복 패턴은 프로젝트 간 자동화 관계로 학습됩니다.

> v1(TypeScript)에서 v2(Rust)로 전면 재작성됐습니다(ADR-0001). v1은 `git tag v1-typescript`로 보존. **설계 정본은 [`docs/v2-standalone.md`](docs/v2-standalone.md).**

## 핵심 흐름

```
luida campaign plan "agora와 admin에 검색 필터 추가"
        │   planner: LLM → quest DAG (의존성 위상정렬)
        ▼
luida campaign run <id>
        │   각 quest 를 worktree 에 worker(claude -p 등)로 디스패치
        │   escalation 발생 시 triage → 자동 해소 또는 사용자 대기
        │   quest 완료 시 관계 트리거 평가:
        │     └ auto_dispatch → 같은 원정에 후속 quest(교차 프로젝트) 추가
        ▼
luida campaign report <id>      → 모험의 서(vault) 기록
luida reflect                   → 이벤트 분석 → 프로젝트 관계 제안(학습)

모든 상태의 단일 진실: ~/.luida/tavern.db  (SQLite · WAL)
```

## 빠른 시작

```bash
# 빌드 · 검증
cargo build --release
cargo test
cargo clippy --all-targets

# 초기화
luida db init        # ~/.luida/tavern.db 생성 + 마이그레이션
luida agents init    # ~/.luida/agents.json (claude/codex 런타임·행위 매핑)

# 모험지 등록 → 원정
luida project add agora --path /path/to/agora --desc "거래 서비스"
luida campaign plan "agora에 검색 필터 추가"
luida campaign run 1
luida campaign report 1

# 대시보드
luida ui                        # TUI
luida server start --port 4321  # HTTP/SSE (웹/GUI 브리지)
```

> `luida` 바이너리는 `cargo build` 후 `target/{debug,release}/luida` 에 생깁니다. PATH에 넣거나 `cargo run -- <명령>` 으로 실행하세요.

## 데모 (외부 LLM 없이 전체 파이프라인)

`LUIDA_FAKE=1` 이면 외부 LLM·repo 없이 결정적 fake 런타임으로 plan→run→report→reflect 전체를 시연·CI할 수 있습니다. 임시 경로로 완전히 격리:

```bash
export LUIDA_FAKE=1
export LUIDA_DB_PATH=/tmp/luida-demo/tavern.db
export LUIDA_AGENTS_PATH=/tmp/luida-demo/agents.json
export LUIDA_MEMORY_DIR=/tmp/luida-demo/memory

luida db init && luida agents init
luida project add agora --path /tmp/agora
luida project add admin --path /tmp/admin
luida campaign plan "agora와 admin 정렬"   # "등록된 모험지:" 파싱 → quest DAG
luida campaign run 1                         # 전부 완료
luida campaign report 1                      # vault 에 보고서 기록
luida reflect                                # 관계 제안(비활성)
luida relationship list
```

자세한 절차·산출물: [`docs/operations.md`](docs/operations.md).

## crate 구조

| crate | 역할 |
|---|---|
| `luida-core` | tavern.db 스키마·SQLite·repository + agents.json 해소(resolver) |
| `luida-brain` | 모험의 서(Obsidian vault): project ingest · campaign report · reflect(관계 학습) · Memory Tree |
| `luida-planner` | 원정 계획·실행: LLM 프롬프트 → quest DAG 파싱·검증·위상정렬 → 의존성 순 디스패치 |
| `luida-runtimes` | 실 런타임(claude/codex CLI) spawn + `FakeRuntime`(데모·테스트용 결정적 런타임) |
| `luida-sidecar` | quest 오케스트레이션: worktree(worktrunk) 생성 · dispatch · escalation triage · 관계 트리거 |
| `luida-server` | Axum HTTP/SSE — `/api/health` · `/api/snapshot` · `/api/stream` · `POST /api/projects` |
| `luida-tui` | ratatui 대시보드 (Projects / Campaigns / Quests 탭) |
| `luida-cli` | 단일 진입점 `luida` |

## CLI 명령

```
luida db init
luida project   add <name> --path <p> [--base main] [--desc <d>] | list | remove <name> | ingest <name>
luida agents    init | resolve <action> [--project <p>] | show
luida campaign  plan <prompt> | run <id> | report <id> | list
luida quest     resume <id> <answer> | triage <id>
luida adventure suspend <id> [--out <f>] [--force] | resume [--from <f>]
luida reflect   [--since-hours 24]
luida relationship list | enable <name> | disable <name>
luida server    start [--port 4321]
luida ui
```

## 설정 · 데이터 경로

| | 기본 | override |
|---|---|---|
| tavern.db | `~/.luida/tavern.db` | `LUIDA_DB_PATH` |
| agents.json | `~/.luida/agents.json` | `LUIDA_AGENTS_PATH` |
| memory vault | `~/.luida/memory` | `LUIDA_MEMORY_DIR` |

설정 예시: [`docs/examples/agents.json`](docs/examples/agents.json) · [`docs/examples/relationships.yaml`](docs/examples/relationships.yaml)

## 문서

- [`docs/v2-standalone.md`](docs/v2-standalone.md) — **v2 정본 설계** (아키텍처 · 행위 분류 · agents.json · Phase 분해)
- [`docs/operations.md`](docs/operations.md) — 운영 · 데모 가이드
- [`docs/adr/`](docs/adr/) — 0001 Rust 채택 · 0002 프론트엔드 브리지
- [`docs/reviews/v2-p*.md`](docs/reviews/) — V2 Phase별 셀프 리뷰
- [`docs/implementation-plan.md`](docs/implementation-plan.md) — ⚠️ v1(TypeScript) 아카이브

## 라이선스

UNLICENSED — 개인 프로젝트 (private)
