# ADR-0001: Luida v2 구현 언어 — Rust vs TypeScript

| | |
|---|---|
| **Status** | Proposed (결정 대기) |
| **Date** | 2026-05-28 |
| **Decision makers** | Roto |
| **선행** | v1(TS/Bun) Phase 0~5 + A~E 완료. v2 설계(`v2-standalone.md` v0.6) |

---

## 1. 배경

v1은 TypeScript + Bun으로 구현되어 동작·검증됨(215 tests). v2는 cmux 제거 + projects/campaigns/planner + PTY 영속 세션 + suspend/resume + Memory Tree + TokenJuice로 **사실상 전면 재작성** 수준의 변경이다.

이 재작성을 (A) Rust로 새로 쓸지, (B) 하이브리드(일부만 Rust), (C) TS 유지할지 결정한다.

핵심 동인: **장기 실행 안정성**(brain daemon 24/7), **단일 바이너리 배포**(노트북 2대 hand-off), **Tauri 시너지**.

---

## 2. 의사결정 기준 (가중치)

| 기준 | 가중치 | 설명 |
|---|---|---|
| 장기 실행 안정성 | ★★★ | brain daemon·PTY 세션이 며칠씩 떠 있음. crash/leak이 치명적 |
| 단일 바이너리 배포 | ★★★ | 노트북 2대(집·회사) hand-off. 런타임 의존 없는 배포가 유리 |
| Tauri 통합 단순성 | ★★ | 데스크탑 앱이 목표. core가 같은 언어면 프로세스·IPC 단순화 |
| 개발 속도 | ★★★ | 설계가 아직 진화 중. 빠른 반복이 중요 |
| 기존 자산 재사용 | ★★ | v1 코드 215테스트·스키마·개념 |
| 유지보수 일관성 | ★★ | 사용자 주력이 TS(agora/admin). 학습 비용 |
| 생태계 성숙도 | ★★ | 필요 라이브러리 존재·안정성 |

---

## 3. 옵션별 평가

### Option A — 전면 Rust v2
**구성**: core/brain/sidecar/TUI(ratatui) = Rust. web frontend만 React. Tauri(Rust)에 core 직접 임베드.

| 기준 | 점수 | 근거 |
|---|---|---|
| 장기 안정성 | ◎ | GC 없음, 메모리 안전. daemon·PTY에 최적 |
| 단일 바이너리 | ◎ | `cargo build` → 단일 실행파일. Bun 런타임 의존 0 |
| Tauri 통합 | ◎ | core가 Tauri 프로세스 안. 별도 backend·IPC 불필요 |
| 개발 속도 | △ | borrow checker·컴파일 시간. 단 설계 성숙도 높아 완화 |
| 기존 자산 | △ | v1 코드 폐기(레퍼런스화). 스키마·개념·테스트 시나리오는 이식 |
| 유지보수 | △ | Rust 학습. agora/admin과 언어 불일치 |
| 생태계 | ◎ | ratatui/rusqlite/portable-pty/tauri/tokio 전부 성숙 |

**비용 추정**: v2 기능을 0에서 Rust로. 단 v1이 청사진이라 설계 시간은 절약. 코드량 중간(러스트가 다소 verbose).

### Option B — 하이브리드 (안정성 critical만 Rust)
**구성**: brain daemon + PTY 세션 = Rust 바이너리(별도). 나머지(core/sidecar/mcp/web/TUI) = TS 유지. TS↔Rust는 tavern.db 파일 + JSON IPC로 통신.

| 기준 | 점수 | 근거 |
|---|---|---|
| 장기 안정성 | ○ | 가장 critical한 daemon/PTY만 Rust로 → 핵심 이득 확보 |
| 단일 바이너리 | △ | Bun + Rust 두 바이너리. 배포 복잡 |
| Tauri 통합 | △ | 여전히 별도 backend 프로세스 |
| 개발 속도 | ○ | 대부분 TS 유지라 빠름 |
| 기존 자산 | ◎ | v1 대부분 유지 |
| 유지보수 | △ | **2언어 backend 경계** — 타입 동기화·디버깅 이중. 가장 큰 단점 |
| 생태계 | ◎ | — |

**비용 추정**: 낮음~중간. 단 2언어 경계가 영구 세금.

