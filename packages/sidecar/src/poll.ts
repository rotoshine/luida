import type {
  CmuxBridge,
  CmuxTarget,
  Inmail,
  InmailRepo,
} from '@luida/core';
import { renderInmailPrompt } from './render';

export type PollOpts = {
  me: string;
  target: CmuxTarget;
  inmail: InmailRepo;
  cmux: CmuxBridge;
  /** inmail 도착 시 호출. dispatch 등 추가 처리 훅 */
  onMessage?: (msg: Inmail) => Promise<void> | void;
  /** 주입 전에 호출 (테스트에서 prompt 확인) */
  onInject?: (msg: Inmail, prompt: string) => void;
};

/**
 * 보류 중인 inmail을 한 번 polling해서 cmux로 주입한다.
 *
 * 에러 격리 정책 (Phase 1 리뷰 C1 대응):
 *  - 메시지 1건 처리는 try-catch로 격리되어, 한 건 실패가 큐 전체를 stall시키지 않음
 *  - `sendPrompt` 단계 실패 → markDelivered 안 함 → 다음 tick에 재시도됨
 *  - `markDelivered` 이후 `onMessage` 실패 → 메시지는 delivered로 보존, 에러는 로그.
 *    추가 보상은 onMessage 내부(handleDispatch)에서 책임진다 (quest=failed + ack)
 */
export async function pollOnce(opts: PollOpts): Promise<number> {
  const messages = opts.inmail.pendingFor(opts.me);
  let processed = 0;

  for (const msg of messages) {
    try {
      const prompt = renderInmailPrompt(msg);
      opts.onInject?.(msg, prompt);
      await opts.cmux.sendPrompt(opts.target, prompt);
      opts.inmail.markDelivered(msg.id);
      processed += 1;

      if (opts.onMessage) {
        try {
          await opts.onMessage(msg);
        } catch (err) {
          console.error(
            `[sidecar:${opts.me}] onMessage(inmail#${msg.id}) failed:`,
            err,
          );
          // 이미 delivered. 보상 처리는 onMessage 측 책임.
        }
      }
    } catch (err) {
      console.error(
        `[sidecar:${opts.me}] delivery failed for inmail#${msg.id}:`,
        err,
      );
      // sendPrompt 실패 시 markDelivered 안 했으므로 다음 tick에 재시도.
      // 큐 stall 방지를 위해 다음 메시지로 진행.
      continue;
    }
  }

  return processed;
}

export type PollLoopHandle = {
  stop: () => void;
};

/** 10초 간격으로 pollOnce를 계속 호출하는 long-running 루프 */
export function startPollLoop(
  opts: PollOpts,
  intervalMs = 10_000,
): PollLoopHandle {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const tick = async (): Promise<void> => {
    if (stopped) return;
    try {
      await pollOnce(opts);
    } catch (err) {
      console.error(`[sidecar:${opts.me}] poll error:`, err);
    } finally {
      if (!stopped) {
        timer = setTimeout(tick, intervalMs);
      }
    }
  };

  timer = setTimeout(tick, intervalMs);
  return {
    stop() {
      stopped = true;
      if (timer) clearTimeout(timer);
      timer = null;
    },
  };
}
