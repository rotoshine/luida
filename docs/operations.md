# Luida — Operations Guide

| | |
|---|---|
| **대상** | 처음 Luida를 띄우는 사용자 (= roto 본인) |
| **버전** | v2 (Rust · Cargo) — ADR-0001 Accepted |
| **환경** | macOS · Rust toolchain · (실 모드) `claude`/`codex` CLI · worktrunk(`wt`) · `gh` CLI |

> v1(TypeScript/Bun) 운영 가이드는 `git tag v1-typescript` 시점의 이 파일을 참고하세요. 아래는 전부 v2(Rust) 기준입니다.

---

## 1. 설치 · 빌드

### 1.1 의존성
```bash
cargo --version     # Rust toolchain (rustup)
# 아래는 "실 모드"(외부 LLM 실제 호출)에서만 필요 — 데모 모드는 불필요:
claude --version    # Claude Code (claude 런타임)
codex --version     # Codex CLI (codex 런타임, 선택)
wt --version        # worktrunk — quest용 worktree 생성
gh --version        # PR 생성 등 (선택)
```

### 1.2 빌드
```bash
cd /Users/roto/workspace/luida
cargo build --release      # target/release/luida
cargo test                 # 전체 그린 확인
cargo clippy --all-targets # 0 warning 확인
```
`target/release/luida` 를 PATH에 두거나(`ln -s`) `cargo run --` 로 호출합니다. 이하 `luida` 는 이 바이너리.

### 1.3 초기화
```bash
luida db init        # ~/.luida/tavern.db 생성 + 마이그레이션
luida agents init    # ~/.luida/agents.json 생성 (있으면 유지)
```

런타임 데이터 경로 (env로 override 가능):

| | 기본 | override |
|---|---|---|
| tavern.db | `~/.luida/tavern.db` | `LUIDA_DB_PATH` |
| agents.json | `~/.luida/agents.json` | `LUIDA_AGENTS_PATH` |
| memory vault | `~/.luida/memory` | `LUIDA_MEMORY_DIR` |

```
~/.luida/
├── tavern.db                       # SQLite (WAL) — 모든 상태의 단일 진실
├── agents.json                     # 런타임/모델/행위 매핑
└── memory/                         # 모험의 서 (Obsidian 호환 vault)
    ├── chronicle.md                # 일지 (롤링 기록)
    ├── projects/<name>.md          # 프로젝트 맥락 (project ingest)
    └── campaigns/<id>-<slug>.md    # 원정 보고서 (campaign report)
```

---

## 2. `LUIDA_FAKE` 데모 — 외부 LLM 없이 전체 파이프라인

`LUIDA_FAKE=1` 이면 `claude`/`codex` 호출과 실제 git worktree 없이 **결정적 fake 런타임**으로 plan→run→report→reflect 를 끝까지 돌립니다. CI·시연·동작 확인용. 위 3개 경로 env를 임시 디렉터리로 잡으면 실제 `~/.luida` 데이터를 건드리지 않고 완전히 격리됩니다.

```bash
DEMO=$(mktemp -d /tmp/luida-demo.XXXXXX)
export LUIDA_FAKE=1
export LUIDA_DB_PATH="$DEMO/tavern.db"
export LUIDA_AGENTS_PATH="$DEMO/agents.json"
export LUIDA_MEMORY_DIR="$DEMO/memory"

luida db init
luida agents init
luida project add agora --path "$DEMO/agora" --desc "거래 서비스"
luida project add admin --path "$DEMO/admin" --desc "관리 콘솔"
luida project ingest agora           # → memory/projects/agora.md

luida campaign plan "agora와 admin에 통합 검색 필터 추가"
#   fake 플래너가 프롬프트의 "등록된 모험지: agora, admin" 을 파싱해
#   q1(admin) → q2(agora) 의존성 체인 quest DAG 생성
luida campaign run 1                  # 완료 2 / 대기 0 / 실패 0
luida campaign report 1               # → memory/campaigns/0001-*.md
luida reflect                         # 관계 제안 demo-link (비활성) 학습
luida relationship list
luida relationship enable demo-link   # 비활성 → 활성

# 정리
rm -rf "$DEMO"
```

### 동작 메커니즘
- `LUIDA_FAKE` 판정: `crates/luida-cli/src/main.rs` 의 `is_fake()` → `make_factory()`(런타임)·`make_worktree()`(temp 디렉터리) 분기.
- fake 이벤트: `crates/luida-runtimes/src/fake.rs` 의 `canned_events(action, prompt)`.
  - `campaign.plan`·`learning.reflect` 는 프롬프트의 `등록된 모험지:` 줄을 파싱해 **실제 등록 프로젝트**로 계획·관계 제안 생성.
  - `quest.execute`·`campaign.report`·`project.ingest`·`escalation.triage` 는 그럴듯한 canned 결과.

