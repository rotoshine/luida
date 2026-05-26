import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRepos, migrate, openDb } from '@luida/core';
import {
  applyFollowUps,
  evaluatePostQuest,
  syncRelationshipsFromYaml,
} from './rules';

let tempDir: string;
let dbPath: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-brain-test-'));
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
  const repos = createRepos(db);
  // 모험가 시드
  for (const name of ['luida', 'agora', 'admin', 'kontrol']) {
    repos.adventurers.upsert({
      name,
      workspace_id: 'w',
      surface_id: 's',
      role: name === 'luida' ? 'main' : 'worker',
    });
  }
  return { db, repos };
}

describe('syncRelationshipsFromYaml', () => {
  test('inserts rules from yaml', async () => {
    const { db, repos } = await setup();
    const yaml = `
relationships:
  - name: agora-schema-to-admin
    from: agora
    trigger:
      kind: path_changed
      paths:
        - "prisma/**"
    to: admin
    action: auto_dispatch
    brief_template: "agora schema 변경 ({files})을 admin에 반영"
`;
    const r = syncRelationshipsFromYaml(repos.relationships, yaml);
    expect(r.added).toBe(1);
    expect(repos.relationships.listEnabled().length).toBe(1);
    repos.close();
    db.close();
  });

  test('같은 name 재싱크는 update(upsert)', async () => {
    const { db, repos } = await setup();
    const yaml = `
relationships:
  - name: dup
    from: agora
    trigger:
      kind: path_changed
      paths:
        - "**"
    to: admin
    action: auto_dispatch
`;
    const r1 = syncRelationshipsFromYaml(repos.relationships, yaml);
    expect(r1.added).toBe(1);
    expect(r1.updated).toBe(0);

    // 같은 name + 다른 to_session으로 재싱크 → upsert
    const yaml2 = `
relationships:
  - name: dup
    from: agora
    trigger:
      kind: path_changed
      paths:
        - "**"
    to: kontrol
    action: propose
`;
    const r2 = syncRelationshipsFromYaml(repos.relationships, yaml2);
    expect(r2.added).toBe(0);
    expect(r2.updated).toBe(1);

    const found = repos.relationships.findByName('dup');
    expect(found?.to_session).toBe('kontrol');
    expect(found?.action).toBe('propose');

    repos.close();
    db.close();
  });
});

