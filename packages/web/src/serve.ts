// Bun.serve 기반 정적 + API 서버.
//   - / → static (디자인 prototype, Babel-standalone로 in-browser JSX 컴파일)
//   - /api/* → tavern.db 조회 (Phase B에서 본격화)
//   - /events → SSE 라이브 갱신 (Phase B)
//
// Tauri 래핑 시: 같은 Bun 프로세스를 src-tauri에서 sidecar로 띄우거나,
// 또는 frontend는 tauri://localhost에서 fetch로 이 서버를 호출.

import { existsSync, readFileSync, statSync } from 'node:fs';
import { extname, join, normalize, resolve } from 'node:path';
import {
  type Repos,
  createRepos,
  openDb,
} from '@luida/core';

export type WebServeOpts = {
  port?: number;
  host?: string;
  staticDir?: string;
  dbPath?: string;
  /** 외부 repos 주입 시 자체 DB 오픈 안 함 (테스트) */
  reposOverride?: Repos;
};

export type WebServeHandle = {
  port: number;
  url: string;
  stop(): Promise<void>;
};

const MIME: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.jsx': 'application/javascript; charset=utf-8', // Babel-standalone가 브라우저에서 컴파일
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.woff2': 'font/woff2',
};

function defaultStaticDir(): string {
  return resolve(import.meta.dir, '..', 'static');
}

export async function runWebServer(
  opts: WebServeOpts = {},
): Promise<WebServeHandle> {
  const port = opts.port ?? 4321;
  const host = opts.host ?? '127.0.0.1';
  const staticDir = opts.staticDir ?? defaultStaticDir();

  const db = opts.reposOverride ? null : openDb(opts.dbPath);
  const repos = opts.reposOverride ?? createRepos(db!);

  const server = Bun.serve({
    port,
    hostname: host,
    fetch: async (req) => {
      const url = new URL(req.url);

      // API: tavern 상태 스냅샷
      if (url.pathname === '/api/snapshot') {
        const snap = {
          adventurers: repos.adventurers.list(),
          quests: repos.quests.listActive(),
          inmail: repos.inmail.tail(50),
          taken_at: Date.now(),
        };
        return new Response(JSON.stringify(snap), {
          headers: { 'content-type': 'application/json; charset=utf-8' },
        });
      }

      if (url.pathname === '/api/health') {
        return new Response('OK', { headers: { 'content-type': 'text/plain' } });
      }

      // Static (디자인 prototype)
      const requested = url.pathname === '/' ? '/Luida Tavern.html' : url.pathname;
      const decoded = decodeURIComponent(requested);
      const safe = normalize(decoded).replace(/^(\.\.[/\\])+/, '');
      const filePath = join(staticDir, safe);
      if (!filePath.startsWith(staticDir)) {
        return new Response('forbidden', { status: 403 });
      }

      if (!existsSync(filePath) || !statSync(filePath).isFile()) {
        return new Response('not found', { status: 404 });
      }

      const ext = extname(filePath).toLowerCase();
      const type = MIME[ext] ?? 'application/octet-stream';
      const body = readFileSync(filePath);
      return new Response(body, {
        headers: {
          'content-type': type,
          'cache-control': 'no-cache',
        },
      });
    },
  });

  const actualPort = server.port ?? port;
  return {
    port: actualPort,
    url: `http://${host}:${actualPort}`,
    async stop() {
      server.stop(true);
      if (db && !opts.reposOverride) {
        try {
          repos.close();
        } catch {
          // ignore
        }
        try {
          db.close();
        } catch {
          // ignore
        }
      }
    },
  };
}