### 생성 산출물 (vault)
- `memory/projects/agora.md` — 프로젝트 맥락 요약
- `memory/campaigns/0001-*.md` — 원정 보고서 (frontmatter + 본문)
- `memory/chronicle.md` — 일지 누적

---

## 3. 실 모드 — 실제 에이전트로 원정 수행

`LUIDA_FAKE` 없이 실행하면 `agents.json` 의 런타임(claude/codex)으로 실제 작업을 수행합니다.

### 3.1 agents.json
행위(action) → 런타임/모델/모드 매핑. 예시는 [`docs/examples/agents.json`](examples/agents.json).
```bash
luida agents show                       # 현재 설정 요약
luida agents resolve campaign.plan      # 특정 행위가 어떤 런타임/모델로 해소되는지
luida agents resolve quest.execute --project agora   # 프로젝트별 override 포함
```
- `defaults.runtime`/`defaults.tier`, `runtimes.<name>`(kind·command·models·enabled), `actions.<action>`, `projectOverrides.<project>.<action>` 로 구성.
- `agents resolve` 의 "사용가능" 행이 `아니오` 면 해당 CLI(claude/codex) 미설치.

### 3.2 모험지 등록 → 원정
```bash
luida project add agora --path ~/workspace/agora --base main --desc "거래 서비스"
luida project list

luida campaign plan "agora에 검색 필터 기능 추가"   # LLM 플래너 → quest DAG
luida campaign run 1
#   각 quest 를 worktrunk worktree 에 worker 로 디스패치(의존성 순).
#   escalation 발생 시 triage → 자동 해소 가능하면 자동 재개, 아니면 사용자 대기.
luida campaign report 1                              # 완료 후 모험의 서 기록
```

### 3.3 escalation 대응
원정 실행 중 worker가 판단을 요청(needs_input)하면:
```bash
luida campaign list                 # 진행 상황
luida quest triage <id>             # 자동 해소 가능 여부 분류
luida quest resume <id> "<답변>"    # 사용자 답변으로 재개
```

### 3.4 프로젝트 간 자동화 관계
```bash
luida reflect                          # 최근 이벤트 분석 → 관계 제안(비활성 저장)
luida relationship list                # 활성·비활성 전체
luida relationship enable <name>       # 학습 제안 승인 → 활성화
luida relationship disable <name>
```
활성 관계는 quest 완료 시 평가되어 `auto_dispatch`(같은 원정에 후속 quest 추가) 또는 `propose`(사용자에게 제안)로 이어집니다. 관계 스키마 예시: [`docs/examples/relationships.yaml`](examples/relationships.yaml).

### 3.5 모험 중단 · 재개 (기기 간 핸드오프)
```bash
luida adventure suspend <id> --out handoff.json   # 원정 봉인 → JSON
luida adventure resume --from handoff.json        # 다른 기기에서 이어받기
```

---

## 4. 대시보드

### 4.1 TUI
```bash
luida ui
```
- 탭: 모험지(Projects) / 원정(Campaigns) / 모험(Quests) — 상단에 escalation 대기 카운트, 실행 중 진행 바
- **대시보드는 1.2초마다 자동 갱신**되어 백그라운드(server/daemon/다른 프로세스)의 변경을 실시간 반영(선택 항목은 정체성으로 보존).

| 키 | 동작 |
|---|---|
| `Tab`/`Shift+Tab` · `→`/`←` | 탭 전환(정/역방향) — 십자키만으로도 이동(모바일 SSH) |
| `j`/`k` · `↑`/`↓` | 항목 이동 |
| `Enter`/`d` | 선택 항목 상세(타임라인+메타데이터) 토글 |
| `PgUp`/`PgDn`·`Home`·`End` | 상세 스크롤 / 맨 위(Home) / 꼬리추적(tail) 복귀(End) |
| `x` | 선택 원정 실행 |
| `p` | 새 원정 계획(프롬프트 입력) |
| `a` | 모험지 등록(`이름 경로 [브랜치] [설명]`) |
| `c` | 완료 원정 보고(모험의 서) |
| `r` | 모험 재개(답변 입력) · `t` escalation triage |
| `n` | 다음 판단대기 모험으로 점프 |
| `?` | 키 도움말 오버레이 · `q`/`Esc` 종료 |

