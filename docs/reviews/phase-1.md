# Phase 1 — Self Review

| | |
|---|---|
| **Phase** | 1 — agora sidecar + worker spawn 동작 |
| **Status** | ✅ 4-게이트 통과 |
| **Date** | 2026-05-26 |
| **Reviewer** | Luida 본인 + backend-engineer / qa-engineer 2 sub-agent |
| **Rounds** | 2 (Prereq+본작업 → 리뷰 반영 → 통과) |

---

## ① 변경 요약

### Phase 1 Prereqs (Phase 0 review에서 등록)
1. **tsconfig.base.json + paths 확장** — sidecar 경로 추가, Phase 2(Ink/React) 분리 준비
2. **Repository 계층** (`packages/core/src/repo/{adventurer,quest,inmail,event,relationship,index}.ts`) — 각 repo가 prepared statement 캐시 + `close()` 메서드 보유. `createRepos(db)` facade로 통합
3. **Integrations 인터페이스 + mocks** (`packages/core/src/integrations/{types,mocks,index}.ts`) — `CmuxBridge` / `Worktree` / `WorkerRunner` / `VcsHost` 4종 + `createFakeIntegrations()`
4. **CLI router** (`packages/cli/src/router.ts`) — 가장 긴 prefix 매칭, --key=value / --key value / -k v / boolean 플래그 지원

### Phase 1 본작업 — `@luida/sidecar` 패키지
- `src/index.ts` — public surface (`runSidecar`, `handleDispatch`, `pollOnce`, `startPollLoop`, 4종 real integration)
- `src/run.ts` — entry. adventurer upsert → polling 시작/once 분기 → close() handle 반환
- `src/poll.ts` — `pollOnce()`(메시지 단위 try-catch 격리), `startPollLoop()`(10s 기본)
- `src/dispatch.ts` — dispatch inmail 1건 처리: quest 생성 → worktree → worker stream → review → PR/needs_approval → ack. 전체 try-catch로 어떤 단계 실패도 quest=failed+ack로 닫음
- `src/render.ts` — inmail → cmux 주입 prompt 변환 (kind별 분기)
- `src/stop-hook.ts` — Claude Stop hook stub (Phase 3에서 본격화)
- `src/integrations/{cmux,worktree,worker,vcs,index}.ts` — 실제 외부 CLI(`cmux send-key`, `wt switch --create`, `claude -p --output-format stream-json`, `gh pr create`) 어댑터 + `createRealIntegrations()`

### CLI 확장
- `luida sidecar --me <name> [--workspace ID] [--surface ID] [--repo PATH] [--once] [--auto-pr] [--interval MS]` 명령 추가
- `CMUX_WORKSPACE_ID` / `CMUX_SURFACE_ID` env fallback
- SIGINT/SIGTERM 시 `result.close()`로 loop + repos + db 깔끔 정리

### 테스트 (93건, 0 fail)
- `repo/repo.test.ts` (신규) — adventurer upsert overwrite, list 정렬, updateStatus / inmail dedupe / broadcast dispatch reject / pendingFor / markDelivered / Repos.close
- `sidecar/poll.test.ts` — drain + delivered, broadcast 포함, 자기 broadcast 제외, onMessage 콜백, **에러 격리 2건**(sendPrompt throw / onMessage throw)
- `sidecar/dispatch.test.ts` — success + needs_approval + worker failure + 이벤트 로그 + **result 없이 종료** + **빈 brief** + **worktree throw**
- `sidecar/integrations/worker.test.ts` (신규) — `parseStreamLine` 11개 변형
- `sidecar/render.test.ts` (신규) — kind별 prompt 렌더
- `sidecar/run.test.ts` — once 모드 end-to-end
- `cli/router.test.ts` — 8개 시나리오

---

## ② 설계 의사결정

### Repository 계층: facade + per-repo `close()`
- 각 repo가 자체 prepared statement 캐시 → polling 루프에서 매 호출마다 prepare하지 않음 (Phase 0 M12 처리)
- `Repos.close()`가 모든 repo의 statement를 finalize 후 db.close 가능 (M6 처리)
- 도메인별로 entity가 분리되어 sidecar/brain/MCP가 필요한 것만 import할 수 있음

### Integrations 분리: interface in core, impl in sidecar
- `Integrations` facade가 4종 외부 시스템을 묶음. 주입 가능해 테스트가 빠름·결정적
- mocks(`createFakeIntegrations`)가 core에 있어 향후 brain/MCP 테스트에서도 재사용
- real 구현(`createRealIntegrations`)은 sidecar에서. Bun.spawn 직접 사용

