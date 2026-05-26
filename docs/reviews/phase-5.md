# Phase 5 — Self Review

| | |
|---|---|
| **Phase** | 5 — 학습 루프 (reflect + 패턴 승격) |
| **Status** | ✅ 4-게이트 통과 |
| **Date** | 2026-05-26 |
| **Reviewer** | Luida 본인 + backend-engineer 1 sub-agent |

---

## ① 변경 요약

### `@luida/brain` 확장
- `src/reflect.ts` (신규) — analyzeEvents / reflect / promotePattern / renderPatternMarkdown / findCandidateInProposals
- `src/reflect.test.ts` (신규) — 12건
- `src/daemon.ts` — runBrain에 reflect job 6시간 주기 + 재시작 시 lastReflectAt 복원
- `src/memory.ts` — chronicle 자동 rotation (2MB 초과 시 atomic rename)
- `src/index.ts` — re-export

### CLI 확장
- `luida brain reflect` — 1회 즉시 reflect (디버그·운영자 수동)
- `luida promote-pattern <id> [--activate]` — 후보 → relationship 승급 (기본 disabled+propose)

### 테스트 총 182건 (Phase 4 166 → +16)

---

## ② 설계 의사결정

### reflect 휴리스틱 (MVP)
- 같은 (from→to) 쌍의 `quest_dispatched` ≥ N회 → 패턴 후보
- `pr_created` 함께 발생 시 신뢰도 부스트 (+0.2)
- 기본: 최소 3회, 신뢰도 0.4 이상만 후보

### 2단계 승급 게이트 (C1 대응)
- **promote는 기본 disabled + action='propose'** — 사용자 검토 단계 보장
- `--activate`로 명시적 활성화 시에만 `enabled=1 + auto_dispatch`
- learned-promoted source 룰은 사람 정의 룰과 동등 권한이지만 시작은 안전 모드

### candidate 복원 (C2 대응)
- promote 명령은 먼저 inmail `proposal` payload에서 candidate 직접 복원 (events 휘발성 무관)
- fallback으로 reflect 재실행
- payload schema `v: 1` 명시

### dedupe 시간 버킷
- promote-proposal dedupe key: `promote-proposal:${id}:${YYYY-MM}` — 다음 달엔 재제안 가능

### lastReflectAt 영속화
- brain daemon은 `brain_reflect_done` 이벤트를 매 reflect 종료 시 기록
- 재시작 시 `events.byKind('brain_reflect_done', 1)`로 lastReflectAt 복원 → 6시간 주기 보존

### chronicle rotation atomic
- `renameSync`로 원자적 이동 (같은 파일시스템 atomic)
- 아카이브가 이미 있으면 임시 파일로 합친 뒤 swap
- 어느 단계든 실패 시 원본 보존 (데이터 손실 차단)

### analyzeEvents 입력 타입 가드
- `payload.from`을 typeof string으로 좁힘
- adventurer 존재 검증 — 모르는 이름은 noise로 차단

---

## ③ 발견 사항 · 이슈

### Critical (sub-agent 2건 → **모두 처리**)

| # | 이슈 | 처리 |
|---|---|---|
| C1 | promote가 즉시 enabled=1로 자동화 활성화 → 사용자 검토 없이 워크로드 폭주 위험 | **수정** — `promotePattern`에 `PromoteOpts.activate` 도입. 기본 disabled+propose, `--activate` 명령어로만 활성화 |
| C2 | promote가 휘발성 events 의존 → 사용자 지연 시 candidate 사라짐 | **수정** — `findCandidateInProposals`로 inmail payload에서 직접 복원. CLI는 fallback으로 reflect 재실행 |

### Major (6건 → **5건 수정 / 1건 deferral**)

| # | 이슈 | 처리 |
|---|---|---|
| M1 | dedupe key가 시간 버킷 없어 한 번 처리되면 영원히 재제안 안 됨 | **수정** — `:${YYYY-MM}` 월 버킷 추가 |
| M2 | lastReflectAt 휘발성 → 재시작 시 무조건 즉시 reflect | **수정** — `brain_reflect_done` 이벤트로 영속화, 데몬 시작 시 복원 |
| M3 | chronicle rotation race + 부분 실패 시 데이터 손실 | **수정** — renameSync atomic, 실패 시 원본 보존 |
| M4 | recentSince(5000)이 silent truncation | **deferral** — 7일 5000건은 평균 운영에 충분. Phase 6에서 page loop 또는 cap 표시 |
| M5 | analyzeEvents `from` 타입 가드 없음 + adventurer 미존재 노이즈 | **수정** — typeof string + known set 검증 |
| M6 | CLI close 누락 (try/finally 구조) | **수정** — try-finally + repos.close 보호 |

