// Luida MCP tool 정의 — 순수 함수 + 입력 schema 형태.
// MCP 프로토콜 wiring은 server.ts에서. tools.ts는 단위 테스트 가능한 로직만.

import {
  type Adventurer,
  type Quest,
  type Repos,
} from '@luida/core';
import { MemoryStore, type RecallScope } from '@luida/brain';

export type ToolContext = {
  repos: Repos;
  memory: MemoryStore;
  /** main pane Claude의 이름 (dispatcher) */
  me: string;
};

export type ToolDef<I, O> = {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  handler: (input: I, ctx: ToolContext) => Promise<O> | O;
};

// =======================================================================
// quest.list
// =======================================================================

export type QuestListInput = {
  status?: 'active' | 'all';
  to?: string;
  limit?: number;
};

export type QuestSummary = {
  id: number;
  dispatched_by: string;
  dispatched_to: string;
  brief: string;
  status: string;
  branch: string | null;
  pr_url: string | null;
  progress: string | null;
  updated_at: number;
};

export const questList: ToolDef<QuestListInput, { quests: QuestSummary[] }> = {
  name: 'quest.list',
  description: '활성 quest 목록 또는 전체 quest를 조회합니다.',
  inputSchema: {
    type: 'object',
    properties: {
      status: { type: 'string', enum: ['active', 'all'] },
      to: { type: 'string', description: '특정 모험가 이름으로 필터' },
      limit: { type: 'number', default: 50 },
    },
  },
  handler: (input, ctx) => {
    const limit = input.limit ?? 50;
    let quests: Quest[];
    if (input.to) {
      quests = ctx.repos.quests.listFor(input.to);
      if (input.status === 'active') {
        quests = quests.filter(
          (q) =>
            q.status !== 'completed' &&
            q.status !== 'failed' &&
            q.status !== 'aborted',
        );
      }
    } else if (input.status === 'all') {
      quests = ctx.repos.quests.listActive(); // active만 fast path; all은 future
    } else {
      quests = ctx.repos.quests.listActive();
    }
    return {
      quests: quests.slice(0, limit).map(toSummary),
    };
  },
};

function toSummary(q: Quest): QuestSummary {
  return {
    id: q.id,
    dispatched_by: q.dispatched_by,
    dispatched_to: q.dispatched_to,
    brief: q.brief,
    status: q.status,
    branch: q.branch,
    pr_url: q.pr_url,
    progress: q.progress,
    updated_at: q.updated_at,
  };
}

// =======================================================================
// quest.get
// =======================================================================

export type QuestGetInput = { id: number };

export const questGet: ToolDef<QuestGetInput, { quest: Quest | null }> = {
  name: 'quest.get',
  description: '특정 quest의 상세 정보를 조회합니다.',
  inputSchema: {
    type: 'object',
    properties: { id: { type: 'number' } },
    required: ['id'],
  },
  handler: (input, ctx) => {
    const id = Number(input?.id);
    if (!Number.isFinite(id)) throw new Error('id는 number여야 합니다');
    return { quest: ctx.repos.quests.get(id) };
  },
};

// =======================================================================
// quest.dispatch
// =======================================================================

export type QuestDispatchInput = {
  to: string;
  brief: string;
  branch?: string;
  base?: string;
  pr_title?: string;
};

export const questDispatch: ToolDef<
  QuestDispatchInput,
  { inmail_id: number | null; inserted: boolean; to: string }
