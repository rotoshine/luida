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
import { pollOnce } from './poll';

let tempDir: string;
let dbPath: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-sidecar-test-'));
  dbPath = join(tempDir, 'tavern.db');
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

async function setup(): Promise<{
  db: ReturnType<typeof openDb>;
  repos: ReturnType<typeof createRepos>;
  fakes: ReturnType<typeof createFakeIntegrations>;
}> {
  const db = openDb(dbPath);
  await migrate(db);
  const repos = createRepos(db);
  repos.adventurers.upsert({
    name: 'luida',
    workspace_id: 'ws-luida',
    surface_id: 'sf-luida',
    role: 'main',
  });
  repos.adventurers.upsert({
    name: 'agora',
    workspace_id: 'ws-agora',
    surface_id: 'sf-agora',
    role: 'worker',
  });
  return { db, repos, fakes: createFakeIntegrations() };
}

describe('pollOnce', () => {
  test('drains pending inmail and marks delivered', async () => {
    const { db, repos, fakes } = await setup();

    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'info',
      payload: { msg: 'hi' },
    });

    const n = await pollOnce({
      me: 'agora',
      target: { workspace_id: 'ws-agora', surface_id: 'sf-agora' },
      inmail: repos.inmail,
      cmux: fakes.cmux,
    });

    expect(n).toBe(1);
    expect(fakes.cmux.sent.length).toBe(1);
    expect(fakes.cmux.sent[0]?.target.surface_id).toBe('sf-agora');

    const n2 = await pollOnce({
      me: 'agora',
      target: { workspace_id: 'ws-agora', surface_id: 'sf-agora' },
      inmail: repos.inmail,
      cmux: fakes.cmux,
    });
    expect(n2).toBe(0);
    expect(fakes.cmux.sent.length).toBe(1);

    repos.close();
    db.close();
  });

  test('picks up broadcast (@all) for non-sender', async () => {
    const { db, repos, fakes } = await setup();

    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: '@all',
      kind: 'alert',
      payload: { msg: 'attention' },
    });

    const n = await pollOnce({
      me: 'agora',
      target: { workspace_id: 'ws-agora', surface_id: 'sf-agora' },
      inmail: repos.inmail,
      cmux: fakes.cmux,
    });
    expect(n).toBe(1);
    repos.close();
    db.close();
  });

  test('skips own broadcast', async () => {
    const { db, repos, fakes } = await setup();

    repos.inmail.enqueue({
      from_session: 'agora',
      to_session: '@all',
      kind: 'info',
      payload: { msg: 'self' },
    });

    const n = await pollOnce({
      me: 'agora',
      target: { workspace_id: 'ws-agora', surface_id: 'sf-agora' },
      inmail: repos.inmail,
      cmux: fakes.cmux,
    });
    expect(n).toBe(0);
    repos.close();
    db.close();
  });

  test('onMessage callback is invoked for each delivered inmail', async () => {
    const { db, repos, fakes } = await setup();

    for (let i = 0; i < 3; i++) {
      repos.inmail.enqueue({
        from_session: 'luida',
        to_session: 'agora',
        kind: 'info',
        payload: { i },
      });
    }

    const seen: number[] = [];
    await pollOnce({
      me: 'agora',
      target: { workspace_id: 'ws-agora', surface_id: 'sf-agora' },
      inmail: repos.inmail,
      cmux: fakes.cmux,
      onMessage: (msg) => {
        seen.push(msg.id);
      },
    });

    expect(seen.length).toBe(3);
    expect(fakes.cmux.sent.length).toBe(3);
    repos.close();
    db.close();
  });

  test('per-message try-catch: sendPrompt 실패해도 다음 메시지 계속, 실패 메시지는 재시도 가능', async () => {
    const { db, repos, fakes } = await setup();

    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'info',
      payload: { msg: 'A' },
    });
    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'info',
      payload: { msg: 'B' },
    });

    // 두 번째 메시지에서만 throw
    let count = 0;
    const flakyCmux = {
      sendPrompt: async () => {
        count += 1;
        if (count === 2) throw new Error('cmux down');
      },
      readScreen: async () => '',
    };

    const n = await pollOnce({
      me: 'agora',
      target: { workspace_id: 'ws-agora', surface_id: 'sf-agora' },
      inmail: repos.inmail,
      cmux: flakyCmux,
    });

    // 첫 번째만 delivered, 두 번째는 실패해서 pending에 남음
    expect(n).toBe(1);
    const pending = repos.inmail.pendingFor('agora');
    expect(pending.length).toBe(1);
    // 두 번째 메시지(payload B)가 pending
    expect(pending[0]?.payload).toContain('"B"');

    repos.close();
    db.close();
  });

  test('onMessage throw → 메시지는 delivered로 보존, 다음 메시지 계속', async () => {
    const { db, repos, fakes } = await setup();

    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'info',
      payload: { i: 0 },
    });
    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'info',
      payload: { i: 1 },
    });
    repos.inmail.enqueue({
      from_session: 'luida',
      to_session: 'agora',
      kind: 'info',
      payload: { i: 2 },
    });

    let calls = 0;
    const n = await pollOnce({
      me: 'agora',
      target: { workspace_id: 'ws-agora', surface_id: 'sf-agora' },
      inmail: repos.inmail,
      cmux: fakes.cmux,
      onMessage: async () => {
        calls += 1;
        if (calls === 2) throw new Error('boom');
      },
    });

    expect(n).toBe(3);
    expect(fakes.cmux.sent.length).toBe(3);
    expect(calls).toBe(3);
    expect(repos.inmail.pendingFor('agora').length).toBe(0);
    repos.close();
    db.close();
  });
});
