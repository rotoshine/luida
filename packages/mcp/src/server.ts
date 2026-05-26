// 최소 JSON-RPC stdio MCP server.
// MCP 핵심 메서드 4종(initialize, tools/list, tools/call, ping)을 직접 구현.
//
// 호출 패턴: stdin/stdout NDJSON. 라인당 1개 JSON-RPC 메시지.

import {
  type Repos,
  createRepos,
  openDb,
} from '@luida/core';
import { MemoryStore } from '@luida/brain';
import { ALL_TOOLS, type ToolContext, type ToolDef } from './tools';

export type McpServerOpts = {
  me: string;
  dbPath?: string;
  memoryDir?: string;
  reposOverride?: Repos;
};

export type McpRequest = {
  jsonrpc: '2.0';
  id?: number | string | null;
  method: string;
  params?: Record<string, unknown>;
};

export type McpResponse = {
  jsonrpc: '2.0';
  id?: number | string | null;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
};

const TOOL_MAP = new Map<string, ToolDef<unknown, unknown>>(
  ALL_TOOLS.map((t) => [t.name, t as ToolDef<unknown, unknown>]),
);

/**
 * 단일 메시지를 처리. stdio 라인 핸들러에서 호출.
 * 테스트는 이 함수를 직접 부르면 됨.
 */
export async function handleMessage(
  msg: McpRequest,
  ctx: ToolContext,
): Promise<McpResponse | null> {
  if (msg.method === 'initialize') {
    return {
      jsonrpc: '2.0',
      id: msg.id ?? null,
      result: {
        protocolVersion: '2024-11-05',
        capabilities: { tools: {} },
        serverInfo: { name: 'luida-mcp', version: '0.0.0' },
      },
    };
  }

  if (msg.method === 'ping') {
    return { jsonrpc: '2.0', id: msg.id ?? null, result: {} };
  }

  if (msg.method === 'tools/list') {
    return {
      jsonrpc: '2.0',
      id: msg.id ?? null,
      result: {
        tools: ALL_TOOLS.map((t) => ({
          name: t.name,
          description: t.description,
          inputSchema: t.inputSchema,
        })),
      },
    };
  }

  if (msg.method === 'tools/call') {
    const params = msg.params as
      | { name?: string; arguments?: Record<string, unknown> }
      | undefined;
    const name = params?.name;
    const tool = name ? TOOL_MAP.get(name) : undefined;
    if (!tool) {
      return {
        jsonrpc: '2.0',
        id: msg.id ?? null,
        error: { code: -32601, message: `unknown tool: ${name ?? ''}` },
      };
    }
    try {
      const result = await tool.handler(params?.arguments ?? {}, ctx);
      let textPayload: string;
      try {
        textPayload = JSON.stringify(result, null, 2);
      } catch (err) {
        textPayload = `[serialization error: ${(err as Error).message}]`;
      }
      return {
        jsonrpc: '2.0',
        id: msg.id ?? null,
        result: {
          content: [{ type: 'text', text: textPayload }],
        },
      };
    } catch (err) {
      return {
        jsonrpc: '2.0',
        id: msg.id ?? null,
        error: {
          code: -32000,
          message: (err as Error)?.message ?? String(err),
        },
      };
    }
  }

  // notifications/* 등은 응답하지 않음
  if (msg.id == null) return null;

  return {
    jsonrpc: '2.0',
    id: msg.id ?? null,
    error: { code: -32601, message: `unknown method: ${msg.method}` },
  };
}

export type McpServerHandle = {
  close(): void;
};

/**
 * stdio MCP server를 실행한다.
 *
 * 견고성 (Phase 4 review 대응):
 *  - stdin chunk 직렬화 (C1) — data 이벤트들 간 race 방지
 *  - SIGINT/SIGTERM 핸들러는 라이브러리 내부에서 등록하지 않음 (C2 / m1)
 *    호출자(CLI)가 handle.close()를 책임.
 */
export async function runMcpServer(opts: McpServerOpts): Promise<McpServerHandle> {
  const db = opts.reposOverride ? null : openDb(opts.dbPath);
  const repos = opts.reposOverride ?? createRepos(db!);
  const memory = new MemoryStore(opts.memoryDir);
  const ctx: ToolContext = { repos, memory, me: opts.me };

  let buffer = '';
  // 처리 직렬화 큐 — 동시 data 이벤트의 buffer race 차단
  let processing: Promise<void> = Promise.resolve();

  const onData = (chunk: string | Buffer): void => {
    buffer += typeof chunk === 'string' ? chunk : chunk.toString('utf8');
    const lines: string[] = [];
    let nl: number;
    while ((nl = buffer.indexOf('\n')) >= 0) {
      lines.push(buffer.slice(0, nl).trim());
      buffer = buffer.slice(nl + 1);
    }
    if (lines.length === 0) return;
    processing = processing.then(async () => {
      for (const line of lines) {
        if (!line) continue;
        let req: McpRequest;
        try {
          req = JSON.parse(line) as McpRequest;
        } catch {
          continue;
        }
        const res = await handleMessage(req, ctx);
        if (res) process.stdout.write(JSON.stringify(res) + '\n');
      }
    });
  };

  process.stdin.setEncoding('utf8');
  process.stdin.on('data', onData);

  let closed = false;
  const close = (): void => {
    if (closed) return;
    closed = true;
    process.stdin.off('data', onData);
    if (db && !opts.reposOverride) {
      try {
        repos.close();
      } catch {
        // ignore
      }
      try {
        db.close();
      } catch {
        // ignore
      }
    }
  };

  return { close };
}
