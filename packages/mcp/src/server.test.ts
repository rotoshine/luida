import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { MemoryStore } from '@luida/brain';
import { createRepos, migrate, openDb, type Repos } from '@luida/core';
import { handleMessage, type McpRequest } from './server';

let tempDir: string;
let dbPath: string;
let memoryDir: string;
let db: ReturnType<typeof openDb>;
let repos: Repos;
let memory: MemoryStore;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-mcp-srv-'));
  dbPath = join(tempDir, 'tavern.db');
  memoryDir = join(tempDir, 'memory');
  db = openDb(dbPath);
  await migrate(db);
  repos = createRepos(db);
  memory = new MemoryStore(memoryDir);
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
});

afterEach(async () => {
  repos.close();
  db.close();
  await rm(tempDir, { recursive: true, force: true });
});

function rpc(method: string, params?: Record<string, unknown>, id = 1): McpRequest {
  return { jsonrpc: '2.0', id, method, params };
}

describe('MCP handleMessage', () => {
  const ctx = (): { repos: Repos; memory: MemoryStore; me: string } => ({
    repos,
    memory,
    me: 'luida',
  });

  test('initialize 반환', async () => {
    const r = await handleMessage(rpc('initialize'), ctx());
    expect(r?.result).toMatchObject({ protocolVersion: '2024-11-05' });
  });

  test('tools/list는 모든 툴 노출', async () => {
    const r = await handleMessage(rpc('tools/list'), ctx());
    const tools = (r?.result as { tools: { name: string }[] }).tools;
    const names = tools.map((t) => t.name);
    expect(names).toContain('quest.list');
    expect(names).toContain('quest.dispatch');
    expect(names).toContain('adventurer.list');
    expect(names).toContain('memory.recall');
  });

  test('tools/call quest.dispatch', async () => {
    const r = await handleMessage(
      rpc('tools/call', {
        name: 'quest.dispatch',
        arguments: { to: 'agora', brief: '스키마 작업' },
      }),
      ctx(),
    );
    expect(r?.result).toBeDefined();
    const content = (r?.result as { content: { text: string }[] }).content;
    const payload = JSON.parse(content[0]!.text);
    expect(payload.inserted).toBe(true);
    expect(payload.to).toBe('agora');
  });

  test('tools/call unknown tool → error', async () => {
    const r = await handleMessage(
      rpc('tools/call', { name: 'bogus.tool' }),
      ctx(),
    );
    expect(r?.error?.code).toBe(-32601);
  });

  test('tools/call handler throw → error response', async () => {
    const r = await handleMessage(
      rpc('tools/call', {
        name: 'quest.dispatch',
        arguments: { to: 'agora', brief: '   ' },
      }),
      ctx(),
    );
    expect(r?.error).toBeDefined();
    expect(r?.error?.message).toContain('brief');
  });

  test('ping', async () => {
    const r = await handleMessage(rpc('ping'), ctx());
    expect(r?.result).toEqual({});
  });

  test('unknown method 응답', async () => {
    const r = await handleMessage(rpc('foo/bar'), ctx());
    expect(r?.error?.code).toBe(-32601);
  });

  test('notification (id 없음)은 null 반환', async () => {
    const msg = { jsonrpc: '2.0' as const, method: 'something' };
    const r = await handleMessage(msg, ctx());
    expect(r).toBeNull();
  });
});