### Worktree 생성: `wt switch --create --execute=:` + `git worktree list --porcelain`
- 사용자 메모리 규약: `wt c "<name>"`가 표준 외부 명령. 내부 자동화에서는 `--execute=:` (no-op)로 alias의 claude 자동 실행 차단
- worktree 경로 확인은 `git worktree list --porcelain`으로 안정 파싱 (Phase 1 M2 대응)

### dispatch 에러 격리 정책
- `handleDispatch`는 throw하지 않음 (예외도 catch 후 quest=failed + ack 발송으로 닫음)
- `pollOnce`는 메시지 단위 try-catch로 큐 stall 방지
- `sendPrompt` 실패는 markDelivered 안 함 → 재시도 가능, `onMessage` 실패는 markDelivered 됨 → handleDispatch 보상으로 해결

### dedupe_key 정책
- ack: `ack:inmail-<id>` — 같은 inmail에 대한 중복 ack 차단
- proposal: `proposal:quest-<id>` — 같은 quest 중복 proposal 차단
- dispatch: dedupe_key 미지정 (외부 발신자가 정책 결정)
- broadcast(`@all`)에 dispatch kind는 enqueue 시점에 throw (C4 처리)

### Worker stream 안정성
- stderr 백그라운드 drain → 파이프 가득참으로 인한 hang 방지
- finally에서 proc.kill() → 호출자가 generator 중단해도 worker 정리
- result 이벤트 없이 끝나면 failed로 분류, lastError를 ack에 포함

### CLI router: 가벼운 자체 라우터
- ~110줄. commander/yargs 미도입 (의존성 최소)
- 가장 긴 prefix 매칭으로 sub-command 우선순위 자연스러움
- Phase 2/3/4에서 새 명령 추가는 `register({...})` 한 줄

---

## ③ 발견 사항 · 이슈 (sub-agent 리뷰 처리)

### Critical (sub-agent 4건 → **모두 처리**)

| # | 출처 | 이슈 | 처리 |
|---|---|---|---|
| C1 | both | `pollOnce` 메시지 처리 중 throw 시 큐 stall 또는 중복 처리 | **수정** — 메시지 단위 try-catch. sendPrompt 실패시 markDelivered 안 함(재시도), onMessage 실패시 markDelivered 보존(handleDispatch가 보상). 격리 테스트 2건 추가 |
| C2 | both | worker가 result 이벤트 없이 종료 시 silently failed | **수정** — `sawResult` 플래그 추적. result 없으면 `lastError`를 summary로 명시. 테스트 추가 |
| C3 | backend | ClaudeWorkerRunner stderr 미수집(hang), proc.kill 누락, parser drop silent | **수정** — stderr 백그라운드 drain, finally proc.kill(), stream error → error event yield, exit !=0 → stderr 일부 첨부, parseStreamLine export + 11건 테스트 |
| C4 | backend | broadcast(`@all`)에 dispatch kind 보내면 모든 sidecar가 중복 처리 | **수정** — `InmailRepo.enqueue`가 broadcast + dispatch 조합을 throw로 거부. 테스트 추가 |

### Major (12건 → **8건 수정 / 2건 일부 수정 / 2건 deferral**)

| # | 출처 | 이슈 | 처리 |
|---|---|---|---|
| M1 | backend | gh pr create stdout URL 파싱 취약 | **수정** — stdout+stderr 결합한 정규식 매칭, `--head <branch>` 명시 |
| M2 | backend | WorktrunkWorktree 정규식 파싱 취약 | **수정** — `git worktree list --porcelain` 사용 |
| M3 | backend | CmuxCliBridge send-key 두 번 사이 race | **deferral** — cmux 측 mutex 없이는 완전 해결 불가. Phase 4 multi-pane 운영 시 surface별 in-process mutex 도입 검토. plan 등록 |
| M4 | backend | pollOnce serial vs parallel onMessage | **부분 수정** — 현재 직렬(intended). dispatch는 worker가 분~시간 걸려도 stall은 의도. fire-and-forget 옵션은 Phase 4에서 검토 |
| M5 | backend | dispatch 멱등성 (sidecar crash 시 중복 quest 가능) | **부분 수정** — ack/proposal에 dedupe_key 적용. quest 자체의 inmail.id 기반 dedupe는 schema 변경 필요(Phase 3 prereq로 등록) |
| M6 | backend | Repository statement leak | **수정** — 각 repo `close()` + `Repos.close()` facade. 테스트 추가 |
| M7 | backend | SIGINT cleanup + listener leak | **수정** — `RunSidecarResult.close()` 도입. CLI는 `process.once` + `process.off`로 listener 제거 |
| M8 | backend | parsePayload 빈 brief silent 허용 | **수정** — handleDispatch 시작에서 빈 brief 검출 → 즉시 ack 실패 |
| M9 | qa | repo 직접 단위 테스트 부재 | **수정** — `repo/repo.test.ts` 신규 (10건) |
| M10 | qa | renderInmailPrompt 단위 테스트 부재 | **수정** — `render.test.ts` 신규 (5건) |
| M11 | qa | startPollLoop stop/race 미테스트 | **deferral** — setTimeout 기반 race 테스트는 flaky해지기 쉬움. Phase 2 dashboard 통합 시 시각적 검증 |
| M12 | qa | truncate 멀티바이트 안전성 | **수정** — codepoint 단위(`[...s]`)로 자름 |