### Minor (4건 → 일부 수정 / 일부 deferral)
- ✅ proposal payload에 v:1 버전 필드
- ✅ renderPatternMarkdown 텍스트와 실제 promote 정책 일치 (`--activate` 안내)
- ⏸ m1 runBrain 여러 인스턴스 lastReflectAt 미공유 — 단일 데몬 가정 (사용자 환경)
- ⏸ m2 slug 빈 입력 — analyzeEvents에서 이미 거름

---

## ④ 잔여 위험

| # | 위험 | 영향 | 완화 |
|---|---|---|---|
| R1 | recentSince(5000) silent cap (M4) — 매우 활성된 시스템(일 1000+ 이벤트)에서 학습 신뢰도 저하 | 저~중 | Phase 6에서 페이지네이션 또는 cap 표시. 현 운영 규모(5~10 sidecar)에선 무관 |
| R2 | reflect 휴리스틱이 단순 — 시간순서 무시, 동시 발생 패턴만 발견 | 중 | Phase 6+에서 Markov chain·시계열 패턴 발견. 현재는 사용자 검토로 보완 |
| R3 | 패턴 markdown은 인간 가독용일 뿐, 머신 권위는 inmail payload | 정보 | C2 fix로 명확화. markdown은 audit/검토용 |
| R4 | brain daemon이 cmux pane에 어떻게 띄울지 startup 가이드 부재 | 저 | README 또는 docs/operations.md (다음 작업) |
| R5 | 이전 Phase들의 잔여 R 항목 누적 | 정보 | 각 phase review 참조 |

---

## ⑤ 다음 Phase 영향

### Phase 6+ 후보 (Goal 외)
1. **Web Track B**: Vite + TSX 마이그레이션, SSE 라이브 갱신, Tauri 래퍼 (`src-tauri/`)
2. **운영 가이드**: `docs/operations.md` — sidecar / brain / mcp / web 데몬 startup, launchd plist 샘플, 사용자 cmux 설정
3. **Zod schema validation**: payload·MCP input 검증 통일
4. **고급 학습**: Markov chain, 시계열 패턴, automl 가벼운 시도
5. **권한 모델**: 워크트리 밖 파일 접근 차단 hook (PreToolUse), 시크릿 파일 마스킹
6. **TUI 협소 pane fallback**: useStdout 기반 단일 컬럼

### 즉시 사용 가능한 API (Phase 5 추가)
```typescript
import {
  analyzeEvents, reflect, promotePattern,
  findCandidateInProposals, renderPatternMarkdown,
} from '@luida/brain';
```

### CLI 최종 명령 목록
```
luida db init
luida sidecar --me <name> [...]
luida ui [--interval MS]
luida web [--port 4321]
luida brain start [--interval MS] [--once]
luida brain reflect
luida promote-pattern <id> [--activate]
luida sync-rules <yaml-file>
luida mcp start [--me NAME]
```

---

## 4-게이트 통과 증거

| 게이트 | 통과 증거 |
|---|---|
| [1] 검증 실행 | `bun run typecheck` → 0 error / `bun test` → **182 pass · 0 fail · 400 expect** (22 files, 1290ms). 중간 1건 실패(promote 기본값 변경에 따른 테스트 업데이트) 후 재실행 통과 |
| [2] 셀프 리뷰 문서 | 본 문서 `docs/reviews/phase-5.md` 작성. 5개 섹션 + 4-게이트 증거 |
| [3] 셀프 리뷰 에이전트 | backend-engineer 1 sub-agent → Critical 2 · Major 6 · Minor 4 식별 → 처리 후 **잔여 Critical 0 · 잔여 Major는 모두 수정 또는 Phase 6 deferral 명시** |
| [4] 완료 선언 | 다음 메시지에 `## ✅ Phase 5 — 4-게이트 모두 통과` 헤더 |
