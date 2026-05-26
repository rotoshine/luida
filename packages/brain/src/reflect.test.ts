import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRepos, migrate, openDb, type Repos } from '@luida/core';
import { MemoryStore } from './memory';
import {
  analyzeEvents,
  promotePattern,
  reflect,
  renderPatternMarkdown,
  type PatternCandidate,
} from './reflect';

let tempDir: string;
let dbPath: string;
let memoryDir: string;
let db: ReturnType<typeof openDb>;
let repos: Repos;
let memory: MemoryStore;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-reflect-'));
  dbPath = join(tempDir, 'tavern.db');
  memoryDir = join(tempDir, 'memory');
  db = openDb(dbPath);
  await migrate(db);
  repos = createRepos(db);
  memory = new MemoryStore(memoryDir);
  for (const n of ['luida', 'luida-brain', 'agora', 'admin']) {
    repos.adventurers.upsert({
      name: n,
      workspace_id: 'w',
      surface_id: 's',
      role: n === 'luida' ? 'main' : n === 'luida-brain' ? 'brain' : 'worker',
    });
  }
});

afterEach(async () => {
  repos.close();
  db.close();
  await rm(tempDir, { recursive: true, force: true });
});

function seedDispatches(
  from: string,
  to: string,
  count: number,
  withPr = false,
): void {
  for (let i = 0; i < count; i++) {
    repos.events.record({
      actor: to,
      kind: 'quest_dispatched',
      payload: { from, inmail_id: i + 1 },
    });
    if (withPr) {
      repos.events.record({
        actor: to,
        kind: 'pr_created',
        payload: { url: `https://x/pr/${i + 1}` },
      });
    }
  }
}

describe('analyzeEvents', () => {
  test('빈 events → 빈 결과', () => {
    expect(analyzeEvents([], 3)).toEqual([]);
  });

  test('minSamples 미달은 제외', () => {
    seedDispatches('luida', 'agora', 2);
    const events = repos.events.recentSince(0, 100);
    expect(analyzeEvents(events, 3)).toEqual([]);
  });

  test('minSamples 도달은 candidate', () => {
    seedDispatches('luida', 'agora', 3);
    const events = repos.events.recentSince(0, 100);
    const c = analyzeEvents(events, 3);
    expect(c.length).toBe(1);
    expect(c[0]?.from).toBe('luida');
    expect(c[0]?.to).toBe('agora');
    expect(c[0]?.evidence).toBe(3);
  });

  test('pr_created 함께 있으면 신뢰도 boost', () => {
    seedDispatches('luida', 'agora', 3, false);
    const withoutPr = analyzeEvents(repos.events.recentSince(0, 100), 3)[0]!;

    // 동일 DB에서 다음 시드는 confidence를 비교 위해 별도 함수로
    seedDispatches('luida', 'admin', 3, true);
    const cs = analyzeEvents(repos.events.recentSince(0, 100), 3);
    const withPr = cs.find((c) => c.to === 'admin')!;
    expect(withPr.confidence).toBeGreaterThan(withoutPr.confidence);
  });

  test('결과는 신뢰도 내림차순', () => {
    seedDispatches('luida', 'agora', 8);
    seedDispatches('luida', 'admin', 3);
    const cs = analyzeEvents(repos.events.recentSince(0, 100), 3);
    expect(cs[0]?.to).toBe('agora');
    expect(cs[1]?.to).toBe('admin');
  });
});

describe('renderPatternMarkdown', () => {
  test('필수 섹션 포함', () => {
    const c: PatternCandidate = {
      id: 'luida-to-agora',
      topic: 'luida → agora 연쇄',
      from: 'luida',
      to: 'agora',
      confidence: 0.7,
      evidence: 5,
      proposedBriefTemplate: 'test brief',
    };
    const md = renderPatternMarkdown(c, 1779782400000);
    expect(md).toContain('# 패턴 후보');
    expect(md).toContain('신뢰도: 7.0 / 10');
    expect(md).toContain('근거 이벤트: 5건');
    expect(md).toContain('```yaml');
    expect(md).toContain('luida promote-pattern luida-to-agora');
  });
});

