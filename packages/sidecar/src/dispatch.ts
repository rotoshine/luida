import type {
  EventRepo,
  Inmail,
  Integrations,
  QuestRepo,
  InmailRepo,
  Repos,
  WorkerStreamEvent,
} from '@luida/core';
import { applyFollowUps, evaluatePostQuest } from '@luida/brain';

export type DispatchHandlerOpts = {
  me: string;
  repoPath: string;
  quests: QuestRepo;
  inmail: InmailRepo;
  events: EventRepo;
  integrations: Integrations;
  /** PR 자동 생성 여부. 기본 false(needs_approval 멈춤) */
  autoCreatePr?: boolean;
  /**
   * Phase 3: PR/needs_approval 후 relationships 평가 실행.
   * 전체 Repos 핸들이 있어야 brain이 평가/적용 가능.
   */
  repos?: Repos;
  /**
   * Phase 3: 변경 파일 목록 계산 hook.
   * 기본은 sidecar의 git 헬퍼 사용 — 테스트에선 mock으로 대체.
   */
  getChangedFiles?: (cwd: string) => Promise<string[]>;
};

export type DispatchPayload = {
  brief: string;
  branch?: string;
  base?: string;
  pr_title?: string;
};

export type DispatchResult = {
  questId: number;
  prUrl: string | null;
  success: boolean;
  error?: string;
  /** Phase 3: 평가 결과 요약 */
  chained?: { dispatched: number; proposed: number };
};

/**
 * dispatch kind 메시지를 받아 quest를 만들고 worker를 돌리고 PR까지 완료한다.
 *
 * 견고성:
 *  - 전체 try-catch로 단계 실패도 quest=failed + ack로 닫음
 *  - 빈 brief는 즉시 ack 실패
 *  - worker stream이 result 이벤트 없이 끝나면 failed로 분류
 *  - Phase 3: PR 생성/needs_approval 후 brain.evaluatePostQuest로 후속 룰 발사
 *  - source_inmail_id 멱등성으로 같은 inmail 두 번 처리 시 중복 quest 차단
 */
