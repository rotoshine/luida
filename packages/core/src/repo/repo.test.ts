import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { migrate, openDb } from '../db';
import { createRepos } from './index';

let tempDir: string;
let dbPath: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-repo-test-'));
  dbPath = join(tempDir, 'tavern.db');
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

async function setup(): Promise<{
  db: ReturnType<typeof openDb>;
  repos: ReturnType<typeof createRepos>;
}> {
  const db = openDb(dbPath);
  await migrate(db);
  return { db, repos: createRepos(db) };
}

describe('AdventurerRepo', () => {
  test('upsert overwrites existing row fields', async () => {
    const { db, repos } = await setup();
    repos.adventurers.upsert({
      name: 'agora',
      workspace_id: 'ws1',
      surface_id: 'sf1',
      role: 'worker',
      status: 'busy',
      pid: 100,
    });
    repos.adventurers.upsert({
      name: 'agora',
      workspace_id: 'ws2', // 변경
      surface_id: 'sf2', // 변경
      role: 'worker',
      status: 'idle', // busy → idle
      pid: 200,
    });
    const adv = repos.adventurers.findByName('agora');
    expect(adv?.workspace_id).toBe('ws2');
    expect(adv?.surface_id).toBe('sf2');
    expect(adv?.status).toBe('idle');
    expect(adv?.pid).toBe(200);
    repos.close();
    db.close();
  });

  test('list returns sorted by name', async () => {
    const { db, repos } = await setup();
    repos.adventurers.upsert({ name: 'b', workspace_id: 'w', surface_id: 's', role: 'worker' });
    repos.adventurers.upsert({ name: 'a', workspace_id: 'w', surface_id: 's', role: 'main' });
    repos.adventurers.upsert({ name: 'c', workspace_id: 'w', surface_id: 's', role: 'worker' });
    expect(repos.adventurers.list().map((a) => a.name)).toEqual(['a', 'b', 'c']);
    repos.close();
    db.close();
  });

  test('updateStatus changes status and bumps last_seen', async () => {
    const { db, repos } = await setup();
    repos.adventurers.upsert({
      name: 'x',
      workspace_id: 'w',
      surface_id: 's',
      role: 'worker',
    });
    const before = repos.adventurers.findByName('x')?.last_seen ?? 0;
    await Bun.sleep(2);
    repos.adventurers.updateStatus('x', 'busy');
    const after = repos.adventurers.findByName('x');
    expect(after?.status).toBe('busy');
    expect((after?.last_seen ?? 0)).toBeGreaterThan(before);
    repos.close();
    db.close();
  });
});

describe('InmailRepo', () => {
  test('enqueue with dedupe_key prevents duplicate', async () => {
    const { db, repos } = await setup();
    const r1 = repos.inmail.enqueue({
      from_session: 'a',
      to_session: 'b',
      kind: 'info',
      payload: {},
      dedupe_key: 'k1',
    });
    const r2 = repos.inmail.enqueue({
      from_session: 'a',
      to_session: 'b',
      kind: 'info',
      payload: {},
      dedupe_key: 'k1',
    });
    expect(r1.inserted).toBe(true);
    expect(r1.id).not.toBeNull();
    expect(r2.inserted).toBe(false);
    expect(r2.id).toBeNull();
    repos.close();
    db.close();
  });

  test('enqueue rejects dispatch to broadcast address', async () => {
    const { db, repos } = await setup();
    expect(() =>
      repos.inmail.enqueue({
        from_session: 'luida',
        to_session: '@all',
        kind: 'dispatch',
        payload: { brief: 'x' },
      }),
    ).toThrow();
    // non-dispatch는 broadcast OK
    const ok = repos.inmail.enqueue({
      from_session: 'luida',
      to_session: '@all',
      kind: 'alert',
      payload: { msg: 'attention' },
    });
    expect(ok.inserted).toBe(true);
    repos.close();
    db.close();
  });

  test('pendingFor includes own direct + broadcasts, excludes own broadcast', async () => {
    const { db, repos } = await setup();
    // seed adventurers for FK on quests (not strictly needed here)
    repos.adventurers.upsert({ name: 'a', workspace_id: 'w', surface_id: 's', role: 'worker' });
    repos.adventurers.upsert({ name: 'b', workspace_id: 'w', surface_id: 's', role: 'worker' });
    repos.inmail.enqueue({ from_session: 'a', to_session: 'b', kind: 'info', payload: {} });
    repos.inmail.enqueue({ from_session: 'a', to_session: '@all', kind: 'alert', payload: {} });
    repos.inmail.enqueue({ from_session: 'b', to_session: '@all', kind: 'info', payload: {} });

    const forB = repos.inmail.pendingFor('b');
    expect(forB.length).toBe(2); // direct + a's broadcast (not b's own)
    expect(forB.every((m) => m.from_session !== 'b')).toBe(true);

    repos.close();
    db.close();
  });

  test('markDelivered removes from pending', async () => {
    const { db, repos } = await setup();
    const e = repos.inmail.enqueue({
      from_session: 'a',
      to_session: 'b',
      kind: 'info',
      payload: {},
    });
    repos.inmail.markDelivered(e.id!);
    expect(repos.inmail.pendingFor('b').length).toBe(0);
    repos.close();
    db.close();
  });
});

describe('Repos.close', () => {
  test('finalize allows db.close without error', async () => {
    const { db, repos } = await setup();
    // 몇 가지 쿼리 실행해 statement 사용
    repos.adventurers.upsert({
      name: 'x',
      workspace_id: 'w',
      surface_id: 's',
      role: 'worker',
    });
    repos.inmail.enqueue({
      from_session: 'x',
      to_session: 'x',
      kind: 'info',
      payload: {},
    });
    repos.close();
    expect(() => db.close()).not.toThrow();
  });
});