describe('evaluatePostQuest', () => {
  test('path_changed: 매칭되면 auto_dispatch 후보', async () => {
    const { db, repos } = await setup();
    repos.relationships.insert({
      name: 'schema',
      from_session: 'agora',
      trigger_kind: 'path_changed',
      trigger_config: { paths: ['prisma/**'] },
      to_session: 'admin',
      action: 'auto_dispatch',
      brief_template: 'schema 변경({files}) admin 반영',
      source: 'human',
    });
    const qid = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'migrate',
      status: 'completed',
    });
    const result = evaluatePostQuest(repos, {
      quest: repos.quests.get(qid)!,
      changedFiles: ['prisma/schema.prisma', 'src/x.ts'],
    });
    expect(result.autoDispatch.length).toBe(1);
    expect(result.proposals.length).toBe(0);
    expect(result.autoDispatch[0]?.brief).toContain('prisma/schema.prisma');
    repos.close();
    db.close();
  });

  test('path_changed: 매칭 없으면 빈 결과', async () => {
    const { db, repos } = await setup();
    repos.relationships.insert({
      name: 'schema',
      from_session: 'agora',
      trigger_kind: 'path_changed',
      trigger_config: { paths: ['prisma/**'] },
      to_session: 'admin',
      action: 'auto_dispatch',
      source: 'human',
    });
    const qid = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'x',
      status: 'completed',
    });
    const result = evaluatePostQuest(repos, {
      quest: repos.quests.get(qid)!,
      changedFiles: ['docs/x.md'],
    });
    expect(result.autoDispatch.length).toBe(0);
    repos.close();
    db.close();
  });

  test('action=propose는 proposals로 분리', async () => {
    const { db, repos } = await setup();
    repos.relationships.insert({
      name: 'maybe',
      from_session: 'agora',
      trigger_kind: 'path_changed',
      trigger_config: { paths: ['**'] },
      to_session: 'admin',
      action: 'propose',
      source: 'human',
    });
    const qid = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'x',
      status: 'completed',
    });
    const result = evaluatePostQuest(repos, {
      quest: repos.quests.get(qid)!,
      changedFiles: ['any/file.ts'],
    });
    expect(result.proposals.length).toBe(1);
    expect(result.autoDispatch.length).toBe(0);
    repos.close();
    db.close();
  });

  test('enabled=0 룰은 평가 대상 아님', async () => {
    const { db, repos } = await setup();
    const id = repos.relationships.insert({
      name: 'off',
      from_session: 'agora',
      trigger_kind: 'path_changed',
      trigger_config: { paths: ['**'] },
      to_session: 'admin',
      action: 'auto_dispatch',
      source: 'human',
    });
    repos.relationships.setEnabled(id, false);
    const qid = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'x',
      status: 'completed',
    });
    const result = evaluatePostQuest(repos, {
      quest: repos.quests.get(qid)!,
      changedFiles: ['x'],
    });
    expect(result.autoDispatch.length).toBe(0);
    repos.close();
    db.close();
  });

  test('quest_completed 룰: status 매칭', async () => {
    const { db, repos } = await setup();
    repos.relationships.insert({
      name: 'every-done',
      from_session: 'agora',
      trigger_kind: 'quest_completed',
      trigger_config: { status: 'completed' },
      to_session: 'admin',
      action: 'auto_dispatch',
      source: 'human',
    });
    const completedId = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'x',
      status: 'completed',
    });
    expect(
      evaluatePostQuest(repos, {
        quest: repos.quests.get(completedId)!,
      }).autoDispatch.length,
    ).toBe(1);

    const failedId = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'y',
      status: 'failed',
    });
    expect(
      evaluatePostQuest(repos, {
        quest: repos.quests.get(failedId)!,
      }).autoDispatch.length,
    ).toBe(0);
    repos.close();
    db.close();
  });
});

describe('applyFollowUps', () => {
  test('auto_dispatch는 dispatch inmail 발행', async () => {
    const { db, repos } = await setup();
    const relId = repos.relationships.insert({
      name: 'schema',
      from_session: 'agora',
      trigger_kind: 'path_changed',
      trigger_config: { paths: ['**'] },
      to_session: 'admin',
      action: 'auto_dispatch',
      brief_template: 'admin 반영',
      source: 'human',
    });
    const qid = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'x',
      status: 'completed',
    });
    const result = evaluatePostQuest(repos, {
      quest: repos.quests.get(qid)!,
      changedFiles: ['file.ts'],
    });
    const counts = applyFollowUps(
      repos,
      'agora',
      { quest: repos.quests.get(qid)!, changedFiles: ['file.ts'] },
      result,
    );
    expect(counts.dispatched).toBe(1);
    expect(counts.proposed).toBe(0);
    const tail = repos.inmail.tail(5);
    const dispatched = tail.find((m) => m.kind === 'dispatch');
    expect(dispatched?.to_session).toBe('admin');
    void relId;
    repos.close();
    db.close();
  });

  test('같은 (quest, rule)에 대해 dedupe — 2번 호출해도 1건', async () => {
    const { db, repos } = await setup();
    repos.relationships.insert({
      name: 'schema',
      from_session: 'agora',
      trigger_kind: 'path_changed',
      trigger_config: { paths: ['**'] },
      to_session: 'admin',
      action: 'auto_dispatch',
      source: 'human',
    });
    const qid = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'x',
      status: 'completed',
    });
    const ctx = {
      quest: repos.quests.get(qid)!,
      changedFiles: ['file.ts'],
    };
    const r1 = applyFollowUps(repos, 'agora', ctx, evaluatePostQuest(repos, ctx));
    const r2 = applyFollowUps(repos, 'agora', ctx, evaluatePostQuest(repos, ctx));
    expect(r1.dispatched).toBe(1);
    expect(r2.dispatched).toBe(0);
    repos.close();
    db.close();
  });
});
