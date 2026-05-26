# Luida Web Dashboard — Design Specification

| | |
|---|---|
| **Status** | Draft v0.1 |
| **Owner** | Roto |
| **Last updated** | 2026-05-26 |
| **Audience** | 디자인·구현 담당 Claude 세션, 향후 합류할 디자이너/개발자 |
| **Related** | [Luida System Overview](./system-overview.md) *(예정)* · [Tavern Schema](./schema.md) *(예정)* |

> 이 문서는 Luida 시스템의 **웹 대시보드** 디자인·구현을 위한 단일 진실 소스(SSOT)예요. 자세한 시스템 아키텍처(sidecar, brain, MCP server 등)는 별도 문서를 참조하세요. 이 문서는 "사용자가 브라우저에서 보는 것"만 다룹니다.

---

## 1. 컨텍스트

### 1.1 Luida가 무엇인가
**Luida**는 여러 Claude Code 세션을 오케스트레이션하는 멀티 에이전트 시스템이다. 컨셉은 **드래곤퀘스트 3의 "루이다의 술집"**(ルイダの酒場): 모험가들을 등록·편성해 의뢰(quest)를 보내고, 결과를 모아 보고받는 술집의 메타포로 구성된다.

### 1.2 핵심 엔티티
| 엔티티 | 역할 |
|---|---|
| **메인 에이전트 (Luida)** | 사용자와 대화하며 의뢰를 분배하고, 프로젝트 간 관계를 학습 |
| **모험가 (Adventurer)** | 각 프로젝트(예: `agora`, `admin`, `kontrol`)에 대응하는 Claude 세션. cmux 터미널 multiplexer에서 동작 |
| **의뢰 (Quest)** | 메인이 모험가에게 분배한 코딩 작업. 각 quest는 새 git worktree에서 수행되고 PR로 마무리 |
| **술집 (Tavern)** | SQLite 기반 inmail bus — 모든 통신과 상태의 중심 |
| **연감 (Chronicle)** | 학습된 프로젝트 관계와 패턴 기록 |

### 1.3 데이터 흐름 한 줄
사용자 → 메인 Luida → 의뢰 발급 → 모험가가 worktree에서 작업 → 검토 → PR 생성 → 완료 알림 → 메인이 사용자에게 보고

### 1.4 이 웹 대시보드의 목적
- 운영 중인 모든 quest/모험가/이벤트를 **한눈에 파악**
- 모바일·외부에서도 **현황 확인 + 승인 게이트 처리**
- TUI 대시보드(`luida ui`, Ink 기반)와 **동일 데이터를 다른 표면으로** 제공

---

## 2. 디자인 방향

### 2.1 비주얼 톤
- **드래곤퀘스트 3 메뉴 스타일** — 깊은 네이비 배경 + 흰색 더블 라인 테두리 + 픽셀 폰트
- 또는 **NES 패미컴 인터페이스** 풍 (NES.css를 출발점으로 활용 가능)
- 게임 메뉴 같은 즐거움, 다만 **운영용 대시보드로서 정보 밀도는 충분히 확보**
- **다크 모드 필수** (라이트는 옵션)

