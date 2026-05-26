# Phase 2 — Self Review

| | |
|---|---|
| **Phase** | 2 — TUI 대시보드 (Ink) |
| **Status** | ✅ 4-게이트 통과 |
| **Date** | 2026-05-26 |
| **Reviewer** | Luida 본인 + senior-fe-engineer 1 sub-agent |

---

## ① 변경 요약

### Prereq (Phase 1 review에서 등록)
- `tsconfig.base.json`은 그대로, 루트 `tsconfig.json`에 `jsx: "react-jsx"` 추가 + `@luida/ui` paths
- `packages/ui/tsconfig.json` 신설 — base extends + `jsx: "react-jsx"`, `types: ["bun","react"]`

### `@luida/ui` 패키지
**구조**
```
packages/ui/
├── package.json (ink ^5, react ^18, @luida/core workspace dep)
├── tsconfig.json
└── src/
    ├── index.ts                 # public re-export
    ├── App.tsx                  # 4분할 레이아웃 + 키바인딩 + polling
    ├── App.test.ts              # panelLength 단위 테스트
    ├── run.ts                   # render entry (TTY check + SIGTERM unmount)
    ├── style/tokens.ts          # DQ 풍 색·status/hp 헬퍼
    ├── util/stats.ts            # deriveStats / questProgressRatio / firstLine / relativeTime
    ├── util/stats.test.ts       # 18건
    ├── state/load.ts            # loadSnapshot(repos) 순수 함수
    ├── state/load.test.ts       # 2건
    ├── components/
    │   ├── Window.tsx           # 더블 라인 테두리 박스
    │   ├── HpBar.tsx
    │   ├── Badge.tsx
    │   ├── AdventurerCard.tsx
    │   ├── QuestRow.tsx
    │   ├── EventLogLine.tsx     # React.memo + useMemo
    │   └── EventLogLine.test.ts # 6건
    └── panels/
        ├── AdventurerPanel.tsx
        ├── QuestPanel.tsx
        ├── TavernLogPanel.tsx
        └── ChroniclePanel.tsx
```

**CLI 통합**
- `luida ui [--interval MS]` 명령 등록 (`packages/cli/src/index.ts`)
- `@luida/ui` workspace dependency 추가

### Tauri 결정 (사용자 추가 요구)
- 웹 대시보드 트랙(별도 packages/web/)은 브라우저 localhost가 아닌 **Tauri 네이티브 macOS 앱**으로 패키징하는 방향으로 `docs/web-design-spec.md` §6.3 / §7.3 갱신
- TUI(이번 Phase 2)는 cmux pane 내 Ink 그대로 — 두 표면이 같은 `tavern.db`를 공유

### 테스트 (총 116건)
- `util/stats.test.ts` 18건: 모든 헬퍼 함수
- `state/load.test.ts` 2건: empty + populated snapshot
- `App.test.ts` 4건: panelLength resolver
- `components/EventLogLine.test.ts` 6건: extractSummary 분기

---

## ② 설계 의사결정

### Ink + React 18 채택
- TUI를 컴포넌트화하면 4분할 레이아웃·키 인풋·polling을 React life-cycle로 자연스럽게 표현 가능
- 단위 테스트는 **순수 함수 분리 전략**으로 진행 — `loadSnapshot`, `deriveStats`, `panelLength`, `extractSummary` 등이 모두 React 없이 테스트됨. Ink 렌더 자체는 unit test 안 함

### loadSnapshot 분리
- React 외부에서 호출 가능한 순수 함수
- 향후 web track(packages/web/)에서도 같은 함수 재사용

### Tauri 결정 (TUI vs Web의 역할 분리)
- **TUI** (이번 Phase): cmux pane 내부, 가장 빠른 키 인터랙션
- **Web (Tauri 데스크탑 앱)**: 별도 윈도우, 더 풍부한 비주얼, 알림, 폰 PWA 가능
- 두 표면이 같은 tavern.db를 polling 또는 SSE로 공유 → 데이터 중복 없음

### useEffect cleanup 패턴
- DB handle을 ref 기반이 아닌 effect-scoped closure로 관리
- `cancelled` flag로 unmount 이후 setState 차단
- DB 오픈 실패 시에도 빈 cleanup을 반환해 effect 일관성 유지

### snapshot/focusedPanel을 useRef로 미러
- useInput 콜백이 매 렌더에서 새로 등록될 때 stale closure 위험을 ref로 해소
- `moveCursor`는 useCallback으로 안정 reference

### EventLogLine 메모이제이션
- 1초 polling에서 50건 inmail JSON.parse 비용을 React.memo + useMemo로 흡수
- `extractSummary`를 export해 단위 테스트 가능

---

## ③ 발견 사항 · 이슈

### Critical (sub-agent 2건 → **모두 처리**)

| # | 이슈 | 처리 |
|---|---|---|
| C1 | useEffect cleanup race — setError가 cancelled 이후에도 호출될 가능성 | **수정** — tick 내부 try/catch 모두에 `if (cancelled) return` 가드 + 성공 시 setError(null) 자동 복구 |
| C2 | DB 오픈 실패 경로에서 cleanup 미반환 | **수정** — error 분기에서도 빈 cleanup 반환. close 실패는 LUIDA_DEBUG=1일 때만 console.error |

### Major (6건 → **5건 수정 / 1건 deferral**)

