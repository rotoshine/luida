// 외부 시스템(cmux, worktrunk, claude CLI, gh CLI) 통합 인터페이스.
// 실제 구현은 sidecar 패키지에서 Bun.spawn 기반으로 제공.
// 테스트는 mock 구현(./mocks)으로 가짜화 가능.

export type SurfaceId = string;
export type WorkspaceId = string;

export type CmuxTarget = {
  workspace_id: WorkspaceId;
  surface_id: SurfaceId;
};

export interface CmuxBridge {
  /** 텍스트를 surface에 입력하고 Enter 키를 보낸다 */
  sendPrompt(target: CmuxTarget, text: string): Promise<void>;
  /** 현재 화면 텍스트를 읽는다 (busy/idle 판정용) */
  readScreen(target: CmuxTarget): Promise<string>;
}

export type WorktreeHandle = {
  branch: string;
  path: string;
};

export interface Worktree {
  /** `wt c "<name>"` — 새 worktree 생성 + 경로 반환 */
  create(repoPath: string, branchName: string): Promise<WorktreeHandle>;
}

export type WorkerSpawnOptions = {
  cwd: string;
  brief: string;
  sessionId?: string; // claude --session-id
  env?: Record<string, string>;
};

/**
 * stream-json 모드 worker의 이벤트.
 * Claude Code의 `claude -p --output-format stream-json` 스펙을 단순화.
 */
export type WorkerStreamEvent =
  | { kind: 'system'; subtype: string; payload: unknown }
  | { kind: 'tool_use'; name: string; input: unknown }
  | { kind: 'text'; text: string }
  | { kind: 'result'; success: boolean; summary?: string }
  | { kind: 'error'; message: string };

export interface WorkerRunner {
  /** headless claude worker를 띄우고 stream-json 이벤트를 yield */
  spawn(opts: WorkerSpawnOptions): AsyncIterable<WorkerStreamEvent>;
}

export type PullRequestOptions = {
  cwd: string;
  title: string;
  body: string;
  base?: string;
  /** 명시적으로 PR이 머지 대상으로 삼을 head branch */
  head?: string;
};

export interface VcsHost {
  /** `gh pr create` — PR URL 반환 */
  createPullRequest(opts: PullRequestOptions): Promise<{ url: string }>;
}

/** 4개 통합을 묶은 facade. sidecar 시작 시 주입. */
export type Integrations = {
  cmux: CmuxBridge;
  worktree: Worktree;
  worker: WorkerRunner;
  vcs: VcsHost;
};