describe('reflect', () => {
  test('패턴 후보 → markdown + proposal inmail', async () => {
    seedDispatches('luida', 'agora', 5);
    const r = await reflect(repos, memory, { minSamples: 3, minConfidence: 0.4 });
    expect(r.candidates.length).toBe(1);
    expect(r.written.length).toBe(1);
    expect(r.proposed).toBe(1);

    // proposal inmail이 luida에게 갔는지
    const proposals = repos.inmail.tail(5).filter((m) => m.kind === 'proposal');
    expect(proposals.length).toBe(1);
    expect(proposals[0]?.to_session).toBe('luida');
    expect(proposals[0]?.from_session).toBe('luida-brain');

    // 같은 reflect 두 번 → dedupe로 추가 proposal 없음
    const r2 = await reflect(repos, memory, { minSamples: 3, minConfidence: 0.4 });
    expect(r2.proposed).toBe(0);
  });

  test('신뢰도 미달은 출력 안 함', async () => {
    seedDispatches('luida', 'agora', 1);
    const r = await reflect(repos, memory, {
      minSamples: 1,
      minConfidence: 0.95,
    });
    expect(r.candidates.length).toBe(0);
    expect(r.written.length).toBe(0);
  });

  test('chronicle에 자동 기록', async () => {
    seedDispatches('luida', 'agora', 4);
    await reflect(repos, memory, { minSamples: 3 });
    const rec = memory.recall('chronicle');
    expect(rec.chronicle).toContain('reflect');
    expect(rec.chronicle).toContain('luida-to-agora');
  });
});

describe('promotePattern', () => {
  test('relationships row 생성 + source=learned-promoted, 기본 disabled+propose', () => {
    const candidate: PatternCandidate = {
      id: 'luida-to-agora',
      topic: 'x',
      from: 'luida',
      to: 'agora',
      confidence: 0.8,
      evidence: 5,
      proposedBriefTemplate: 'tmpl',
    };
    const r = promotePattern(repos.relationships, candidate);
    expect(r.promoted).toBe(true);
    const rel = repos.relationships.findByName('luida-to-agora');
    expect(rel?.source).toBe('learned-promoted');
    expect(rel?.confidence).toBe(0.8);
    // Phase 5 C1: 기본은 비활성 + propose (사용자 검토 게이트)
    expect(rel?.enabled).toBe(0);
    expect(rel?.action).toBe('propose');
  });

  test('promotePattern({activate: true})는 즉시 활성 + auto_dispatch', () => {
    const candidate: PatternCandidate = {
      id: 'luida-to-admin-active',
      topic: 'x',
      from: 'luida',
      to: 'admin',
      confidence: 0.9,
      evidence: 5,
      proposedBriefTemplate: 'tmpl',
    };
    promotePattern(repos.relationships, candidate, { activate: true });
    const rel = repos.relationships.findByName('luida-to-admin-active');
    expect(rel?.enabled).toBe(1);
    expect(rel?.action).toBe('auto_dispatch');
  });

  test('재호출은 upsert (created=false)', () => {
    const candidate: PatternCandidate = {
      id: 'luida-to-agora',
      topic: 'x',
      from: 'luida',
      to: 'agora',
      confidence: 0.8,
      evidence: 5,
      proposedBriefTemplate: 'tmpl',
    };
    promotePattern(repos.relationships, candidate);
    const r2 = promotePattern(repos.relationships, {
      ...candidate,
      confidence: 0.9,
    });
    expect(r2.promoted).toBe(false);
    const rel = repos.relationships.findByName('luida-to-agora');
    expect(rel?.confidence).toBe(0.9);
  });
});