### 2.2 참고 비주얼
- DQ3 「ルイダの酒場」메뉴 화면 (검색: "Dragon Quest 3 Luida's Tavern menu")
- DQ 시리즈 전투 메뉴 / 스테이터스 화면
- Final Fantasy 4~6의 메뉴 화면
- [NES.css 갤러리](https://nostalgic-css.github.io/NES.css/)
- [PSone.css](https://psone.style/), [98.css](https://jdan.github.io/98.css/) (대안 톤)

### 2.3 폰트
- **한국어**: [Galmuri](https://galmuri.quiple.dev/) — 한글 픽셀 폰트의 사실상 표준
- **영문/숫자**: DotGothic16 또는 Press Start 2P
- 둘을 섞어 다국어 자연스럽게 표시

### 2.4 색 팔레트 (제안값, 디자이너 재량으로 다듬을 것)

```
Window BG:     #08197B  (DQ 깊은 네이비)
Window Border: #FFFFFF  (두 줄 흰 테두리, 안쪽 1px gap)
Text Primary:  #FFFFFF
Text Dim:      #A8B8E8
Text Gold:     #FCD34D  (헤더·강조)
HP Green:      #4ADE80
HP Yellow:     #FACC15
HP Red:        #EF4444
MP Blue:       #60A5FA
Accent Pink:   #F472B6  (이벤트·알림)
Background:    #000814  (가장 깊은 바깥 배경)
```

토큰은 `packages/web/src/design/tokens.ts`로 추출해 둘 것.

### 2.5 절대 피할 것 ❌
- 일반적인 SaaS 대시보드 느낌 (Tailwind admin template 같은 미니멀 카드 그리드)
- 둥근 모서리(border-radius) 남발 — 픽셀 RPG는 직각이 기본
- 부드러운 그림자, 그라데이션 — 8-bit 정신에 위배
- 모던 아이콘 라이브러리(Lucide, Heroicons) — 이모지·픽셀 아이콘으로 대체

---

## 3. 데이터 모델

UI가 표현해야 할 엔티티. 타입은 `packages/core/src/schema.ts`에서 import해서 사용.

```typescript
// Adventurer (모험가)
type Adventurer = {
  name: string                                     // 'agora' | 'admin' | 'kontrol'
  workspace_id: string                             // cmux workspace
  surface_id: string                               // cmux pane id
  repo_path: string                                // /Users/roto/workspace/community-web-agora
  role: 'main' | 'worker' | 'brain'
  status: 'idle' | 'busy' | 'offline'
  last_seen: number                                // unix ms

  // 표현용 가상 스탯 (UI 레이어에서 계산)
  level: number                                    // 누적 quest 수 환산
  hp: { current: number; max: number }             // status 기반 비율
  class: string                                    // 'Frontend Knight' | 'API Mage' | …
}

// Quest (의뢰서)
type Quest = {
  id: number
  dispatched_by: string                            // 'luida'
  dispatched_to: string                            // 'agora'
  brief: string                                    // 작업 설명 (긴 텍스트)
  branch: string                                   // 'feat/schema-migration'
  worktree_path: string
  status:
    | 'pending' | 'running' | 'reviewing'
    | 'needs_approval' | 'pr_ready'
    | 'completed' | 'failed' | 'aborted'
  progress: string                                 // '5개 파일 중 3개 처리'
  pr_url: string | null
  log_path: string
  parent_quest_id: number | null                   // 연쇄 디스패치 (agora→admin)
  created_at: number
  updated_at: number
}

// Inmail (메시지 이벤트)
type Inmail = {
  id: number
  from_session: string
  to_session: string                               // 또는 '@all'
  kind: 'dispatch' | 'progress' | 'ack' | 'proposal' | 'alert' | 'info'
  payload: object
  created_at: number
  delivered_at: number | null
  handled_at: number | null
}

// Relationship (학습된 자동 룰)
type Relationship = {
  id: number
  name: string                                     // 'agora-schema-to-admin'
  from_session: string
  trigger_kind: 'path_changed' | 'quest_completed' | 'tag_pushed'
  trigger_config: object                           // { paths: ['prisma/**'] }
  to_session: string
  action: 'auto_dispatch' | 'propose'
  source: 'human' | 'learned-promoted'
  confidence: number | null                        // 0~1, 학습 룰에만
  enabled: boolean
}

// Event (학습용 영속 로그)
type Event = {
  id: number
  quest_id: number | null
  actor: string
  kind:
    | 'quest_dispatched' | 'tool_used' | 'pr_created'
    | 'review_passed' | 'review_failed' | 'conflict'
    | 'user_approved' | 'user_rejected' | 'pattern_proposed'
  payload: object
  occurred_at: number
}
```

---

## 4. 화면 명세

### 4.1 메인 대시보드 `/`
한 화면에 술집 전경. 4개 패널 그리드 (데스크탑 기준):

| 위치 | 패널 |
|---|---|
| 좌상 | 모험가 패널 — 등록된 모험가 카드 리스트 |
| 우상 | 의뢰 게시판 — 활성 quest 리스트 |
| 좌하 | 술집 게시판 — 최근 이벤트 라이브 피드 |
| 우하 | 연감 위젯 — 학습 패턴/제안 카드 |

**모험가 패널**
- 카드 1장: 이름, class, level, HP bar, 현재 status, 진행 중 quest id
- 클릭 → adventurer 상세

**의뢰 게시판**
- 항목 1줄: id, 담당자, brief 1줄 요약, status 뱃지, progress bar
- 클릭 → quest 상세 모달
- 상단 필터: 전체 / 진행중 / 승인 필요 / 완료

**술집 게시판**
- 시각 + 아이콘 + 한 줄 요약
- DQ 메시지창 스타일 (테두리 박스 + 단색 배경)
- 자동 스크롤 (SSE로 신규 항목 푸시)

**연감 위젯**
- "💡 새 패턴 후보: agora schema → admin codegen (신뢰도 7/10)" 같은 카드
- 사용자 승인/거절 버튼

### 4.2 모험가 상세 `/adventurer/:name`
- 스테이터스 화면 (DQ 캐릭터 스테이터스 메뉴 풍)
- 좌측: 캐릭터 아바타(픽셀 아이콘) + 기본 정보
- 우측: 누적 quest 통계, 평균 처리 시간, 성공률, 최근 PR 목록
- 하단: 이 모험가가 가진 관계(relationships) 목록

### 4.3 의뢰 상세 `/quest/:id`
- 의뢰서 양피지 느낌 (테두리 장식)
- brief, branch, worktree 경로, 상태, PR 링크
- 진행 로그 (stream-json 파싱 결과를 사람이 읽기 좋게 변환)
- **needs_approval 상태**일 때 큰 [승인] / [거절] 버튼
- parent_quest_id가 있으면 "연쇄 의뢰" 트리 표시

### 4.4 술집 로그 `/tavern`
- inmail 전체 흐름 (시간 역순 무한 스크롤)
- 필터: from/to, kind, 기간
- 메시지 1건 클릭 → JSON payload 펼침

### 4.5 관계 그래프 `/relationships`
- 모험가들 간 관계를 노드-엣지 그래프로
- 인간 정의 룰(파란 엣지) vs 학습 승격 룰(주황 엣지) 시각 구분
- 클릭 → 룰 상세 + enable/disable 토글

### 4.6 연감 `/chronicle`
- markdown 기반 chronicle 렌더링 (시간 역순)
- 학습 패턴 목록 + 승급 후보 카드
- "💡 패턴 발견" 카드: 신뢰도 게이지, 근거 이벤트 링크, **[룰로 승격]** 버튼

### 4.7 설정 `/settings`
- adventurer 등록/제거
- 자동화 강도 슬라이더 (수동 → 제안 → 자동)
- 알림 채널 (브라우저 푸시 / Slack webhook)
- 폰트, 색 테마

---

## 5. 인터랙션 · 동작

### 5.1 실시간 갱신
- **Server-Sent Events** 또는 WebSocket으로 새 이벤트 즉시 반영
- 폴링 fallback: 5초마다
- 새 quest 완료 시 우상단 토스트 (DQ 메시지창 스타일, 글자 한 자씩 떨어지는 효과 옵션)

### 5.2 키보드 우선
- `j/k`로 항목 이동, `Enter`로 선택 (DQ 메뉴 조작감)
- `Cmd+K` 커맨드 팔레트: "agora에 의뢰" 같은 자연어로 액션 호출
- vim-like 화면 이동: `g d` 대시보드, `g q` 의뢰, `g a` 모험가

### 5.3 모바일
- 외출 중 폰으로 확인 가능해야 함 → **반응형 필수**
- 모바일에서는 세로 스택, 패널이 탭으로 전환
- 푸시 알림(Web Push API): `needs_approval` / `completed` / `failed`만 대상

### 5.4 사운드 (옵션, 끄기 가능)
- 새 quest 알림: 짧은 DQ 레벨업 풍 효과음
- `needs_approval`: 도착 알림 효과음
- 설정에서 토글 가능, 기본 OFF

---

## 6. 기술 스택

### 6.1 프런트엔드
- **Vite + React + TypeScript**
- **Tanstack Router** (파일 기반 라우팅)
- **Tanstack Query** (서버 상태)
- **vanilla-extract** 또는 **CSS Modules** (Tailwind는 픽셀 UI 표현에 비효율)
- **NES.css**를 출발점으로 차용하되, 부족한 부분은 직접 컴포넌트 작성
- **SSE**(`EventSource`) 우선, 필요시 WebSocket

### 6.2 백엔드
- **Hono** (Bun 위에서 동작, 가벼움)
- **bun:sqlite** 또는 **better-sqlite3** — `~/.luida/tavern.db` 공유
- REST:
  - `GET /api/quests`, `GET /api/quests/:id`
  - `GET /api/adventurers`, `GET /api/adventurers/:name`
  - `POST /api/quests/:id/approve`, `POST /api/quests/:id/reject`
  - `GET /api/relationships`, `PATCH /api/relationships/:id`
  - `GET /api/chronicle`, `POST /api/chronicle/patterns/:id/promote`
- SSE: `GET /api/stream` — 새 inmail/event 푸시

### 6.3 배포·실행 가정 — **Tauri 데스크탑 앱으로 래핑**
- 브라우저에서 `localhost:4321` 접속하는 형태가 아니라 **Tauri로 네이티브 macOS 앱**으로 패키징한다.
  - 사용자가 `Luida.app`을 실행하면 자체 윈도우가 열리고 그 안에 React 대시보드 렌더
  - 브라우저 탭과 분리된 독립 윈도우라 cmd+tab으로 빠르게 전환 가능 + 도크 아이콘 + 알림 권한
- **개발 모드**: `bun run dev`(Vite, 포트 4321) + `tauri dev`로 핫리로드 가능
- **빌드**: `bun run build` → `tauri build`로 `Luida.app` + dmg 생성
- 백엔드(Hono 서버)는 Tauri sidecar로 묶어 같은 프로세스 트리에서 동작하거나, 별도 `luida brain serve` 데몬으로 분리(Phase 4 brain 데몬과 통합 검토)
- **알림**: Tauri의 native notification API 사용 (`needs_approval`, `completed`, `failed`)
- **모바일 접근**: Tauri 데스크탑 앱과 별개로, 동일 React 코드를 PWA 빌드로 함께 산출 → ngrok/Tailscale로 노출 시 폰에서도 사용 가능 (선택)
- **인증**: 로컬 앱이므로 인증 없음. Hono 서버는 `127.0.0.1`만 listen.

---

## 7. 산출물

### 7.1 Phase A: 디자인
1. **무드보드** — DQ3·NES 참고 이미지 모은 페이지 1장 (markdown + 이미지 링크)
2. **디자인 시스템** — `colors.ts`, `typography.ts`, `tokens.ts` 등 토큰 정의
3. **공용 컴포넌트 카탈로그** — Storybook 또는 단일 갤러리 페이지
   - `<Window>` (DQ 더블 라인 테두리 박스)
   - `<DialogBox>` (메시지창, 글자 애니메이션 옵션)
   - `<MenuList>` (▶ 커서 선택 가능 리스트)
   - `<StatusBar>` (HP/MP 바)
   - `<AdventurerCard>`
   - `<QuestRow>` (게시판 1줄)
   - `<EventLogLine>`
   - `<Toast>` (알림)
   - `<Badge>` (status 표시)
4. **메인 대시보드 화면 mockup** (Figma 또는 직접 React 컴포넌트로)

### 7.2 Phase B: 구현
1. Vite 프로젝트 부트스트랩 (`packages/web/`)
2. 위 4.1~4.7 화면 구현
3. 더미 데이터(seed JSON) 기반 동작 데모
4. README에 실행 방법

### 7.3 디렉터리 위치
이 웹 대시보드는 메인 모노레포(`/Users/roto/workspace/luida`)의 **`packages/web/`**에 들어간다. 별도 워크스페이스로 격리하되 `packages/core`의 타입(`Quest`, `Adventurer`, …)은 import해서 쓴다.

#### Tauri 래퍼 구조
```
packages/web/
├── src/                  # React + Vite frontend
├── src-tauri/            # Tauri Rust backend (필요한 IPC만)
│   ├── tauri.conf.json   # Tauri 설정 (윈도우 크기, 메뉴, 알림 권한)
│   ├── Cargo.toml
│   └── src/main.rs
├── public/
├── index.html
├── package.json          # vite, react, @tauri-apps/api
└── vite.config.ts
```

- 가능하면 `src-tauri/src/main.rs`는 **얇게** 유지 (윈도우 띄우기 + 알림 IPC만). DB 접근은 frontend가 Hono 서버 통해서 함 (TUI와 같은 데이터 경로).
- Tauri allowlist는 최소 권한(notification, dialog)만 켠다.

---

## 8. 톤 · UX 가이드

### 8.1 카피 톤 — 술집 NPC 화법
- "🍺 모험가 admin이 의뢰 #142를 완수하고 돌아왔어요"
- "📜 새 의뢰가 게시판에 붙었습니다"
- "⚠ kontrol이 함정에 빠진 듯합니다 (typecheck 실패)"

### 8.2 에러 메시지도 게임처럼
- "💀 모험가가 쓰러졌습니다" (worker crash)
- "🌀 의뢰가 무효화되었습니다" (aborted)

### 8.3 Empty state도 살리기
- 의뢰 없음 → "오늘은 평화로운 하루입니다. 술집이 조용하네요. 🍺"
- 모험가 없음 → "아직 등록된 모험가가 없습니다. `luida adventurer register <name>`으로 모집해주세요."

### 8.4 금기
- 가짜 게이미피케이션(레벨업 알림 남발 등) 금지 — 정보가 거짓이면 도구 신뢰가 무너진다.
- 운영 도구로서 한눈에 상태 파악이 **1순위**, RPG 감성은 2순위. 둘이 충돌하면 운영성 승.

---

## 9. 제약 · 환경

- 사용자 워크플로우는 한국어 중심 — UI 기본 한국어, 영문 toggle 옵션 제공
- **macOS 환경 only** (Windows·Linux 호환은 비관심)
- 인증 없음 (로컬 only). 단 SSE 채널은 origin check 정도는 적용.
- worktree 관련 작업은 항상 **worktrunk(`wt`) 전용**. 표준 명령어 `wt c "<name>"`.
  - 자세한 규약은 사용자 메모리 [`feedback-worktree-use-worktrunk`](../../.claude/projects/-Users-roto-workspace-roto-ai-agent/memory/feedback_worktree_use_worktrunk.md) 참조.

---

## 10. 시작 추천 순서

1. 무드보드 + 색·폰트 토큰 결정
2. `<Window>` 하나만 완성하고 사용자 컨펌
3. 컨펌되면 `<AdventurerCard>`, `<QuestRow>`, `<EventLogLine>` 3개로 메인 대시보드 1차 mock
4. SSE 연동·키보드 인터랙션은 그 후

각 단계 끝마다 스크린샷 1장씩 사용자에게 보여주고 방향 확인 받기.

---

## 11. 구현 현황 (Web Track A 완료)

- ✅ `packages/web/` 신설
- ✅ Claude Design에서 받은 prototype을 `packages/web/static/`에 통합
  - `Luida Tavern.html` + `src/{app,primitives,cards,catalog,dashboard,data,tweaks-panel}.jsx`
  - Babel-standalone로 in-browser JSX 컴파일 (Vite 마이그레이션은 Web Track B에서)
- ✅ `packages/web/src/serve.ts` — Bun.serve 기반 dev 서버
  - `/api/snapshot` — tavern.db 라이브 스냅샷 (adventurers/quests/inmail)
  - `/api/health` — 헬스체크
  - Path traversal 차단 + MIME 타입 매핑
- ✅ CLI: `luida web [--port 4321]`
- ✅ 디자인 결정 반영: "주점" 표기, 정통 블랙 테마 (`#000000` + 흰 더블 라인)
- ✅ smoke test: HTML 200 / JSX 200 / API JSON OK

### Web Track B (다음 단계)
- Vite + React 18 + TSX 마이그레이션 (Babel-standalone 제거)
- `/api/stream` SSE 라이브 갱신
- frontend의 더미 데이터를 `/api/snapshot` fetch로 교체
- src-tauri/ 추가 + `tauri build`로 `Luida.app` 빌드

## 12. 변경 이력

| 날짜 | 버전 | 변경 |
|---|---|---|
| 2026-05-26 | 0.1 | 최초 draft |
| 2026-05-26 | 0.2 | Tauri 결정 반영 (§6.3 / §7.3) |
| 2026-05-26 | 0.3 | Web Track A 완료 — packages/web/ + Bun.serve 통합 |

---

> 좋은 술집을 만들어주세요. 🍺
