import {
  type Integrations,
  createRepos,
  openDb,
} from '@luida/core';
import { handleDispatch } from './dispatch';
import { type PollLoopHandle, pollOnce, startPollLoop } from './poll';
import { changedFiles } from './git';

export type RunSidecarOpts = {
  me: string;
  repoPath: string;
  workspaceId: string;
  surfaceId: string;
  intervalMs?: number;
  /** 1회만 polling하고 종료 (테스트·디버그용) */
  once?: boolean;
  /** PR 자동 생성 여부 */
  autoCreatePr?: boolean;
  /** integration facade. 기본은 sidecar의 real 구현 */
  integrations: Integrations;
  /** DB path override */
  dbPath?: string;
};

export type RunSidecarResult = {
  loop: PollLoopHandle | null;
  processedOnce: number;
  /** loop 정지 + repos finalize + db close까지 한 번에 정리. idempotent. */
  close(): void;
};

/**
 * sidecar 데몬을 띄운다.
 *  1. adventurer를 등록
 *  2. once=true면 pollOnce 1번 후 close+반환
 *  3. else startPollLoop으로 영속 polling, close()로 정리 가능
 */
export async function runSidecar(
  opts: RunSidecarOpts,
): Promise<RunSidecarResult> {
  const db = openDb(opts.dbPath);
  const repos = createRepos(db);
  let closed = false;

  repos.adventurers.upsert({
    name: opts.me,
    workspace_id: opts.workspaceId,
    surface_id: opts.surfaceId,
    repo_path: opts.repoPath,
    role: 'worker',
    status: 'idle',
    pid: process.pid,
  });

  const target = {
    workspace_id: opts.workspaceId,
    surface_id: opts.surfaceId,
  };

  const onMessage = async (
    msg: import('@luida/core').Inmail,
  ): Promise<void> => {
    if (msg.kind === 'dispatch') {
      await handleDispatch(msg, {
        me: opts.me,
        repoPath: opts.repoPath,
        quests: repos.quests,
        inmail: repos.inmail,
        events: repos.events,
        integrations: opts.integrations,
        autoCreatePr: opts.autoCreatePr ?? false,
        // Phase 3 — brain 평가 활성화
        repos,
        getChangedFiles: async (cwd) => {
          try {
            return await changedFiles({ cwd });
          } catch (err) {
            console.warn(
              `[sidecar:${opts.me}] changedFiles 실패 (base ref 부재?):`,
              err,
            );
            return [];
          }
        },
      });
    }
  };

  const cleanup = (): void => {
    if (closed) return;
    closed = true;
    try {
      repos.close();
    } catch (err) {
      console.error('[sidecar] repos.close error:', err);
    }
    try {
      db.close();
    } catch (err) {
      console.error('[sidecar] db.close error:', err);
    }
  };

  if (opts.once) {
    const n = await pollOnce({
      me: opts.me,
      target,
      inmail: repos.inmail,
      cmux: opts.integrations.cmux,
      onMessage,
    });
    cleanup();
    return {
      loop: null,
      processedOnce: n,
      close: cleanup,
    };
  }

  const loop = startPollLoop(
    {
      me: opts.me,
      target,
      inmail: repos.inmail,
      cmux: opts.integrations.cmux,
      onMessage,
    },
    opts.intervalMs ?? 10_000,
  );

  return {
    loop,
    processedOnce: 0,
    close() {
      loop.stop();
      cleanup();
    },
  };
}