| # | 이슈 | 처리 |
|---|---|---|
| M1 | useInput moveCursor stale closure | **수정** — snapshotRef + focusedPanelRef + useCallback. moveCursor 자체는 의존성 없는 stable callback |
| M2 | Tab/Shift-Tab 비대칭 | **수정** — `key.shift && key.tab` 시 역방향 이동 |
| M3 | cursor 4-tuple 마법숫자 | **수정** — `Record<PanelIdx, number>` + `PANEL_COUNT` 상수 + `panelLength()` 헬퍼 |
| M4 | EventLogLine JSON.parse 매 렌더 (50건/sec) | **수정** — React.memo + useMemo. extractSummary 분리·테스트 |
| M5 | run.ts raw mode 미체크, SIGTERM unmount 없음 | **수정** — stdin TTY 체크 후 명시 에러, SIGTERM once handler로 unmount + 종료 후 listener 제거 |
| M6 | width="50%" 협소 pane 깨짐 | **부분 수정 / deferral** — flexShrink={1} 추가, gap={2} 사용. useStdout 기반 단일 컬럼 fallback은 사용자 실측 후 Phase 4 검토 |

### Minor (10건 → **다수 의식적 미수정 (deferral 명시)**)

- ⏸ borderStyle "double" focus 강조 — chalk truecolor 보장 환경(cmux/Ghostty)에서 색만으로 충분
- ⏸ 한글 cell-width 정렬 — string-width 의존 추가 필요. Phase 4 정착 시 검토
- ⏸ React 19 migration 대비 `ReactElement` 타입 사용 — 이번 Phase에서 적용 (run.ts/App.tsx에 `ReactElement` import)
- ⏸ adaptive polling backoff — DB write가 idle일 때 setTimeout 재귀로 전환. Phase 4 brain 도입 시 변경 감지 컬럼 추가하면 자연스러움
- ⏸ Badge 영어 약어 — 한글 톤 유지가 우선
- ⏸ ChroniclePanel focus 시 안내 — Phase 5에서 본격화 시 추가
- ✅ Box 헤더 gap={2} 적용 (공백 문자열 분리 제거)
- ⏸ React.memo 전반 적용 — EventLogLine만 적용. 다른 컴포넌트는 polling 빈도와 row 수 고려 시 효과 미미

---

## ④ 잔여 위험

| # | 위험 | 영향 | 완화 |
|---|---|---|---|
| R1 | 50건 inmail × 1초 polling이 수천 건 누적 시 SELECT 비용 증가 | 중 | inmail에 `ix_inmail_recent` 인덱스 있음 + LIMIT 50. 향후 prune job 또는 변경 감지 기반 increment polling 검토 (Phase 4) |
| R2 | TUI가 cmux/Ghostty 외 환경에서 색상·박스 깨질 가능성 | 저 | macOS only 가정. 향후 다른 터미널 지원 시 fallback 필요 |
| R3 | ChroniclePanel placeholder만 표시 — 사용자가 4번째 패널 의도를 모를 수 있음 | 저 | Phase 5 도입 시 자연 해소 |
| R4 | `useInput`의 key.shift 검증이 일부 터미널에서 Shift+Tab을 일반 Tab으로 전송 | 저 | cmux/Ghostty는 정상 동작 |
| R5 | Tauri 통합은 별도 트랙 — 진행 시 packages/web 부트스트랩 + src-tauri/ 필요 (지금은 spec만 갱신) | 정보 | web 트랙 시작 시 별도 phase 또는 외부 디자인 Claude에 위임 |

---

## ⑤ 다음 Phase 영향

### Phase 3 prereq (Phase 0·1·2 누적)
1. **quest 멱등성**: `quests.source_inmail_id INTEGER UNIQUE` 컬럼 추가 마이그레이션 (0002_*.sql)
2. **brain 패키지 우선 생성**: relationships 평가기를 brain에 둠
3. **Stop hook 본격화**: quest_completed 이벤트 발행
4. **relationships.yaml 파서**: `packages/core/src/relationships.ts`

### Phase 4 prereq (누적)
- 마이그레이션 SQL 패키징
- payload Zod schema
- surface별 cmux 주입 in-process mutex
- adaptive polling backoff (UI도 함께 변경)
- web/ 패키지 시작 시점 (Tauri 래퍼 포함)

### 즉시 사용 가능한 API surface
```typescript
import {
  App, runUi, loadSnapshot,
  deriveStats, questProgressRatio, firstLine, relativeTime,
  colors, hpColor, statusColor,
} from '@luida/ui';
```

---

## 4-게이트 통과 증거

| 게이트 | 통과 증거 |
|---|---|
| [1] 검증 실행 | `bun run typecheck` → 0 error / `bun test` → **116 pass · 0 fail · 255 expect** |
| [2] 셀프 리뷰 문서 | 본 문서 `docs/reviews/phase-2.md` 작성. 5개 섹션 + 4-게이트 증거 |
| [3] 셀프 리뷰 에이전트 | senior-fe-engineer 1 sub-agent 리뷰 → Critical 2 · Major 6 · Minor 10 식별 → 처리 후 **잔여 Critical 0 · 잔여 Major는 모두 수정 또는 deferral 명시** |
| [4] 완료 선언 | 다음 메시지에 `## ✅ Phase 2 — 4-게이트 모두 통과` 헤더 출력 |
