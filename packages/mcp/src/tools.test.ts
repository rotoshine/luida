import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { MemoryStore } from '@luida/brain';
import { createRepos, migrate, openDb, type Repos } from '@luida/core';
import {
  ALL_TOOLS,
  adventurerList,
  memoryRecall,
  memoryRecord,
  questDispatch,
  questGet,
  questList,
} from './tools';

let tempDir: string;
let dbPath: string;
let memoryDir: string;
let db: ReturnType<typeof openDb>;
let repos: Repos;
let memory: MemoryStore;
const me = 'luida';

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-mcp-'));
  dbPath = join(tempDir, 'tavern.db');
  memoryDir = join(tempDir, 'memory');
  db = openDb(dbPath);
  await migrate(db);
  repos = createRepos(db);
  memory = new MemoryStore(memoryDir);
  // 모험가 시드
  for (const n of ['luida', 'agora', 'admin']) {
    repos.adventurers.upsert({
      name: n,
      workspace_id: 'w',
      surface_id: 's',
      role: n === 'luida' ? 'main' : 'worker',
    });
  }
});

afterEach(async () => {
  repos.close();
  db.close();
  await rm(tempDir, { recursive: true, force: true });
});

describe('quest.list', () => {
  test('active quest만 반환', async () => {
    repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'a',
      status: 'running',
    });
    repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'b',
      status: 'completed',
    });
    const r = await questList.handler({}, { repos, memory, me });
    expect(r.quests.length).toBe(1);
    expect(r.quests[0]?.brief).toBe('a');
  });

  test('to 필터', async () => {
    repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'for agora',
      status: 'running',
    });
    repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'admin',
      brief: 'for admin',
      status: 'running',
    });
    const r = await questList.handler({ to: 'admin' }, { repos, memory, me });
    expect(r.quests.length).toBe(1);
    expect(r.quests[0]?.dispatched_to).toBe('admin');
  });
});

describe('quest.get', () => {
  test('id로 조회', async () => {
    const id = repos.quests.insert({
      dispatched_by: 'luida',
      dispatched_to: 'agora',
      brief: 'x',
      status: 'pending',
    });
    const r = await questGet.handler({ id }, { repos, memory, me });
    expect(r.quest?.brief).toBe('x');
  });

  test('없는 id는 null', async () => {
    const r = await questGet.handler({ id: 999 }, { repos, memory, me });
    expect(r.quest).toBeNull();
  });
});

describe('quest.dispatch', () => {
  test('inmail 발행', async () => {
    const r = await questDispatch.handler(
      { to: 'agora', brief: '스키마 작업' },
      { repos, memory, me },
    );
    expect(r.inserted).toBe(true);
    expect(r.inmail_id).toBeGreaterThan(0);
    const tail = repos.inmail.tail(5);
    const d = tail.find((m) => m.kind === 'dispatch');
    expect(d?.to_session).toBe('agora');
    expect(d?.from_session).toBe('luida');
  });

  test('빈 brief는 throw', () => {
    expect(() =>
      questDispatch.handler(
        { to: 'agora', brief: '   ' },
        { repos, memory, me },
      ),
    ).toThrow();
  });

  test('broadcast 주소(@all)는 enqueue 단에서 거부', () => {
    expect(() =>
      questDispatch.handler(
        { to: '@all', brief: 'x' },
        { repos, memory, me },
      ),
    ).toThrow();
  });
});

describe('adventurer.list', () => {
  test('등록된 모험가 전부 반환', async () => {
    const r = await adventurerList.handler({}, { repos, memory, me });
    const names = r.adventurers.map((a) => a.name).sort();
    expect(names).toEqual(['admin', 'agora', 'luida']);
  });
});

describe('memory.recall + memory.record', () => {
  test('record chronicle → recall 반영', async () => {
    await memoryRecord.handler(
      { type: 'chronicle', content: '오늘의 일지' },
      { repos, memory, me },
    );
    const r = await memoryRecall.handler(
      { scope: 'chronicle' },
      { repos, memory, me },
    );
    expect(r.result.chronicle).toContain('오늘의 일지');
  });

  test('record project + recall project', async () => {
    await memoryRecord.handler(
      { type: 'project', name: 'agora', content: '# agora 메모' },
      { repos, memory, me },
    );
    const r = await memoryRecall.handler(
      { scope: 'project', project: 'agora' },
      { repos, memory, me },
    );
    expect(r.result.project).toContain('agora 메모');
  });

  test('record without name throws for project', () => {
    expect(() =>
      memoryRecord.handler(
        { type: 'project', content: 'x' },
        { repos, memory, me },
      ),
    ).toThrow();
  });
});

describe('ALL_TOOLS', () => {
  test('이름이 모두 고유', () => {
    const names = ALL_TOOLS.map((t) => t.name);
    expect(new Set(names).size).toBe(names.length);
  });
});
