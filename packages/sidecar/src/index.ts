// Public surface of the sidecar package.
export { handleDispatch } from './dispatch';
export type { DispatchHandlerOpts, DispatchPayload } from './dispatch';

export { pollOnce, startPollLoop } from './poll';
export type { PollLoopHandle, PollOpts } from './poll';

export { renderInmailPrompt } from './render';

export { runSidecar } from './run';
export type { RunSidecarOpts } from './run';

export { CmuxCliBridge } from './integrations/cmux';
export { WorktrunkWorktree } from './integrations/worktree';
export { ClaudeWorkerRunner } from './integrations/worker';
export { GhVcsHost } from './integrations/vcs';
export { createRealIntegrations } from './integrations';

// Phase D: 권한 가드
export {
  DEFAULT_SECRET_PATTERNS,
  checkPath,
  defaultBlockedPaths,
  evaluateHook,
  maskSecrets,
} from './guards';
export type { GuardDecision, GuardInput, GuardOpts } from './guards';
