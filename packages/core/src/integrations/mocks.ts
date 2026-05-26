import type {
  CmuxBridge,
  CmuxTarget,
  Integrations,
  PullRequestOptions,
  VcsHost,
  Worktree,
  WorktreeHandle,
  WorkerRunner,
  WorkerSpawnOptions,
  WorkerStreamEvent,
} from './types';

/** 테스트용 in-memory CmuxBridge. 보낸 키를 배열에 누적한다. */
export class FakeCmuxBridge implements CmuxBridge {
  readonly sent: Array<{ target: CmuxTarget; text: string }> = [];
  readonly screens = new Map<string, string>();

  async sendPrompt(target: CmuxTarget, text: string): Promise<void> {
    this.sent.push({ target, text });
  }

  async readScreen(target: CmuxTarget): Promise<string> {
    return this.screens.get(`${target.workspace_id}:${target.surface_id}`) ?? '';
  }
}

/** 테스트용 Worktree. 가짜 경로를 만들지 않고 placeholder를 반환. */
export class FakeWorktree implements Worktree {
  readonly created: Array<{ repoPath: string; branch: string }> = [];

  async create(repoPath: string, branchName: string): Promise<WorktreeHandle> {
    this.created.push({ repoPath, branch: branchName });
    return {
      branch: branchName,
      path: `${repoPath}/.worktrees/${branchName}`,
    };
  }
}

/** 미리 정의된 이벤트 시퀀스를 yield하는 worker. */
export class ScriptedWorkerRunner implements WorkerRunner {
  readonly spawns: WorkerSpawnOptions[] = [];

  constructor(private readonly script: WorkerStreamEvent[]) {}

  async *spawn(opts: WorkerSpawnOptions): AsyncIterable<WorkerStreamEvent> {
    this.spawns.push(opts);
    for (const ev of this.script) {
      yield ev;
    }
  }
}

/** 가짜 PR URL을 반환하는 vcs host. */
export class FakeVcsHost implements VcsHost {
  readonly calls: PullRequestOptions[] = [];
  private counter = 0;

  async createPullRequest(opts: PullRequestOptions): Promise<{ url: string }> {
    this.calls.push(opts);
    this.counter += 1;
    return { url: `https://example.test/pr/${this.counter}` };
  }
}

/** 4종 fake를 한 번에 생성. */
export function createFakeIntegrations(
  script: WorkerStreamEvent[] = [
    { kind: 'text', text: 'starting' },
    { kind: 'tool_use', name: 'Edit', input: {} },
    { kind: 'result', success: true, summary: 'done' },
  ],
): Integrations & {
  cmux: FakeCmuxBridge;
  worktree: FakeWorktree;
  worker: ScriptedWorkerRunner;
  vcs: FakeVcsHost;
} {
  return {
    cmux: new FakeCmuxBridge(),
    worktree: new FakeWorktree(),
    worker: new ScriptedWorkerRunner(script),
    vcs: new FakeVcsHost(),
  };
}
