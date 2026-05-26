import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Vite dev server는 4322번 (Bun.serve가 4321을 쓰기 때문).
// API 호출(/api/*)은 dev 모드에서 Bun.serve로 proxy.
// 빌드 결과는 dist/로 떨어지며 Tauri 또는 정적 호스팅이 이 디렉터리를 직접 서빙.
export default defineConfig({
  plugins: [react()],
  root: '.',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
  },
  server: {
    port: 4322,
    strictPort: true,
    proxy: {
      '/api': 'http://127.0.0.1:4321',
    },
  },
  esbuild: {
    // 디자인 prototype의 ASCII 픽셀 폰트 환경에 맞춰 minify 약하게
    legalComments: 'none',
  },
});
