import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { MemoryStore } from './memory';

let tempDir: string;
let store: MemoryStore;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-memory-'));
  store = new MemoryStore(tempDir);
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

describe('MemoryStore', () => {
  test('appendChronicle adds timestamp + content', () => {
    store.appendChronicle('첫 항목', 1700000000000);
    const r = store.recall('chronicle');
    expect(r.chronicle).toContain('## 2023-11-14');
    expect(r.chronicle).toContain('첫 항목');
  });

  test('multiple appends are preserved', () => {
    store.appendChronicle('A', 1700000000000);
    store.appendChronicle('B', 1700000010000);
    const r = store.recall('chronicle');
    expect(r.chronicle).toContain('A');
    expect(r.chronicle).toContain('B');
  });

  test('writeProject + recall', () => {
    store.writeProject('agora', '# agora\n\n메모 내용');
    const r = store.recall('project', { project: 'agora' });
    expect(r.project).toContain('메모 내용');
  });

  test('writePattern + recall patterns', () => {
    store.writePattern('2026-05-26-schema', '## 패턴\n신뢰도 0.8');
    const r = store.recall('patterns');
    expect(r.patterns?.length).toBe(1);
    expect(r.patterns?.[0]?.name).toContain('schema');
    expect(r.patterns?.[0]?.content).toContain('신뢰도');
  });

  test('recall(all) returns everything available', () => {
    store.appendChronicle('chron');
    store.writeProject('agora', 'proj content');
    store.writePattern('p1', 'pat content');
    const r = store.recall('all', { project: 'agora' });
    expect(r.chronicle).toContain('chron');
    expect(r.project).toContain('proj');
    expect(r.patterns?.[0]?.content).toContain('pat');
  });

  test('recall(chronicle) honors limit (returns tail)', () => {
    const big = 'X'.repeat(5000);
    store.appendChronicle(big);
    const r = store.recall('chronicle', { limit: 100 });
    expect(r.chronicle?.length).toBeLessThanOrEqual(100);
  });

  test('record(project) without name throws', () => {
    expect(() =>
      store.record({ type: 'project', content: 'x' }),
    ).toThrow();
  });

  test('sanitize prevents path traversal', () => {
    store.writeProject('../../../etc/passwd', 'evil');
    // sanitize converted slashes/dots to underscores; file lives in tempDir
    const r = store.recall('project', { project: '../../../etc/passwd' });
    expect(r.project).toBe('evil');
  });
});
