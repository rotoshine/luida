# @luida/web

Luida 웹/데스크탑 대시보드.

```
packages/web/
├── index.html              # Vite entry (production)
├── vite.config.ts
├── src/                    # Frontend (React + TSX, Vite로 빌드)
│   ├── main.tsx
│   ├── app.tsx
│   ├── live.tsx            # SSE 라이브 데이터 hook
│   ├── data.ts             # 토큰 + i18n + seed (fallback)
│   ├── primitives.tsx      # Window, DialogBox, MenuList, ...
│   ├── cards.tsx           # AdventurerCard, QuestRow, ...
│   ├── catalog.tsx         # 무드보드/토큰/컴포넌트 탭
│   ├── dashboard.tsx       # 메인 대시보드
│   └── tweaks-panel.tsx    # 디자인 tweaks 패널
├── src-server/             # Bun.serve 백엔드
│   ├── serve.ts            # /api/snapshot, /api/stream(SSE), /api/health
│   ├── serve.test.ts
│   └── index.ts
├── src-tauri/              # Tauri 데스크탑 래퍼 (Option α)
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── build.rs
│   └── src/main.rs
└── static/                 # 오리지널 prototype (Babel-standalone) — deprecated
    └── Luida Tavern.html
```

## 개발

### 백엔드 (별도 터미널)
```bash
luida web --port 4321
```

### 프런트엔드 dev (HMR)
```bash
cd packages/web
bun run dev          # Vite dev 4322, /api/* proxy → 4321
```

브라우저 → `http://localhost:4322`

### 프런트엔드 빌드
```bash
cd packages/web
bun run build        # dist/ 생성 (Bun.serve가 자동 우선 서빙)
luida web            # dist/ 로 서빙 (production)
```

### Tauri 데스크탑 빌드 (Rust 필요)
```bash
cd packages/web
cargo install tauri-cli --version "^2.0"
# 개발 모드
cargo tauri dev
# 배포 빌드
cargo tauri build    # → target/release/bundle/macos/Luida.app + .dmg
```

빌드 결과: `Luida.app`이 generated. 첫 실행 시 macOS Gatekeeper 우회 필요할 수 있음 (`xattr -dr com.apple.quarantine Luida.app`).

## 아키텍처

- **Frontend (React)**: 정적 데이터(seed)는 `data.ts`, 라이브 데이터는 `useLive()` hook이 `LiveProvider` context로 공급. `/api/snapshot` 초기 fetch + `/api/stream` SSE 구독.
- **Backend (Bun.serve)**: `~/.luida/tavern.db`를 read-only 관점으로 조회. `quest.dispatch` 같은 mutation은 MCP server를 사용.
- **Tauri**: 단순 윈도우 + frontendDist 로딩. 모든 비즈니스 로직은 frontend + Bun 백엔드에 있음. main.rs ~10줄.
