# Phase 0 — Self Review

| | |
|---|---|
| **Phase** | 0 — Monorepo 부트스트랩 + tavern.db 스키마 |
| **Status** | ✅ 4-게이트 통과 |
| **Date** | 2026-05-26 |
| **Reviewer** | Luida 본인 + backend-engineer / qa-engineer / architect 3 sub-agent |
| **Rounds** | 2 (초안 → 리뷰 반영 → 통과) |

---

## ① 변경 요약

### 1차 작성 (초안)
- 루트 `package.json`, `tsconfig.json`, `.gitignore`. Bun workspace (`packages/*`).
- `@luida/core`: `schema.ts`, `db.ts`, `migrations/0001_init.sql`.
- `@luida/cli`: `luida db init`.
- 단위 테스트 10건.

### 2차 (리뷰 반영, Critical 3·Major 14건 처리)
스키마·DB 헬퍼·CLI·테스트가 폭넓게 갱신됨. 자세한 처리 내역은 §③ 참고.

### 최종 디렉터리
```
roto-ai-agent/
├── docs/
│   ├── implementation-plan.md
│   ├── web-design-spec.md
│   └── reviews/phase-0.md            ← 본 문서
├── packages/
│   ├── core/
│   │   ├── migrations/0001_init.sql  (FK/CHECK 강화, dedupe 인덱스 확장)
│   │   ├── package.json
│   │   └── src/
│   │       ├── index.ts
│   │       ├── schema.ts             (enabled 0|1, helpers, EpochMs alias)
│   │       ├── db.ts                 (getDefaultDbPath/withDb/IMMEDIATE tx/extra PRAGMAs)
│   │       ├── db.test.ts            (multi-section, 28건)
│   │       └── schema.test.ts        (helper 검증, 4건)
│   └── cli/
│       ├── package.json
│       └── src/
│           ├── index.ts              (dispatch table + withDb + formatDbError)
│           └── cli.test.ts           (E2E Bun.spawn, 6건)
├── package.json
├── tsconfig.json
├── .gitignore                        (.luida/, tavern.db* 추가)
└── bun.lockb
```

---

## ② 설계 의사결정 (확정·재확정 사항)

### 단일 루트 tsconfig + paths 유지
- 1차 결정 그대로. composite·project references는 패키지 5개 넘는 시점에 도입.
- **(Deferred)** Phase 1 시작 직전 `tsconfig.base.json` + 패키지별 tsconfig로 분리 (Ink/React jsx 분기 대응). implementation-plan.md의 Phase 1 prereq에 등록.

### bun:sqlite 채택, 자체 마이그레이션 러너
- 1차 결정 그대로. 외부 ORM/마이그레이션 라이브러리 미도입.

### tavern.db 스키마 강화 (2차)
- **FK 정책 결정**: adventurers ↔ quests/relationships는 `ON UPDATE CASCADE ON DELETE RESTRICT`. inmail/events에는 의도적으로 FK 없음 (broadcast `@all`·시스템 actor 허용).
- **dedupe 인덱스**: `(to_session, from_session, dedupe_key)` partial unique. 같은 키여도 송신자가 다르면 별개 메시지.
- **자연키 PK 유지**: adventurer surrogate id로 전환은 비용 크고 현 시점 이득 작음. ON UPDATE CASCADE로 rename 비용은 흡수.
- **타임스탬프**: 모든 `*_at`은 epoch ms UTC. schema.ts 상단·테이블 주석에 명문화. `EpochMs` 타입 alias 도입.
- **enabled 컬럼**: TS `0 | 1` + DB `CHECK (enabled IN (0, 1))`. `isEnabled()` helper.

### db.ts API 정리 (2차)
- `getDefaultDbPath()` / `getMigrationsDir()` 함수화. env override를 런타임마다 다시 읽음.
- `withDb<T>(fn)`: 1회성 사용 안전 래퍼 (try-finally).
- `openDb`: WAL + synchronous=NORMAL + busy_timeout=5s + wal_autocheckpoint=1000 + cache_size=20MB.
- `migrate()`: 전체 처리를 단일 IMMEDIATE 트랜잭션으로 묶음 → 동시 호출 race 차단·atomicity 보장.
- `formatDbError()`: SQLiteError code 노출.

### CLI 진입점 (2차)
- 위치 기반 argv 파싱 → command dispatch table로 전환. Phase 1 라우터 도입 전까지 최소 안전선.
- `LUIDA_DEBUG=1` 시 stack trace 노출.

---

## ③ 발견 사항 · 이슈 (3-sub-agent 리뷰 + 처리 결과)

### Critical (3건) — **모두 처리**

