import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRepos, migrate, openDb, type Repos } from '@luida/core';
import { runWebServer } from './serve';

let tempDir: string;
let dbPath: string;
let db: ReturnType<typeof openDb>;
let repos: Repos;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-web-'));
  dbPath = join(tempDir, 'tavern.db');
  db = openDb(dbPath);
  await migrate(db);
  repos = createRepos(db);
  repos.adventurers.upsert({
    name: 'agora',
    workspace_id: 'w',
    surface_id: 's',
    role: 'worker',
  });
  repos.adventurers.upsert({
    name: 'luida',
    workspace_id: 'w',
    surface_id: 's',
    role: 'main',
  });
});

afterEach(async () => {
  repos.close();
  db.close();
  await rm(tempDir, { recursive: true, force: true });
});

describe('web serve', () => {
  test('/api/health → 200 OK', async () => {
    const handle = await runWebServer({
      port: 0, // random
      reposOverride: repos,
    });
    try {
      const r = await fetch(`${handle.url}/api/health`);
      expect(r.status).toBe(200);
      expect(await r.text()).toBe('OK');
    } finally {
      await handle.stop();
    }
  });

  test('/api/snapshot 반환 — adventurers, quests, inmail 포함', async () => {
    repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: '시드 의뢰',
      status: 'running',
    });
    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'info',
      payload: { msg: 'hi' },
    });

    const handle = await runWebServer({
      port: 0,
      reposOverride: repos,
    });
    try {
      const r = await fetch(`${handle.url}/api/snapshot`);
      expect(r.status).toBe(200);
      const data = (await r.json()) as {
        adventurers: { name: string }[];
        quests: { brief: string }[];
        inmail: { kind: string }[];
        taken_at: number;
      };
      expect(data.adventurers.length).toBe(2);
      expect(data.quests.length).toBe(1);
      expect(data.quests[0]?.brief).toBe('시드 의뢰');
      expect(data.inmail.length).toBe(1);
      expect(data.taken_at).toBeGreaterThan(0);
    } finally {
      await handle.stop();
    }
  });

  test('/ → Luida Tavern.html 정적 응답', async () => {
    const handle = await runWebServer({
      port: 0,
      reposOverride: repos,
    });
    try {
      const r = await fetch(`${handle.url}/`);
      expect(r.status).toBe(200);
      const html = await r.text();
      expect(html).toContain('<title>');
      expect(html).toContain('Luida');
      expect(r.headers.get('content-type')).toContain('text/html');
    } finally {
      await handle.stop();
    }
  });

  test('Path traversal 차단', async () => {
    const handle = await runWebServer({
      port: 0,
      reposOverride: repos,
    });
    try {
      const r = await fetch(`${handle.url}/../../../etc/passwd`);
      // 404 또는 forbidden — 절대 200 + /etc/passwd 내용 안 됨
      expect([403, 404]).toContain(r.status);
    } finally {
      await handle.stop();
    }
  });
});
