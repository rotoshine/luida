import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  createFakeIntegrations,
  createRepos,
  migrate,
  openDb,
} from '@luida/core';
import { runSidecar } from '../src/run';

let tempDir: string;
let dbPath: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-run-test-'));
  dbPath = join(tempDir, 'tavern.db');
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

describe('runSidecar (once mode)', () => {
  test('upserts adventurer + processes pending inmail end-to-end', async () => {
    // 사전 셋업: 'luida' 모험가 + dispatch inmail
    const db = openDb(dbPath);
    await migrate(db);
    const repos = createRepos(db);
    repos.adventurers.upsert({
      name: 'luida',
      workspace_id: 'wl',
      surface_id: 'sl',
      role: 'main',
    });
    repos.adventurers.upsert({
      name: 'agora',
      workspace_id: 'wa',
      surface_id: 'sa',
      role: 'worker',
    });
    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'dispatch',
      payload: { brief: '간단한 작업', branch: 'feat/once' },
    });
    db.close();

    const fakes = createFakeIntegrations([
      { kind: 'text', text: 'go' },
      { kind: 'result', success: true, summary: 'done' },
    ]);

    const result = await runSidecar({
      me: 'agora',
      repoPath: '/repos/agora',
      workspaceId: 'wa',
      surfaceId: 'sa',
      once: true,
      autoCreatePr: true,
      integrations: fakes,
      dbPath,
    });

    expect(result.processedOnce).toBe(1);
    expect(fakes.cmux.sent.length).toBe(1);
    expect(fakes.worktree.created.length).toBe(1);
    expect(fakes.worker.spawns.length).toBe(1);
    expect(fakes.vcs.calls.length).toBe(1);

    // adventurer가 등록되어 있어야 함
    const db2 = openDb(dbPath);
    const r2 = createRepos(db2);
    const adv = r2.adventurers.findByName('agora');
    expect(adv?.workspace_id).toBe('wa');
    expect(adv?.repo_path).toBe('/repos/agora');
    db2.close();
  });
});