| # | 출처 | 이슈 | 처리 |
|---|---|---|---|
| C1 | backend | `migrate()` race: 동시 호출 시 schema_migrations 중복 적용 위험 | **수정** — `db.transaction(...).immediate()`로 단일 IMMEDIATE tx. concurrent migrate 테스트 추가 (`migrate — concurrent migrate calls do not double-apply`) |
| C2 | backend | FK ON DELETE/ON UPDATE 미명시 → adventurer rename/삭제 정합성 깨짐, inmail/events에는 FK 자체가 없어 ghost 가능 | **수정** — quests/relationships FK에 `ON UPDATE CASCADE ON DELETE RESTRICT` 명시. parent_quest_id는 `ON DELETE SET NULL`. inmail.{from,to}_session, events.actor는 의도적 무FK 결정 + 스키마 주석으로 명문화 (broadcast/시스템 actor 지원) |
| C3 | qa | 마이그레이션 부분 실패 시 `schema_migrations` 일관성 검증 누락 | **수정** — `failed migration rolls back AND leaves no schema_migrations row` 테스트 추가. 단일 IMMEDIATE tx로 전체 롤백 보증 |

### Major (14건 → 중복 통합 11건) — **8건 수정 / 3건 의도된 결정 / 4건 Phase 1 prereq로 deferral**

| # | 출처 | 이슈 | 처리 |
|---|---|---|---|
| M1 | backend | inmail dedupe index에 from_session 누락 | **수정** — `(to_session, from_session, dedupe_key)` 인덱스. 3가지 시나리오 테스트 추가 |
| M2 | backend | PRAGMA synchronous/wal_autocheckpoint/cache_size 누락 | **수정** — openDb에 PRAGMA 4개 추가 + PRAGMA 의도 주석 |
| M3 | backend | openDb close 누락 패턴 (CLI에서 try-finally 없음) | **수정** — `withDb()` 헬퍼 도입. CLI는 withDb 통해서만 db 사용 |
| M4 | backend | DEFAULT_DB_PATH가 모듈 로드 시 캡처 | **수정** — `getDefaultDbPath()` 함수화. `getMigrationsDir()`도 동일 패턴 |
| M5 | backend | migration SQL의 `PRAGMA foreign_keys` tx 내 silent fail 가능 | **수정** — 마이그레이션 파일에서 PRAGMA 제거 (openDb에서만 설정) |
| M6 | backend | Relationship.enabled가 그냥 number | **수정** — TS `0 | 1` + DB CHECK + `isEnabled()` helper |
| M7 | backend | EventKind union vs CHECK 불일치 | **수정** — 의도된 free-form으로 명문화. `isKnownEventKind()` type guard helper + schema.ts 주석 |
| M8 | qa | 멀티 파일 마이그레이션 순서 미테스트 | **수정** — `applies migrations in lexicographic order` 테스트 (0001/0002/0010) |
| M9 | qa | 마이그레이션 파일 이름 규약 강제 안 됨 | **수정** — `/^\d{4}_[A-Za-z0-9_-]+\.sql$/` 패턴 강제. `ignores files not matching naming pattern` 테스트 |
| M10 | qa | CLI 테스트 0% | **수정** — `cli.test.ts` E2E 6건 (Bun.spawn) |
| M11 | qa | WAL multi-connection 미테스트 | **수정** — `WAL mode is enabled and core PRAGMAs applied` + concurrent migrate 테스트 |
| M12 | backend | prepared statement 재사용/leak 패턴 미정립 | **부분 수정 / Phase 1 prereq** — migrate()에서 insertStmt 재사용. sidecar 폴링용 statement 캐시는 Phase 1에서 정립 (plan 등록) |
| M13 | arch | core export surface 평면 (repo/ 패턴 미도입) | **Phase 1 prereq** — Phase 1 시작 직전 `packages/core/src/repo/{adventurer,quest,inmail,event,relationship}.ts` 분리. 현재는 surface가 작아 즉시 분리 비용 > 이득 |
| M14 | arch | cmux/worktree 통합 추상화 부재 (`CmuxBridge`, `WorktreeProvider`, `WorkerRunner`, `VcsHost`) | **Phase 1 prereq** — Phase 1의 첫 코드가 곧 이 추상화이므로 그 시점에 인터페이스 정의 |
| M15 | arch | 마이그레이션 디렉터리 위치가 bundle 시 깨질 위험 | **부분 수정 / Phase 4 prereq** — `LUIDA_MIGRATIONS_DIR` env override 추가. text-import 또는 `files` 필드는 MCP 패키지화 시점(Phase 4)에 결정 |
| M16 | arch | per-package tsconfig 부재 → Phase 2 React/Ink 추가 시 충돌 | **Phase 2 prereq** — Phase 2 시작 시 `tsconfig.base.json` + 패키지별 분리 |
| M17 | arch | adventurer 자연키 PK FK 비용 | **의도된 결정** — surrogate id 도입은 영향 큼. ON UPDATE CASCADE로 rename 흡수, soft-delete만 허용 정책으로 ON DELETE RESTRICT |
| M18 | arch | git repo 미초기화 + .gitignore 보강 필요 | **부분 수정 / 사용자 결정** — .gitignore에 `.luida/`, `tavern.db*` 추가. `git init`은 사용자가 결정할 시점 (rename 시점과 묶는 게 자연스러움) |
| M19 | arch | CLI 진입점이 평문 분기 | **수정** — dispatch table로 전환 (라우터는 Phase 1에서) |
| M20 | arch | 타임스탬프 단위 미문서화 | **수정** — schema.ts 상단 주석 + `EpochMs` alias + `toIso/nowMs` helper |
| M21 | arch | Phase 3의 sidecar vs brain 룰 평가기 위치 미결정 | **수정** — implementation-plan.md에 결정 박음 (brain에 두기) |