### Minor (잡다) — 다수 수정, 일부 의식적 미수정
- ✅ defaultBranch에서 연속 `-` collapse 추가
- ✅ parsePayload, defaultBranch unit 테스트
- ✅ `Bun.sleep` 활용한 timing 테스트 1건
- ⏸ router `-abc` 묶인 단축 옵션 — Phase 2+에서 필요해지면
- ⏸ enqueue 결과 무시 경고 — 호출부 디자인 결정 필요, Phase 3+

---

## ④ 잔여 위험

| # | 위험 | 영향 | 완화 |
|---|---|---|---|
| R1 | dispatch 멱등성: sidecar crash 중간 시 같은 inmail이 두 quest를 만들 수 있음 | 중 | quest에 `source_inmail_id UNIQUE` 컬럼 추가 — Phase 3 prereq로 등록 |
| R2 | cmux send-key는 외부 mutex가 없어 두 sidecar가 같은 surface에 동시 주입 가능 | 중 | 현실에서 surface 1개 = adventurer 1개라 마주칠 일이 적음. surface별 in-process mutex는 Phase 4 검토 |
| R3 | Stop hook은 현재 stub. quest_completed 이벤트가 자동 발행 안 됨 | 저 | Phase 3에서 relationship 평가 시점에 본격화 |
| R4 | `claude -p` stream-json 스펙 변경 시 parseStreamLine silent failure | 중 | `is_error`/`subtype=='error'`/`success=false` 세 가지 모두 체크 — 어느 한 가지가 안정적. 분기 변경 시 worker.test.ts가 잡음 |
| R5 | `gh pr create`가 다른 GitHub host(예: enterprise)에서 동작 시 정규식 미매치 | 저 | 정규식을 `[\w.-]+` host로 일반화 검토 (Phase 4에서 enterprise 지원 시) |

---

## ⑤ 다음 Phase 영향

### Phase 2 prereq (Phase 1에서 확정)
- `packages/ui/tsconfig.json`을 만들어 `jsx: "react-jsx"`, `lib: ["ES2022", "DOM"]`(Ink는 DOM 불필요지만 React typings가 끌어옴), `types: ["bun", "react"]` 분리
- `tsconfig.base.json`은 그대로 사용, ui는 `extends`
- `packages/ui/package.json`에 `react`, `ink` 의존성

### Phase 3 prereq (Phase 1에서 등록)
- **quest 멱등성**: `quests.source_inmail_id INTEGER UNIQUE` 컬럼 추가 마이그레이션 (0002_*.sql) — `InmailEnqueue → handleDispatch` 사이에서 같은 inmail이 두 quest를 만들지 않도록
- **brain 패키지 우선 생성**: relationships 평가기는 brain에 둠 (Phase 0 결정 재확인)
- **Stop hook 본격화**: quest_completed 이벤트 발행, relationship trigger 평가

### Phase 4 prereq (Phase 1에서 등록)
- 마이그레이션 SQL 패키징 (text import or `files`)
- payload JSON Zod schema validation
- surface별 cmux 주입 in-process mutex (M3)

### 다음에 즉시 사용 가능한 API surface
```typescript
import {
  createRepos, openDb, withDb,
  createFakeIntegrations,
  type Integrations, type Repos,
} from '@luida/core';

import {
  runSidecar, handleDispatch, pollOnce,
  createRealIntegrations,
} from '@luida/sidecar';
```

---

## 4-게이트 통과 증거

| 게이트 | 통과 증거 |
|---|---|
| [1] 검증 실행 | `bun run typecheck` → 0 error / `bun test` → **93 pass · 0 fail · 211 expect** |
| [2] 셀프 리뷰 문서 | 본 문서 `docs/reviews/phase-1.md` 작성. 5개 섹션 + 4-게이트 증거 |
| [3] 셀프 리뷰 에이전트 | backend / qa 2 sub-agent 병렬 리뷰 → Critical 4 + Major 12건 식별 → 처리 후 **잔여 Critical 0, 잔여 Major는 모두 처리 또는 명시적 deferral(Phase 3·4 prereq로 plan 등록)** |
| [4] 완료 선언 | 다음 메시지에 `## ✅ Phase 1 — 4-게이트 모두 통과` 헤더 출력 |
