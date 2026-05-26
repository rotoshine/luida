// Bun.serve 기반 정적 + API 서버.
//   - /api/snapshot — tavern.db 1회 스냅샷 (JSON)
//   - /api/stream   — SSE 라이브 스트림 (1초 주기 스냅샷)
//   - /api/health   — 헬스체크
//   - 그 외 정적: Vite build의 dist/가 있으면 우선, 없으면 prototype static/
//
// Tauri 래핑 시: 같은 Bun 프로세스를 src-tauri sidecar로 띄우거나,
//                 frontend가 tauri://localhost에서 이 서버를 fetch.

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
  '.jsx': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.woff2': 'font/woff2',
};

function defaultStaticDir(): string {
  // Vite build 결과(dist/)가 있으면 우선, 없으면 prototype static/
  const distDir = resolve(import.meta.dir, '..', 'dist');
  if (existsSync(distDir)) return distDir;
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

  const snapshot = (): {
    adventurers: unknown[];
    quests: unknown[];
    inmail: unknown[];
    taken_at: number;
  } => ({
    adventurers: repos.adventurers.list(),
    quests: repos.quests.listActive(),
    inmail: repos.inmail.tail(50),
    taken_at: Date.now(),
  });

  const server = Bun.serve({
    port,
    hostname: host,
    fetch: async (req) => {
      const url = new URL(req.url);

      if (url.pathname === '/api/snapshot') {
        return new Response(JSON.stringify(snapshot()), {
          headers: { 'content-type': 'application/json; charset=utf-8' },
        });
      }

      if (url.pathname === '/api/health') {
        return new Response('OK', {
          headers: { 'content-type': 'text/plain' },
        });
      }

      // SSE: 1초 주기 스냅샷 스트리밍
      if (url.pathname === '/api/stream') {
        const stream = new ReadableStream({
          start(controller) {
            const encoder = new TextEncoder();
            const send = (): void => {
              try {
                controller.enqueue(
                  encoder.encode(`data: ${JSON.stringify(snapshot())}\n\n`),
                );
              } catch (err) {
                try {
                  controller.enqueue(
                    encoder.encode(
                      `event: error\ndata: ${JSON.stringify({
                        message: String(err),
                      })}\n\n`,
                    ),
                  );
                } catch {
                  // controller closed
                }
              }
            };
            send();
            const id = setInterval(send, 1000);
            req.signal.addEventListener('abort', () => {
              clearInterval(id);
              try {
                controller.close();
              } catch {
                // already closed
              }
            });
          },
        });
        return new Response(stream, {
          headers: {
            'content-type': 'text/event-stream; charset=utf-8',
            'cache-control': 'no-cache, no-transform',
            connection: 'keep-alive',
          },
        });
      }

      // Static
      const defaultIndex = existsSync(join(staticDir, 'index.html'))
        ? '/index.html'
        : '/Luida Tavern.html';
      const requested = url.pathname === '/' ? defaultIndex : url.pathname;
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
