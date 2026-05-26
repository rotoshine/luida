import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRepos, migrate, openDb } from '@luida/core';
import { loadSnapshot } from './load';

let tempDir: string;
let dbPath: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-ui-load-test-'));
  dbPath = join(tempDir, 'tavern.db');
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

describe('loadSnapshot', () => {
  test('empty DB returns empty snapshot', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const repos = createRepos(db);
    const snap = loadSnapshot(repos);
    expect(snap.adventurers).toEqual([]);
    expect(snap.activeQuests).toEqual([]);
    expect(snap.recentInmail).toEqual([]);
    expect(snap.questCountByAdventurer.size).toBe(0);
    repos.close();
    db.close();
  });

  test('populated DB returns adventurers, active quests, inmail', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const repos = createRepos(db);
    repos.adventurers.upsert({
      name: 'luida',
      workspace_id: 'w',
      surface_id: 's',
      role: 'main',
    });
    repos.adventurers.upsert({
      name: 'agora',
      workspace_id: 'w',
      surface_id: 's',
      role: 'worker',
    });
    const q1 = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'a',
      status: 'running',
    });
    repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'b',
      status: 'running',
    });
    repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'done',
      status: 'completed',
    });
    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'info',
      payload: { msg: 'hi' },
    });

    const snap = loadSnapshot(repos);
    expect(snap.adventurers.length).toBe(2);
    expect(snap.activeQuests.length).toBe(2); // 완료 1건 제외
    const ids = snap.activeQuests.map((q) => q.id).sort();
    expect(ids).toEqual([q1, q1 + 1].sort());
    expect(snap.questCountByAdventurer.get('agora')).toBe(2);
    expect(snap.recentInmail.length).toBe(1);

    repos.close();
    db.close();
  });
});
