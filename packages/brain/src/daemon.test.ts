import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRepos, migrate, openDb } from '@luida/core';
import { runBrain } from './daemon';

let tempDir: string;
let dbPath: string;
let memoryDir: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-brain-daemon-'));
  dbPath = join(tempDir, 'tavern.db');
  memoryDir = join(tempDir, 'memory');
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

describe('runBrain (once mode)', () => {
  test('brain adventurer 자동 등록', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const repos = createRepos(db);

    const handle = await runBrain({
      reposOverride: repos,
      memoryDirOverride: memoryDir,
      once: true,
    });

    const brain = repos.adventurers.findByName('luida-brain');
    expect(brain).toBeTruthy();
    expect(brain?.role).toBe('brain');
    handle.stop();
    repos.close();
    db.close();
  });

  test('stuck quest 감지 → events + chronicle 기록', async () => {
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
    // 매우 오래된 running quest
    const qid = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: '오래된 작업',
      status: 'running',
    });
    // 강제로 updated_at을 오래된 값으로 설정
    db.prepare('UPDATE quests SET updated_at = ? WHERE id = ?').run(
      1_000_000_000,
      qid,
    );

    const handle = await runBrain({
      reposOverride: repos,
      memoryDirOverride: memoryDir,
      stuckThresholdMs: 1_000,
      once: true,
    });

    const result = await handle.tick();
    expect(result.stuckQuests).toContain(qid);

    const events = repos.events.recentSince(0, 10);
    const reviewFailed = events.find(
      (e) => e.kind === 'review_failed' && e.actor === 'luida-brain',
    );
    expect(reviewFailed).toBeTruthy();

    handle.stop();
    repos.close();
    db.close();
  });

  test('healthy quest는 stuck으로 보지 않음', async () => {
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
    repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: '최근 작업',
      status: 'running',
    });

    const handle = await runBrain({
      reposOverride: repos,
      memoryDirOverride: memoryDir,
      stuckThresholdMs: 60 * 60 * 1000,
      once: true,
    });

    const result = await handle.tick();
    expect(result.stuckQuests).toEqual([]);

    handle.stop();
    repos.close();
    db.close();
  });
});