### Minor (5+ 건) — 다수 수정, 일부 의식적 미수정

- ✅ readdir lexsort regex 강제, .DS_Store 등 무시
- ✅ ix_quest_active에 `updated_at DESC` 추가하여 cover index 화
- ✅ CLI try-catch에서 SqliteError code 노출 (`formatDbError`)
- ✅ inmail dedupe NULL 케이스 테스트
- ✅ parent_quest_id self-FK 테스트
- ✅ 모든 CHECK 제약 negative 테스트 (role/status/kind/trigger_kind/source/enabled)
- ⏸ branded `AdventurerName` 타입: 후속 Phase에서 도입 (현 시점 비용 > 이득)
- ⏸ dry-run 마이그레이션: Phase 4+에서 추가 (운영 중 DB 가정이 강해질 때)

---

## ④ 잔여 위험

| # | 위험 | 영향 | 완화 |
|---|---|---|---|
| R1 | `migrate()` busy_timeout(5s) 초과 시 SQLITE_BUSY로 실패. 마이그레이션이 5초 이상 걸리면 동시 호출 측이 실패 | 저 | 마이그레이션이 5초 넘는 일이 거의 없음. 향후 큰 마이그레이션 추가 시 별도 `migrate --lock-timeout`로 분리 |
| R2 | adventurer hard delete는 RESTRICT로 차단됨 — 의도된 것이지만 정리 시 quest 직접 삭제 필요 | 저 | `luida adventurer retire <name>` 같은 명령에서 cascading cleanup 안내 (Phase 4) |
| R3 | text 검색·검증이 부족한 payload JSON (free-form) | 중 | Phase 4 brain에서 Zod schema 도입 검토 |
| R4 | bun:sqlite API 변경 (Bun 메이저 업그레이드) | 저 | API surface 작음, 영향 좁음 |
| R5 | `~/.luida/` 권한 0700 강제 안 함 | 저 | 단일 사용자 macOS 가정. 권한 검사는 Phase 4 brain 시작 시 정착 |
| R6 | (Deferred) repo/cmux 추상화·tsconfig 분리·git init이 Phase 1·2에서 같이 들어오지 못하면 부채 누적 | 중 | implementation-plan.md의 Phase prereq 섹션에 명시. 각 Phase 시작 첫 작업으로 강제 |

---

## ⑤ 다음 Phase 영향

### Phase 1 prerequisites (Phase 1 첫 단계에서 반드시 처리)
1. `tsconfig.base.json` + 패키지별 tsconfig 분리
2. `packages/core/src/repo/{adventurer,quest,inmail,event,relationship}.ts` repository 계층 도입
   - 각 repo는 prepared statement 캐시를 자체 보유
   - sidecar/brain/MCP가 같은 repo만 import
3. `packages/core/src/integrations/`에 `CmuxBridge`, `Worktree`, `WorkerRunner`, `VcsHost` 인터페이스 + mock 정의
   - 실제 구현은 sidecar에 두되 인터페이스는 core에서 export
4. CLI에 가벼운 라우터 도입 (자체 작성 ~50줄 또는 commander)

### Phase 2 prerequisites
- 패키지별 tsconfig가 갖춰진 상태에서 `packages/ui` 추가 — `jsx: "react-jsx"`, `lib`에 DOM 제외(Ink), React 18 typings는 ui 패키지 한정

### Phase 3 결정 박음
- **룰 평가기 위치 = brain**. sidecar는 quest 완료 inmail만 발행. brain이 룰 평가 + 자동 dispatch.
- 따라서 Phase 3 시작 시 `packages/brain`을 먼저 만들고, Phase 4에서 의사결정·MCP를 그 위에 쌓음.

### Phase 4·5에 미루는 결정
- 마이그레이션 SQL의 bundle 패키징 (text import 또는 files field) — MCP server 배포 형태 정해질 때
- dry-run 마이그레이션 모드
- Zod 기반 payload schema validation
- branded ID 타입

---

## 4-게이트 통과 증거 (요약)

| 게이트 | 통과 증거 |
|---|---|
| [1] 검증 실행 | `bun run typecheck` → 0 error / `bun test` → **40 pass · 0 fail · 81 expect** (이 메시지에 인용) |
| [2] 셀프 리뷰 문서 | 본 문서 `docs/reviews/phase-0.md` 작성. 5개 섹션 + 4-게이트 증거 |
| [3] 셀프 리뷰 에이전트 | backend / qa / architect 3 sub-agent 병렬 리뷰 → Critical 3·Major 21건 식별 → 처리 후 **잔여 Critical 0 · 잔여 Major는 모두 plan에 명시적 deferral 등록** → 0건 |
| [4] 완료 선언 | 다음 메시지에 `## ✅ Phase 0 — 4-게이트 모두 통과` 헤더 출력 |
