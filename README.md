# 🍺 Luida

> 루이다의 주점 — cmux pane 위에서 Claude Code 세션들을 오케스트레이션하는 멀티 에이전트 시스템

여러 cmux pane에 띄운 Claude 세션을 **모험가**로 등록하고, 메인 세션(루이다)이 의뢰(quest)를 발급·라우팅·학습하는 도구예요. 컨셉은 드래곤퀘스트 3의 「루이다의 주점」.

```
┌─ cmux pane: main (luida) ──┐    ┌─ cmux pane: agora ─────┐
│  Claude REPL                │    │  Claude REPL              │
│   ← MCP tools                │    │   ← sidecar inject       │
│                              │    │                          │
│  main sidecar               │    │  agora sidecar           │
└──────┬─────────────────────┘    └──────┬───────────────────┘
       │                                  │
       ▼  inmail / quests / events        ▼
  ┌─────────────────────────────────────────────┐
  │      tavern.db  (SQLite, WAL)                │
  │      ~/.luida/tavern.db                      │
  └────────────────────┬────────────────────────┘
                       │
                ┌──────┴──────────┐
                │  brain daemon   │  ← stuck quest 감지, 학습 reflect
                └─────────────────┘
```

## 빠른 시작

```bash
bun install
bun test            # 182 pass / 0 fail
bun run typecheck   # 0 error

luida db init       # tavern.db 초기화
luida ui            # TUI 대시보드
luida web           # Web 대시보드 (http://127.0.0.1:4321)
luida brain start & # 학습 데몬
```

자세한 운영 가이드: [`docs/operations.md`](docs/operations.md)

## 패키지 구조

| 패키지 | 역할 |
|---|---|
| `@luida/core` | tavern.db 스키마·타입·repo, integrations 인터페이스, glob/yaml |
| `@luida/sidecar` | cmux pane별 데몬 (polling, worker spawn, PR 생성) |
| `@luida/brain` | headless daemon (학습, stuck 감지, 패턴 승급, memory) |
| `@luida/mcp` | JSON-RPC stdio MCP server (quest.*, adventurer.*, memory.*) |
| `@luida/ui` | Ink 기반 TUI (`luida ui`) |
| `@luida/web` | Bun.serve + 디자인 정적 prototype (`luida web`) |
| `@luida/cli` | 단일 진입점 `luida` |

## CLI 명령

```
luida db init
luida sidecar --me <name> [--auto-pr]
luida ui
luida web [--port 4321]
luida brain start
luida brain reflect
luida promote-pattern <id> [--activate]
luida sync-rules <yaml-file>
luida mcp start
```

## 문서

- [`docs/implementation-plan.md`](docs/implementation-plan.md) — Phase 0~5 로드맵
- [`docs/operations.md`](docs/operations.md) — 운영·dry-run 가이드
- [`docs/web-design-spec.md`](docs/web-design-spec.md) — Web/Tauri 디자인 스펙
- [`docs/reviews/phase-*.md`](docs/reviews/) — 각 Phase 셀프 리뷰
- [`docs/examples/relationships.yaml`](docs/examples/relationships.yaml) — 자동화 룰 예시

## 라이선스

private (당근마켓 내부 도구)
