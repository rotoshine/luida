# ADR-0002: 프론트엔드 · 브리지 아키텍처

| | |
|---|---|
| **Status** | **Accepted** (2026-05-28) |
| **선행** | ADR-0001 (v2 = Rust) |
| **관련** | v2-standalone.md §2, §15 |

---

## 1. 배경

ADR-0001로 v2 core는 Rust 확정. 남은 결정: **UI를 어떻게 띄우고 core와 통신하는가.**

요구:
- core(db·orchestration·brain·planner·runtimes)는 Rust로 안전성 확보
- **TUI**(터미널)와 **Tauri 기반 GUI**(데스크탑)를 **둘 다** 제공
- GUI는 Claude Design(React) 풀 충실도(픽셀폰트·CSS·애니메이션)

## 2. 결정

### 2.1 브리지 = Rust daemon + 로컬 HTTP/SSE (양 UI 클라이언트)
```
luida-server (Rust core daemon)
  - tavern.db(rusqlite) + orchestration + brain + planner + runtimes
  - 127.0.0.1 로컬 HTTP REST + SSE + command API
        │
        ├──── Tauri GUI (React)   ── HTTP/SSE 클라이언트 + 네이티브 윈도우/알림
        ├──── TUI (ratatui, Rust) ── 현재: core 직접 임베드 / 향후: HTTP 클라이언트로도 가능
        └──── (향후) Ink TUI       ── HTTP/SSE 클라이언트
```
- **core가 단일 진실.** GUI·Ink는 얇은 HTTP 클라이언트
- SSE로 라이브 갱신 (brain·hand-off와 정합)
- HTTP 프레임워크: **axum** (tokio 기반, SSE 지원)
- **타입 동기화**: `ts-rs` crate로 Rust serde 구조체 → TS 타입 자동 생성 (API 경계 드리프트 방지)

### 2.2 TUI = ratatui 유지 (당분간), Ink 전환은 후속 재검토
- V2-P0의 ratatui TUI 유지. Rust core를 **직접 임베드**(HTTP 불필요)해 가장 단순
- "프론트 React 통일"을 위한 Ink 전환은 **Tauri GUI 완성 후 비교 결정** (그때 ratatui vs Ink 실측)
- 정정: Ink도 터미널 셀 그리드라 ratatui보다 렌더가 자유롭지 않음. 렌더 자유도는 **Tauri GUI(픽셀)**의 몫. Ink의 이점은 "프론트 언어 통일"뿐 → GUI 완성 후 그 가치가 실제로 큰지 보고 결정

### 2.4 표면별 렌더 타깃 (혼동 방지)
| 표면 | 렌더 타깃 | 기술 |
|---|---|---|
| TUI | 터미널 셀 그리드 | ratatui(현재, Rust) / Ink(후속, JS) |
| GUI (Tauri) | 웹뷰(HTML/CSS) | **React DOM** — ratatui·Ink 아님 |

- **Tauri 내부에서는 ratatui도 Ink도 동작하지 않는다.** Tauri는 웹뷰라 React(DOM)를 로드 (`packages/web`).
- Ink·ratatui는 **터미널 전용** 렌더러. Ink의 유일한 자리는 TUI(ratatui 대체)이지 Tauri가 아니다.
- "React 통일"의 의미: TUI=Ink(React-for-터미널) + GUI=React(React-for-DOM) → 둘 다 JSX 멘탈모델(렌더 타깃은 다름). 이게 Ink 전환의 유일한 이점.

### 2.3 기각한 대안
- **Tauri native invoke + Ink는 CLI/직접DB**: 두 UI가 서버를 공유 못 해 라이브 동기화·hand-off 불리
- **napi-rs로 Rust를 Node 애드온화**: 빌드 복잡 + Tauri와 별개 경로 → 통일성 저하

## 3. 결과 (crate 영향)
- **신규** `luida-server` crate (axum) — HTTP REST + SSE + command. brain daemon 통합 가능
- `luida-tui` (ratatui) — 현재 core 직접 사용. 향후 server 클라이언트 전환 옵션
- `packages/web` (React) + `src-tauri` — GUI. luida-server의 HTTP 클라이언트
- core의 공개 도메인 타입에 `ts-rs` derive 부착 → `bindings/` TS 타입 생성

## 4. 트레이드오프
- (+) core 안전성(Rust) + 양 UI 제공 + GUI 풀 충실도 + 단일 진실 daemon
- (−) JSON API 타입을 Rust↔TS 동기화 (ts-rs로 자동화) + Ink 경로는 Bun 런타임 의존
- ratatui는 Rust라 브리지 없이 동작 → 현 단계 비용 0

## 5. 후속
- v2-standalone.md §2/§15에 daemon·client 모델 + `luida-server` crate 반영
- V2-P1(agents.json) 후 또는 병행하여 `luida-server` 골격 (HTTP /api/snapshot, /api/stream SSE) — v1 web backend 설계를 axum으로 이식
- Ink 전환 재검토 지점: Tauri GUI 첫 동작 후
