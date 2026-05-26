# Luida — Operations Guide

| | |
|---|---|
| **대상** | 처음 Luida를 띄우는 사용자 (= roto 본인) |
| **환경** | macOS · cmux 0.63+ · Bun 1.3+ · worktrunk(`wt`) · `gh` CLI |
| **버전** | v0.1 (Phase 0~5 + Web Track A 완료 시점) |

---

## 1. 처음 설치

### 1.1 의존성 확인
```bash
bun --version       # 1.3+
cmux --version      # 0.63+
wt --version        # worktrunk
gh --version        # GitHub CLI (PR 생성용, gh auth login 완료 상태)
claude --version    # Claude Code 2.1.139+ (--session-id, /goal 지원)
```

### 1.2 프로젝트 클론 + 의존성 설치
```bash
cd /Users/roto/workspace/luida
bun install
bun run typecheck                # 0 error 확인
bun test                         # 전체 grean 확인
```

### 1.3 tavern.db 초기화
```bash
luida db init
# 출력 예:
#   🏮 루이다의 술집을 준비했어요.
#      DB: /Users/roto/.luida/tavern.db
#      새로 적용: 0001_init.sql, 0002_quest_source_inmail.sql
```

런타임 데이터는 `~/.luida/`에 생성됩니다:
```
~/.luida/
├── tavern.db                      # SQLite (WAL)
├── tavern.db-wal
├── tavern.db-shm
├── memory/
│   ├── chronicle.md               # 일지 (자동 rotation: 2MB 초과 시 월 아카이브)
│   ├── projects/<name>.md
│   └── patterns/YYYY-MM-DD-*.md
└── relationships.yaml             # 자동화 룰 (선택)
```

---

## 2. cmux pane별 sidecar 띄우기

각 프로젝트 cmux pane에서 Claude를 띄우기 전에 sidecar를 백그라운드로 시작합니다.

### 2.1 표준 시작 패턴 (pane 첫 명령)
```bash
# 예: agora pane
SESSION_NAME=agora \
  nohup luida sidecar --me "$SESSION_NAME" \
  > ~/.luida/log/$SESSION_NAME.log 2>&1 &
exec claude
```

핵심:
- `--me <name>`이 모험가 이름. `agora`, `admin`, `kontrol` 등 프로젝트별 고유.
- cmux는 `CMUX_WORKSPACE_ID` / `CMUX_SURFACE_ID` 환경변수를 자동 주입 → sidecar가 이를 읽어 `adventurers` 테이블에 자기 등록.
- `--auto-pr` 옵션을 추가하면 worker 작업 완료 후 PR 자동 생성. 안 주면 `needs_approval` 상태에서 멈춤 (사용자 승인 게이트).
- 로그는 `~/.luida/log/<name>.log`로 누적. `tail -f`로 모니터링.

### 2.2 wrapper 스크립트 사용 (권장)
`scripts/cmux-pane.sh`를 이용해 한 줄로:
```bash
~/workspace/luida/scripts/cmux-pane.sh agora
```

---

## 3. 메인(루이다) pane — MCP 통합

`packages/cli/src/index.ts`가 `luida mcp start`를 노출합니다. Claude Code의 MCP 설정에 등록하면 main pane Claude가 quest/adventurer/memory 도구를 즉시 호출할 수 있어요.

### 3.1 Claude Code 프로젝트 MCP 설정
프로젝트 루트(또는 `~/.claude/`)에 `.mcp.json`:
```json
{
  "mcpServers": {
    "luida": {
      "command": "bun",
      "args": ["run", "/Users/roto/workspace/luida/packages/cli/src/index.ts", "mcp", "start"],
      "env": {}
    }
  }
}
```

### 3.2 등록 확인 — main pane Claude에서
```
/mcp list
```
`luida`가 보이면 OK. 사용 가능 도구:
- `quest.list` / `quest.get` / `quest.dispatch`
- `adventurer.list`
- `memory.recall` / `memory.record`

### 3.3 첫 의뢰 발급
main pane에서:
> "agora에게 schema 마이그레이션 의뢰 보내줘"

Claude가 `quest.dispatch({to: 'agora', brief: '...'})`를 호출하면 dispatch inmail이 들어가고, agora pane의 sidecar가 10초 안에 받아 prompt 주입 → worker 실행.

---

## 4. brain daemon

학습 + stuck quest 감지를 위해 brain을 한 번만 띄웁니다 (전체에 1 인스턴스).

### 4.1 수동 (개발용)
```bash
luida brain start &
# 또는 cmux pane 1개를 brain 전용으로 두기:
luida brain start
```

### 4.2 launchd 자동 시작 (운영)
`~/Library/LaunchAgents/com.daangn.luida-brain.plist`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.daangn.luida-brain</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/roto/.bun/bin/bun</string>
    <string>run</string>
    <string>/Users/roto/workspace/luida/packages/cli/src/index.ts</string>
    <string>brain</string>
    <string>start</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/Users/roto/.luida/log/brain.log</string>
  <key>StandardErrorPath</key><string>/Users/roto/.luida/log/brain.err.log</string>
</dict>
</plist>
```
로드:
```bash
launchctl load ~/Library/LaunchAgents/com.daangn.luida-brain.plist
```

### 4.3 검증
```bash
luida brain reflect          # 1회 즉시 reflect
sqlite3 ~/.luida/tavern.db   # adventurers에 luida-brain row 있는지
  > SELECT name, role, status, last_seen FROM adventurers;