- 한글 IME 켠 상태에서도 같은 물리 키로 동작(`q=ㅂ p=ㅔ a=ㅁ c=ㅊ` 등). 입력 모달은 `Shift/Alt+Enter` 개행.

#### 중단 후 이어받기 (고아 프로세스 방지)
- 모험 실행 중 `q`/`Esc`/`Ctrl-C` → 실행 중인 에이전트(외부 CLI 자식 프로세스)를 **즉시 정리**하고 종료한다. 고아 프로세스가 남지 않는다.
- 중단된 모험은 `pending`(이어받기 가능)으로 되돌아가고 `quest_interrupted` 이벤트가 남는다. 원정은 active 로 유지된다.
- **재시작하면** 원정 탭에서 `x`(실행)로 이어받는다 — 기존 worktree + `claude --resume` 로 직전 세션을 이어간다.
- 강제 종료(SIGKILL)로 `running` 상태가 남은 경우, **다음 실행 시 자동 재조정**되어 중단 처리된다(이 머신의 죽은 runner 한정 — 다른 터미널에서 도는 `luida campaign run` 은 PID 생존 확인으로 건드리지 않음).

### 4.2 Web / GUI 브리지 (HTTP·SSE)
```bash
luida server start --port 4321            # 기본 127.0.0.1 (로컬 전용)
curl -s http://127.0.0.1:4321/api/health
curl -s http://127.0.0.1:4321/api/snapshot | head -c 200
```
- 내장 웹 대시보드: `GET /`(반응형 HTML — 모바일 세로 1열). `GET /api/snapshot` 초기 상태, `GET /api/stream` **SSE 라이브 갱신**, 명령 API: `POST /api/projects`, `/api/campaigns/plan`, `/api/campaigns/{id}/run`, `/api/quests/{id}/resume|triage` (ADR-0002).
- (참고) 별도 프론트엔드(Vite/React)·Tauri 래퍼 초안은 레거시 [`packages/web`](../packages/web/README.md).

#### 원격 서버 + Tailscale + 모바일 브라우저
모바일에서 TUI 대신 브라우저로 보고 싶을 때. 서버를 Tailscale 인터페이스에 노출한다.
```bash
# 원격 서버에서: 모든 인터페이스 바인드 (Tailscale IP 로도 접속 가능)
luida server start --host 0.0.0.0 --port 4321
# 모바일(같은 tailnet): 브라우저로 http://<원격-tailscale-IP>:4321
```
- ⚠️ `--host` 가 루프백이 아니면 **명령 API(plan/run 등)가 네트워크에 노출**된다(인증 없음). 반드시 **Tailscale 같은 신뢰 네트워크에서만** 쓸 것 — 공인망/사무실 LAN 바인드 금지. 서버가 비루프백 바인드 시 경고를 출력한다.
- 더 좁히려면 `--host <tailscale-IP>`(예 `100.x.y.z`)로 그 인터페이스에만 바인드. 백그라운드 상주는 `nohup`/`systemd --user`/`tmux` 등으로.

---

## 5. 트러블슈팅

| 증상 | 원인 | 해결 |
|---|---|---|
| `cargo build` 실패: unresolved import | crate 의존성 누락 | 해당 `crates/*/Cargo.toml` 에 `*.workspace = true` 추가 |
| `agents resolve` 사용가능=아니오 | claude/codex CLI 미설치 | 런타임 CLI 설치 또는 데모 모드(`LUIDA_FAKE=1`) 사용 |
| `campaign plan` 실패: "등록된 모험지가 없습니다" | project 미등록 | `luida project add` 먼저 |
| worktree 생성 실패 | `wt`(worktrunk) 미설치/경로 오류 | `wt --version` 확인, repo 경로 점검 (데모는 temp dir 사용) |
| TUI 색상 깨짐 | 터미널 truecolor 미지원 | cmux/Ghostty 권장 |

---

## 6. 다음 작업 (로드맵)
- v2-standalone.md §8 의 미완 Phase: V2-P6(PTY 직접 관리), V2-P7(xterm.js 인터랙티브), V2-P9(Tauri 패키징)
- 트리거 확장: `path_changed`·`tag_pushed` (git watcher) — 현재 `quest_completed` 만 구현
- 관계 사이클 가드 (무한 연쇄 방지)

자세한 설계: [`docs/v2-standalone.md`](v2-standalone.md), Phase별 기록: [`docs/reviews/v2-p*.md`](reviews/).
