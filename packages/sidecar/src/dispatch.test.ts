import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  type Inmail,
  type WorkerStreamEvent,
  createFakeIntegrations,
  createRepos,
  migrate,
  openDb,
} from '@luida/core';
import { defaultBranch, handleDispatch, parsePayload } from './dispatch';

let tempDir: string;
let dbPath: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-dispatch-test-'));
  dbPath = join(tempDir, 'tavern.db');
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

async function setup(
  workerScript?: WorkerStreamEvent[],
  briefOverride?: string,
): Promise<{
  db: ReturnType<typeof openDb>;
  repos: ReturnType<typeof createRepos>;
  fakes: ReturnType<typeof createFakeIntegrations>;
  msg: Inmail;
}> {
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

  repos.inmail.enqueue({
    from_session: 'luida',
    to_session: 'agora',
    kind: 'dispatch',
    payload: {
      brief: briefOverride ?? '스키마 마이그레이션',
      branch: 'feat/schema',
    },
  });
  const msg = repos.inmail.tail(1)[0]!;

  return { db, repos, fakes: createFakeIntegrations(workerScript), msg };
}

describe('handleDispatch', () => {
  test('success path with autoCreatePr=true creates worktree, runs worker, opens PR, sends ack', async () => {
    const { db, repos, fakes, msg } = await setup([
      { kind: 'text', text: 'starting work' },
      { kind: 'tool_use', name: 'Write', input: {} },
      { kind: 'text', text: 'almost done' },
      { kind: 'result', success: true, summary: 'all good' },
    ]);

    const result = await handleDispatch(msg, {
      me: 'agora',
      repoPath: '/repos/agora',
      quests: repos.quests,
      inmail: repos.inmail,
      events: repos.events,
      integrations: fakes,
      autoCreatePr: true,
    });

    expect(result.success).toBe(true);
    expect(result.prUrl).toContain('example.test/pr/');

    const quest = repos.quests.get(result.questId);
    expect(quest?.status).toBe('completed');
    expect(quest?.pr_url).toBe(result.prUrl);
    expect(quest?.branch).toBe('feat/schema');

    expect(fakes.worktree.created.length).toBe(1);
    expect(fakes.worker.spawns.length).toBe(1);
    expect(fakes.vcs.calls.length).toBe(1);
    // head가 PR 호출에 포함되었는지
    expect(fakes.vcs.calls[0]?.head).toBe('feat/schema');

    const tail = repos.inmail.tail(3);
    const ack = tail.find((m) => m.kind === 'ack');
    expect(ack).toBeDefined();
    expect(ack?.to_session).toBe('luida');
    expect(ack?.reply_to).toBe(msg.id);
    repos.close();
    db.close();
  });

  test('failure path: worker reports !success → status=failed, ack with success=false', async () => {
    const { db, repos, fakes, msg } = await setup([
      { kind: 'text', text: 'starting' },
      { kind: 'error', message: 'typecheck failed' },
      { kind: 'result', success: false, summary: 'gave up' },
    ]);

    const result = await handleDispatch(msg, {
      me: 'agora',
      repoPath: '/repos/agora',
      quests: repos.quests,
      inmail: repos.inmail,
      events: repos.events,
      integrations: fakes,
      autoCreatePr: true,
    });

    expect(result.success).toBe(false);
    expect(result.prUrl).toBeNull();
    expect(repos.quests.get(result.questId)?.status).toBe('failed');
    expect(fakes.vcs.calls.length).toBe(0);
    repos.close();
    db.close();
  });

  test('needs_approval path: autoCreatePr=false leaves status=needs_approval + proposal inmail', async () => {
    const { db, repos, fakes, msg } = await setup([
      { kind: 'text', text: 'starting' },
      { kind: 'result', success: true, summary: 'ready for review' },
    ]);

    const result = await handleDispatch(msg, {
      me: 'agora',
      repoPath: '/repos/agora',
      quests: repos.quests,
      inmail: repos.inmail,
      events: repos.events,
      integrations: fakes,
      autoCreatePr: false,
    });

    expect(result.success).toBe(true);
    expect(result.prUrl).toBeNull();
    expect(repos.quests.get(result.questId)?.status).toBe('needs_approval');
    expect(fakes.vcs.calls.length).toBe(0);

    const tail = repos.inmail.tail(2);
    const prop = tail.find((m) => m.kind === 'proposal');
    expect(prop).toBeDefined();
    expect(prop?.to_session).toBe('luida');
    repos.close();
    db.close();
  });

  test('events table records key milestones', async () => {
    const { db, repos, fakes, msg } = await setup([
      { kind: 'tool_use', name: 'Edit', input: {} },
      { kind: 'result', success: true },
    ]);

    await handleDispatch(msg, {
      me: 'agora',
      repoPath: '/repos/agora',
      quests: repos.quests,
      inmail: repos.inmail,
      events: repos.events,
      integrations: fakes,
      autoCreatePr: true,
    });

    const kinds = repos.events.recentSince(0, 100).map((e) => e.kind);
    expect(kinds).toContain('quest_dispatched');
    expect(kinds).toContain('tool_used');
    expect(kinds).toContain('review_passed');
    expect(kinds).toContain('pr_created');
    repos.close();
    db.close();
  });

  test('worker가 result 이벤트 없이 종료 → failed로 분류 + lastError를 ack에 포함', async () => {
    const { db, repos, fakes, msg } = await setup([
      { kind: 'text', text: 'started but crashed' },
      { kind: 'error', message: 'segfault' },
      // no 'result'
    ]);

    const result = await handleDispatch(msg, {
      me: 'agora',
      repoPath: '/repos/agora',
      quests: repos.quests,
      inmail: repos.inmail,
      events: repos.events,
      integrations: fakes,
      autoCreatePr: true,
    });

    expect(result.success).toBe(false);
    expect(repos.quests.get(result.questId)?.status).toBe('failed');
    expect(fakes.vcs.calls.length).toBe(0);

    const tail = repos.inmail.tail(2);
    const ack = tail.find((m) => m.kind === 'ack');
    const ackPayload = JSON.parse(ack?.payload ?? '{}');
    expect(ackPayload.success).toBe(false);
    expect(ackPayload.summary).toContain('segfault');
    repos.close();
    db.close();
  });

  test('빈 brief는 quest 생성 없이 즉시 ack(failed)로 종료', async () => {
    const { db, repos, fakes, msg } = await setup(
      [{ kind: 'result', success: true }],
      '   ', // whitespace-only brief
    );

    const result = await handleDispatch(msg, {
      me: 'agora',
      repoPath: '/repos/agora',
      quests: repos.quests,
      inmail: repos.inmail,
      events: repos.events,
      integrations: fakes,
      autoCreatePr: true,
    });

    expect(result.success).toBe(false);
    expect(result.questId).toBe(-1);
    expect(fakes.worktree.created.length).toBe(0);
    expect(fakes.worker.spawns.length).toBe(0);

    const ack = repos.inmail.tail(2).find((m) => m.kind === 'ack');
    expect(ack).toBeDefined();
    const p = JSON.parse(ack?.payload ?? '{}');
    expect(p.summary).toContain('missing brief');
    repos.close();
    db.close();
  });

  test('worktree 생성 실패 시 quest=failed로 마킹 + ack 발송 + throw 안 함', async () => {
    const { db, repos, fakes, msg } = await setup([
      { kind: 'result', success: true },
    ]);

    // worktree.create를 throw하도록 monkey-patch
    fakes.worktree.create = async () => {
      throw new Error('disk full');
    };

    const result = await handleDispatch(msg, {
      me: 'agora',
      repoPath: '/repos/agora',
      quests: repos.quests,
      inmail: repos.inmail,
      events: repos.events,
      integrations: fakes,
      autoCreatePr: true,
    });

    expect(result.success).toBe(false);
    expect(result.error).toContain('disk full');
    expect(repos.quests.get(result.questId)?.status).toBe('failed');

    const ack = repos.inmail.tail(2).find((m) => m.kind === 'ack');
    expect(ack).toBeDefined();
    const p = JSON.parse(ack?.payload ?? '{}');
    expect(p.success).toBe(false);
    expect(p.summary).toContain('disk full');
    repos.close();
    db.close();
  });
});

describe('parsePayload', () => {
  test('parses valid object', () => {
    expect(parsePayload(JSON.stringify({ brief: 'b', branch: 'br' }))).toEqual({
      brief: 'b',
      branch: 'br',
      base: undefined,
      pr_title: undefined,
    });
  });
  test('non-JSON raw → brief=raw', () => {
    expect(parsePayload('plain text')).toEqual({ brief: 'plain text' });
  });
  test('null/numeric JSON → brief stringified', () => {
    expect(parsePayload('null').brief).toBe('');
    expect(parsePayload('42').brief).toBe('42');
  });
  test('missing brief returns empty', () => {
    expect(parsePayload(JSON.stringify({ branch: 'x' })).brief).toBe('');
  });
});

describe('defaultBranch', () => {
  test('sanitizes from_session', () => {
    expect(defaultBranch('community-web-agora', 5)).toBe(
      'luida/community-web-agora-quest-5',
    );
  });
  test('replaces unsafe chars and collapses dashes', () => {
    expect(defaultBranch('a@b/c d', 1)).toBe('luida/a-b-c-d-quest-1');
  });
});