export async function handleDispatch(
  msg: Inmail,
  opts: DispatchHandlerOpts,
): Promise<DispatchResult> {
  const payload = parsePayload(msg.payload);

  if (!payload.brief?.trim()) {
    opts.inmail.enqueue({
      from_session: opts.me,
      to_session: msg.from_session,
      reply_to: msg.id,
      kind: 'ack',
      payload: { success: false, summary: 'dispatch payload missing brief' },
      dedupe_key: `ack:inmail-${msg.id}`,
    });
    return {
      questId: -1,
      prUrl: null,
      success: false,
      error: 'missing brief',
    };
  }

  const branch = payload.branch ?? defaultBranch(msg.from_session, msg.id);

  let questId: number | null = null;
  try {
    // 1) quest row 생성 (멱등)
    const inserted = opts.quests.insertIdempotent({
      dispatched_by: msg.from_session,
      dispatched_to: opts.me,
      brief: payload.brief,
      branch,
      status: 'running',
      source_inmail_id: msg.id,
    });
    questId = inserted.id;

    if (!inserted.created) {
      // 같은 inmail이 이미 처리됨 — 현재 상태만 알리고 종료
      const existing = opts.quests.get(questId);
      return {
        questId,
        prUrl: existing?.pr_url ?? null,
        success: existing?.status === 'completed',
      };
    }

    opts.events.record({
      quest_id: questId,
      actor: opts.me,
      kind: 'quest_dispatched',
      payload: { from: msg.from_session, branch, inmail_id: msg.id },
    });

    // 2) wt c "<branch>" — worktree 생성
    const wt = await opts.integrations.worktree.create(opts.repoPath, branch);
    opts.quests.updateWorktree(questId, wt.branch, wt.path);

    // 3) headless worker 실행
    const events: WorkerStreamEvent[] = [];
    let success = false;
    let sawResult = false;
    let lastSummary: string | undefined;
    let lastError: string | undefined;

    for await (const ev of opts.integrations.worker.spawn({
      cwd: wt.path,
      brief: payload.brief,
      sessionId: `quest-${questId}`,
    })) {
      events.push(ev);
      if (ev.kind === 'text') {
        opts.quests.updateProgress(questId, truncate(ev.text, 200));
      } else if (ev.kind === 'tool_use') {
        opts.events.record({
          quest_id: questId,
          actor: opts.me,
          kind: 'tool_used',
          payload: { name: ev.name },
        });
      } else if (ev.kind === 'result') {
        sawResult = true;
        success = ev.success;
        lastSummary = ev.summary;
      } else if (ev.kind === 'error') {
        lastError = ev.message;
        opts.events.record({
          quest_id: questId,
          actor: opts.me,
          kind: 'review_failed',
          payload: { message: ev.message },
        });
      }
    }

    if (!sawResult) {
      success = false;
      lastSummary = lastError ?? 'worker가 result 이벤트 없이 종료';
    }

    // 4) 실패면 종료
    if (!success) {
      opts.quests.updateStatus(questId, 'failed');
      opts.inmail.enqueue({
        from_session: opts.me,
        to_session: msg.from_session,
        reply_to: msg.id,
        quest_id: questId,
        kind: 'ack',
        payload: {
          success: false,
          summary: lastSummary ?? '작업 실패',
          error: lastError,
        },
        dedupe_key: `ack:inmail-${msg.id}`,
      });
      return { questId, prUrl: null, success: false, error: lastError };
    }

    // 5) review 상태 후 PR 또는 needs_approval
    opts.quests.updateStatus(questId, 'reviewing');
    opts.events.record({
      quest_id: questId,
      actor: opts.me,
      kind: 'review_passed',
      payload: { summary: lastSummary },
    });

    let prUrl: string | null = null;
    if (!opts.autoCreatePr) {
      opts.quests.updateStatus(questId, 'needs_approval');
      opts.inmail.enqueue({
        from_session: opts.me,
        to_session: msg.from_session,
        reply_to: msg.id,
        quest_id: questId,
        kind: 'proposal',
        payload: {
          question: 'PR 생성 승인?',
          summary: lastSummary ?? '',
          worktree: wt.path,
          branch: wt.branch,
        },
        dedupe_key: `proposal:quest-${questId}`,
      });
    } else {
      const pr = await opts.integrations.vcs.createPullRequest({
        cwd: wt.path,
        title:
          payload.pr_title ?? `[${opts.me}] ${truncate(payload.brief, 60)}`,
        body: lastSummary ?? '',
        base: payload.base ?? 'main',
        head: wt.branch,
      });
      prUrl = pr.url;
      opts.quests.markCompleted(questId, pr.url);
      opts.events.record({
        quest_id: questId,
        actor: opts.me,
        kind: 'pr_created',
        payload: { url: pr.url },
      });
      opts.inmail.enqueue({
        from_session: opts.me,
        to_session: msg.from_session,
        reply_to: msg.id,
        quest_id: questId,
        kind: 'ack',
        payload: {
          success: true,
          pr_url: pr.url,
          summary: lastSummary ?? '',
        },
        dedupe_key: `ack:inmail-${msg.id}`,
      });
    }

    // 6) Phase 3: relationships 평가 → 후속 dispatch/proposal
    //
    // 정책 (M4 대응):
    //  - auto_dispatch: PR이 실제로 만들어진 completed 상태에서만 발화
    //    (needs_approval 단계에서는 변경이 main에 안 올라간 상태라 위험)
    //  - proposal: needs_approval에서도 발화 OK (사용자 판단 보조)
    let chained: { dispatched: number; proposed: number } | undefined;
    if (opts.repos) {
      try {
        const quest = opts.quests.get(questId)!;
        const filesList = opts.getChangedFiles
          ? await opts.getChangedFiles(wt.path)
          : [];
        const evalResult = evaluatePostQuest(opts.repos, {
          quest,
          changedFiles: filesList,
        });
        // PR 만들어지지 않은 needs_approval 상태에서는 auto_dispatch를 보류
        const autoDispatch =
          quest.status === 'completed' ? evalResult.autoDispatch : [];
        chained = applyFollowUps(
          opts.repos,
          opts.me,
          { quest, changedFiles: filesList },
          { autoDispatch, proposals: evalResult.proposals },
        );
        if (chained.dispatched > 0 || chained.proposed > 0) {
          opts.events.record({
            quest_id: questId,
            actor: opts.me,
            kind: 'pattern_proposed',
            payload: chained,
          });
        }
      } catch (err) {
        // 평가 실패는 quest 자체를 실패시키지 않음 — 로그만
        console.error(
          `[sidecar:${opts.me}] post-quest evaluation failed:`,
          err,
        );
      }
    }

    return { questId, prUrl, success: true, chained };
  } catch (err) {
    const message = (err as Error)?.message ?? String(err);
    if (questId != null) {
      opts.quests.updateStatus(questId, 'failed');
      opts.events.record({
        quest_id: questId,
        actor: opts.me,
        kind: 'review_failed',
        payload: { message, where: 'handleDispatch' },
      });
    }
    opts.inmail.enqueue({
      from_session: opts.me,
      to_session: msg.from_session,
      reply_to: msg.id,
      quest_id: questId,
      kind: 'ack',
      payload: { success: false, summary: `dispatch 실패: ${message}` },
      dedupe_key: `ack:inmail-${msg.id}`,
    });
    return {
      questId: questId ?? -1,
      prUrl: null,
      success: false,
      error: message,
    };
  }
}

export function parsePayload(raw: string): DispatchPayload {
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return { brief: raw };
  }
  if (typeof v !== 'object' || v === null) return { brief: String(v ?? '') };
  const o = v as Record<string, unknown>;
  return {
    brief: typeof o.brief === 'string' ? o.brief : '',
    branch: typeof o.branch === 'string' ? o.branch : undefined,
    base: typeof o.base === 'string' ? o.base : undefined,
    pr_title: typeof o.pr_title === 'string' ? o.pr_title : undefined,
  };
}

export function defaultBranch(from: string, msgId: number): string {
  const safe = from.replace(/[^a-zA-Z0-9_-]/g, '-').replace(/-+/g, '-');
  return `luida/${safe}-quest-${msgId}`;
}

function truncate(s: string, n: number): string {
  const chars = [...s];
  return chars.length <= n ? s : chars.slice(0, n - 1).join('') + '…';
}