```

---

## 5. 자동화 룰 (relationships.yaml)

### 5.1 룰 작성
`~/.luida/relationships.yaml`:
```yaml
relationships:
  - name: agora-schema-to-admin
    from: agora
    trigger:
      kind: path_changed
      paths:
        - "prisma/**"
        - "schema/**"
    to: admin
    action: auto_dispatch
    brief_template: "agora schema 변경 ({files})을 admin codegen에 반영"
    enabled: true
```

### 5.2 동기화
```bash
luida sync-rules ~/.luida/relationships.yaml
# 출력: 📜 룰 동기화 — 신규: 1 · 갱신: 0 · 실패: 0
```

### 5.3 학습 패턴 확인 → 승급
brain이 자동 발견한 후보:
```bash
luida brain reflect
# 출력: 🧠 reflect — 후보 N건, markdown N건, proposal N건
#       • luida-to-agora (7.0/10, 5건)

luida promote-pattern luida-to-agora --activate
# 출력: 📜 패턴 승급 — luida-to-agora → relationship #2 · 활성 (auto_dispatch)
```

---

## 6. TUI 대시보드

```bash
luida ui
```
- `q` 종료
- `Tab` / `Shift+Tab` 패널 전환
- `j` / `k` 또는 화살표 항목 이동

---

## 7. Web 대시보드 (beta)

```bash
luida web --port 4321
```
브라우저에서 `http://127.0.0.1:4321` 접속.

추후 Tauri 래퍼(`Luida.app`)로 배포 예정 — Web Track B에서.

---

## 8. dry-run 시나리오 (첫 검증)

실제 cmux pane을 띄우기 전에 단일 머신에서 CLI만으로 전체 흐름 검증:

### 8.1 모험가 시드 + dispatch
```bash
# DB 초기화
luida db init

# 모험가 수동 등록 (sidecar 없이)
sqlite3 ~/.luida/tavern.db <<SQL
INSERT INTO adventurers (name, workspace_id, surface_id, role, status, last_seen, registered_at)
VALUES
  ('luida', 'w', 's', 'main', 'idle', strftime('%s','now')*1000, strftime('%s','now')*1000),
  ('agora', 'w', 's', 'worker', 'idle', strftime('%s','now')*1000, strftime('%s','now')*1000),
  ('admin', 'w', 's', 'worker', 'idle', strftime('%s','now')*1000, strftime('%s','now')*1000);
SQL

# 가상 dispatch
sqlite3 ~/.luida/tavern.db <<SQL
INSERT INTO inmail (from_session, to_session, kind, payload, created_at)
VALUES ('luida', 'agora', 'dispatch',
  json_object('brief', '테스트 의뢰', 'branch', 'feat/test'),
  strftime('%s','now')*1000);
SQL

# brain reflect 후 dashboard 확인
luida brain reflect
luida ui
# 의뢰 #1이 agora에게 pending으로 보임
```

### 8.2 Web 확인
```bash
luida web --port 4321 &
curl -s http://127.0.0.1:4321/api/snapshot | head -c 200
```

---

## 9. 알려진 함정

- **cmux #1472**: 프로그램이 만든(=새로 spawn한) workspace는 PTY가 죽어서 `cmux send-key`가 실패해요. **사용자가 GUI에서 직접 띄운 cmux pane**에서만 sidecar 사용.
- **`wt c` alias**: 자동으로 `claude` REPL을 띄움. sidecar의 headless worker는 `wt switch --create --execute :` (no-op)로 우회. 사용자 직접 사용 시는 `wt c "<name>"` 권장.
- **`gh pr create`**: `--head` 명시되지 않으면 worktree branch 추측 실패 가능. sidecar는 `wt.branch`를 항상 명시.
- **headless worker 안전성**: 현재 `--dangerously-skip-permissions` 가정. 격리 강화는 Phase D(권한 모델)에서 본격화.
- **brain daemon 1개만**: 다중 인스턴스는 lastReflectAt 공유 안 함. launchd로 단일 인스턴스 보장 권장.

---

## 10. 트러블슈팅

| 증상 | 원인 | 해결 |
|---|---|---|
| `cmux send-key` exit 1 | surface_id가 stale (cmux 재시작) | sidecar 재기동 → env 재읽기 |
| 같은 inmail 두 번 처리 | source_inmail_id UNIQUE 위반 — 사실은 정상, dedupe 됨 | quest insertIdempotent가 기존 row 반환 |
| worker가 hang | claude CLI 자체 hang 또는 stdin 대기 | `pkill -f 'claude -p'` 후 quest를 failed로 마킹 |
| TUI 색상 깨짐 | terminal COLORTERM != truecolor | cmux/Ghostty는 항상 OK. 다른 환경은 미지원 |
| `gh pr create` 실패 | gh auth 미완 또는 default repo 설정 부재 | `gh auth login` + `gh repo set-default` |

---

## 11. 다음 작업 (Phase 6+ 후보)
- **C** 디렉터리 rename(`luida`) + git init
- **B** Web Track B — Vite + TSX + Tauri shim
- **D** PreToolUse hook으로 worktree 밖 접근 차단
- **E** Zod schema validation 통일

자세한 로드맵: `docs/implementation-plan.md`, 각 `docs/reviews/phase-N.md`
