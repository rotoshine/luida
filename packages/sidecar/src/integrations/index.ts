import type { Integrations } from '@luida/core';
import { CmuxCliBridge } from './cmux';
import { GhVcsHost } from './vcs';
import { ClaudeWorkerRunner } from './worker';
import { WorktrunkWorktree } from './worktree';

export { CmuxCliBridge } from './cmux';
export { WorktrunkWorktree } from './worktree';
export { ClaudeWorkerRunner } from './worker';
export { GhVcsHost } from './vcs';

/** 실제 외부 CLI에 연결된 Integrations facade를 만든다. */
export function createRealIntegrations(): Integrations {
  return {
    cmux: new CmuxCliBridge(),
    worktree: new WorktrunkWorktree(),
    worker: new ClaudeWorkerRunner(),
    vcs: new GhVcsHost(),
  };
}