### Option C — 전면 TS 유지 (Option α: Tauri shim만 Rust)
**구성**: 현행 그대로. v2도 TS+Bun. Tauri는 얇은 Rust shim. TUI는 Ink.

| 기준 | 점수 | 근거 |
|---|---|---|
| 장기 안정성 | △ | Bun daemon 안정성은 "충분"하나 Rust 대비 열위. GC·메모리 |
| 단일 바이너리 | △ | `bun build --compile`로 단일 바이너리 가능하나 런타임 내장 |
| Tauri 통합 | △ | 별도 Bun backend + shim |
| 개발 속도 | ◎ | 가장 빠름. 기존 흐름 유지 |
| 기존 자산 | ◎ | 100% 재사용 |
| 유지보수 | ◎ | 단일 언어, agora/admin 일관 |
| 생태계 | ○ | Ink/bun:sqlite 충분하나 PTY는 node-pty(네이티브 빌드) 필요 |

**비용 추정**: 가장 낮음.

---

## 4. 비교 요약

| 기준 (가중치) | A 전면Rust | B 하이브리드 | C 전면TS |
|---|---|---|---|
| 장기 안정성 (★★★) | ◎ | ○ | △ |
| 단일 바이너리 (★★★) | ◎ | △ | △ |
| Tauri 통합 (★★) | ◎ | △ | △ |
| 개발 속도 (★★★) | △ | ○ | ◎ |
| 기존 자산 (★★) | △ | ◎ | ◎ |
| 유지보수 (★★) | △ | △ | ◎ |
| 생태계 (★★) | ◎ | ◎ | ○ |

### 가중 해석
- **A**: 안정성·배포·Tauri(고가중)에서 압승, 개발속도·자산에서 손해. v1→v2 경계라 자산 손해가 평소보다 작음.
- **B**: "두 마리 토끼"처럼 보이나 **2언어 backend 경계**가 가중치 높은 유지보수·Tauri에서 지속 손해. 어중간.
- **C**: 속도·자산·유지보수는 최고, 안정성·배포·Tauri는 평범. v2의 핵심 동인(안정성·단일배포)을 충족 못 함.

---

## 5. 권고 (제안)

**Option A (전면 Rust v2)를 권고한다.** 단 조건부:

근거:
1. v2가 어차피 전면 재작성 → 언어 전환의 한계비용이 가장 낮은 시점
2. v2의 3대 동인(daemon 안정성·단일 바이너리·Tauri)이 모두 Rust 강점과 정렬
3. 하이브리드(B)의 2언어 경계는 단기 이득 대비 영구 부채 → 기각
4. C는 v2 동인을 충족 못 함

조건(이게 안 맞으면 C로 회귀):
- (a) 사용자가 Rust 유지보수 의향 있음
- (b) 개발 속도 저하를 감수 (설계가 v0.6로 충분히 굳음)
- (c) web frontend가 React로 남는 것 수용 (Rust core + React = 정상적 구성)

### 위험 완화
- v1 TS는 **삭제하지 않고 `legacy/` 또는 git 태그로 보존** → 청사진·테스트 시나리오 참조
- Rust v2는 v1과 **동일 tavern.db 스키마** 사용 → 데이터 호환, 점진 이전 가능
- 첫 마일스톤(V2-P0: projects + 등록 + TUI 골격)을 Rust로 만들어 **개발 속도를 실측**한 뒤 본격 진행 결정 (early exit 지점)

---

## 6. 결정

> **(대기)** — 사용자 확정 후 Status를 Accepted/Rejected로 변경하고 v2-standalone.md에 언어 확정 반영.

선택지:
- [ ] A 전면 Rust v2 (권고)
- [ ] B 하이브리드
- [ ] C 전면 TS 유지

---

## 7. 결정 시 후속 작업
- **A 채택 시**: v1을 `git tag v1-typescript`로 보존 → Rust 워크스페이스(`Cargo.toml` workspace, crates/{core,brain,sidecar,tui,cli})로 v2 시작. v2-standalone.md의 패키지명(@luida/*)을 crate명으로 매핑.
- **B 채택 시**: `crates/luida-daemon` + 기존 TS. IPC 규약(tavern.db + JSON) 설계.
- **C 채택 시**: 현 구조로 V2-P0 착수.
