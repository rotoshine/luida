// Luida Tauri shim — Option α minimum.
//
// 책임:
//   1. 단일 데스크탑 윈도우를 띄우고
//   2. 빌드된 Vite frontend(../dist)를 로드한다.
//
// API 호출(/api/*)은 Bun.serve(`luida web` 또는 `luida brain start --with-web` 향후)가
// 별도 프로세스에서 127.0.0.1:4321을 listen하고 있다고 가정.
// frontend는 EventSource('/api/stream')로 SSE 연결.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
  tauri::Builder::default()
    .setup(|_app| Ok(()))
    .run(tauri::generate_context!())
    .expect("error while running luida tauri app");
}