> = {
  name: 'quest.dispatch',
  description: '특정 모험가에게 새 의뢰(quest)를 발급합니다.',
  inputSchema: {
    type: 'object',
    properties: {
      to: { type: 'string' },
      brief: { type: 'string' },
      branch: { type: 'string' },
      base: { type: 'string' },
      pr_title: { type: 'string' },
    },
    required: ['to', 'brief'],
  },
  handler: (input, ctx) => {
    // 런타임 input 검증 (MCP 클라이언트는 임의 타입을 보낼 수 있음)
    if (typeof input?.to !== 'string' || !input.to.trim()) {
      throw new Error('to(모험가 이름)는 비어있지 않은 string이어야 합니다');
    }
    if (typeof input?.brief !== 'string' || !input.brief.trim()) {
      throw new Error('brief는 비어있지 않은 string이어야 합니다');
    }
    const r = ctx.repos.inmail.enqueue({
      from_session: ctx.me,
      to_session: input.to,
      kind: 'dispatch',
      payload: {
        brief: input.brief,
        branch: typeof input.branch === 'string' ? input.branch : undefined,
        base: typeof input.base === 'string' ? input.base : undefined,
        pr_title:
          typeof input.pr_title === 'string' ? input.pr_title : undefined,
      },
    });
    return { inmail_id: r.id, inserted: r.inserted, to: input.to };
  },
};

// =======================================================================
// adventurer.list
// =======================================================================

export type AdventurerListInput = Record<string, never>;

export type AdventurerSummary = {
  name: string;
  role: string;
  status: string;
  repo_path: string | null;
  last_seen: number;
};

export const adventurerList: ToolDef<
  AdventurerListInput,
  { adventurers: AdventurerSummary[] }
> = {
  name: 'adventurer.list',
  description: '등록된 모험가(=cmux pane sidecar) 전체를 조회합니다.',
  inputSchema: { type: 'object', properties: {} },
  handler: (_input, ctx) => ({
    adventurers: ctx.repos.adventurers
      .list()
      .map((a: Adventurer) => ({
        name: a.name,
        role: a.role,
        status: a.status,
        repo_path: a.repo_path,
        last_seen: a.last_seen,
      })),
  }),
};

// =======================================================================
// memory.recall
// =======================================================================

export type MemoryRecallInput = {
  scope?: RecallScope;
  project?: string;
  limit?: number;
};

export const memoryRecall: ToolDef<
  MemoryRecallInput,
  { result: ReturnType<MemoryStore['recall']> }
> = {
  name: 'memory.recall',
  description: 'chronicle/project/patterns 메모를 조회합니다.',
  inputSchema: {
    type: 'object',
    properties: {
      scope: {
        type: 'string',
        enum: ['chronicle', 'project', 'patterns', 'all'],
        default: 'all',
      },
      project: { type: 'string' },
      limit: { type: 'number' },
    },
  },
  handler: (input, ctx) => ({
    result: ctx.memory.recall(input.scope ?? 'all', {
      project: input.project,
      limit: input.limit,
    }),
  }),
};

// =======================================================================
// memory.record
// =======================================================================

export type MemoryRecordInput = {
  type: 'chronicle' | 'project' | 'pattern';
  name?: string;
  content: string;
};

export const memoryRecord: ToolDef<MemoryRecordInput, { ok: true }> = {
  name: 'memory.record',
  description: 'chronicle/project/pattern 메모를 기록합니다.',
  inputSchema: {
    type: 'object',
    properties: {
      type: { type: 'string', enum: ['chronicle', 'project', 'pattern'] },
      name: { type: 'string' },
      content: { type: 'string' },
    },
    required: ['type', 'content'],
  },
  handler: (input, ctx) => {
    if (
      input?.type !== 'chronicle' &&
      input?.type !== 'project' &&
      input?.type !== 'pattern'
    ) {
      throw new Error('type은 chronicle/project/pattern 중 하나여야 합니다');
    }
    if (typeof input.content !== 'string') {
      throw new Error('content는 string이어야 합니다');
    }
    const name =
      typeof input.name === 'string' ? input.name : undefined;
    ctx.memory.record({ type: input.type, name, content: input.content });
    return { ok: true };
  },
};

// =======================================================================
// 등록된 모든 툴
// =======================================================================

export const ALL_TOOLS = [
  questList,
  questGet,
  questDispatch,
  adventurerList,
  memoryRecall,
  memoryRecord,
] as const;

export type ToolName = (typeof ALL_TOOLS)[number]['name'];
