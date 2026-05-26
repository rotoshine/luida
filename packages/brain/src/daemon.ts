// headless Luida brain 데몬.
// 책임:
//  1. 자기 자신을 adventurer로 등록 (role='brain')
//  2. 주기적으로 stuck quest 감지 → events 기록 (단, idempotent)
//  3. (Phase 5) reflect job — 최근 events 분석해 패턴 후보 생성

import {
  createRepos,
  openDb,
  nowMs,
  type Repos,
} from '@luida/core';
import { MemoryStore } from './memory';
import { reflect, type ReflectResult } from './reflect';

export type BrainOpts = {
  intervalMs?: number;
  /** 1회 tick 후 종료 (테스트·디버그용) */
  once?: boolean;
  dbPath?: string;
  /** stuck quest 임계치 (ms). 기본 1시간 */
  stuckThresholdMs?: number;
  /** Phase 5: reflect job 실행 주기 (ms). 기본 6시간. 0이면 매 tick마다 */
  reflectIntervalMs?: number;
  /** 테스트 주입용 — 외부 repos 사용 시 brain은 DB 열지 않음 */
  reposOverride?: Repos;
  /** 테스트 주입용 — 메모리 디렉터리 override */
  memoryDirOverride?: string;
  /** 현재 시각 주입 (테스트) */
  now?: () => number;
};

export type BrainHandle = {
  stop(): void;
  /** 1회 tick을 직접 실행 (테스트용) */
  tick(): Promise<TickResult>;
};

export type TickResult = {
  heartbeatAt: number;
  stuckQuests: number[];
  /** 새로 기록한 stuck event 수 (이미 기록된 stuck quest는 카운트 안 함) */
  recorded: number;
  /** Phase 5: 이번 tick에 reflect가 실행되었으면 결과 포함 */
  reflect?: ReflectResult;
};

const DEFAULT_INTERVAL_MS = 60_000;
const DEFAULT_STUCK_MS = 60 * 60 * 1000;
const DEFAULT_REFLECT_INTERVAL_MS = 6 * 60 * 60 * 1000; // 6시간
/** 같은 quest에 대해 stuck event를 재기록하지 않는 cooldown (ms). 기본 stuckThresholdMs와 동일 */

export async function runBrain(opts: BrainOpts): Promise<BrainHandle> {
  const intervalMs = opts.intervalMs ?? DEFAULT_INTERVAL_MS;
  const stuckMs = opts.stuckThresholdMs ?? DEFAULT_STUCK_MS;
  const reflectIntervalMs =
    opts.reflectIntervalMs ?? DEFAULT_REFLECT_INTERVAL_MS;
  const now = opts.now ?? nowMs;

  const db = opts.reposOverride ? null : openDb(opts.dbPath);
  const repos = opts.reposOverride ?? createRepos(db!);
  const memory = new MemoryStore(opts.memoryDirOverride);

  // M2: 데몬 재시작 시 lastReflectAt 복원 — 가장 최근 reflect 종료 이벤트 사용
  let lastReflectAt = 0;
  const recent = repos.events.byKind('brain_reflect_done', 1);
  if (recent.length > 0 && recent[0]) {
    lastReflectAt = recent[0].occurred_at;
  }

  repos.adventurers.upsert({
    name: 'luida-brain',
    workspace_id: 'brain',
    surface_id: 'brain',
    role: 'brain',
    status: 'idle',
  });

  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let inFlight: Promise<TickResult> | null = null;

  async function tick(): Promise<TickResult> {
    const heartbeatAt = now();
    const active = repos.quests.listActive();
    const stuck: number[] = [];
    let recorded = 0;

    // stuck 후보 → 이미 cooldown 내에 stuck event를 기록했는지 확인 (idempotency)
    const cooldown = stuckMs;
    const sinceMs = heartbeatAt - cooldown;
    const recent = repos.events.recentSince(sinceMs, 1000);
    const alreadyRecorded = new Set(
      recent
        .filter((e) => e.kind === 'review_failed' && e.actor === 'luida-brain')
        .map((e) => e.quest_id)
        .filter((id): id is number => id != null),
    );

    for (const q of active) {
      if (q.status === 'running' && heartbeatAt - q.updated_at > stuckMs) {
        stuck.push(q.id);
        if (!alreadyRecorded.has(q.id)) {
          repos.events.record({
            quest_id: q.id,
            actor: 'luida-brain',
            kind: 'review_failed',
            payload: {
              reason: 'stuck',
              elapsed_ms: heartbeatAt - q.updated_at,
            },
          });
          recorded += 1;
        }
      }
    }
    repos.adventurers.upsert({
      name: 'luida-brain',
      workspace_id: 'brain',
      surface_id: 'brain',
      role: 'brain',
      status: 'idle',
    });

    if (recorded > 0) {
      memory.appendChronicle(
        `🧠 brain heartbeat: stuck quests ${stuck.join(', ')}`,
        heartbeatAt,
      );
    }

    // Phase 5: reflect job — 마지막 reflect로부터 reflectIntervalMs 경과 시 실행
    let reflectResult: ReflectResult | undefined;
    if (heartbeatAt - lastReflectAt >= reflectIntervalMs) {
      try {
        reflectResult = await reflect(repos, memory, { now: () => heartbeatAt });
        lastReflectAt = heartbeatAt;
        // 재시작 시 lastReflectAt 복원 가능하도록 이벤트 기록 (M2)
        repos.events.record({
          actor: 'luida-brain',
          kind: 'brain_reflect_done',
          payload: {
            candidates: reflectResult.candidates.length,
            written: reflectResult.written.length,
            proposed: reflectResult.proposed,
          },
        });
      } catch (err) {
        console.error('[brain] reflect 실패:', err);
      }
    }

    return {
      heartbeatAt,
      stuckQuests: stuck,
      recorded,
      reflect: reflectResult,
    };
  }

  const cleanup = (): void => {
    if (timer) clearTimeout(timer);
    timer = null;
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

  if (opts.once) {
    inFlight = tick();
    await inFlight;
    cleanup();
    return {
      stop: cleanup,
      tick: async () => {
        if (inFlight) await inFlight;
        return tick();
      },
    };
  }

  const schedule = (): void => {
    if (stopped) return;
    timer = setTimeout(() => {
      if (stopped) return;
      inFlight = tick();
      inFlight
        .catch((err) => {
          console.error('[brain] tick 실패:', err);
        })
        .finally(() => {
          inFlight = null;
          schedule();
        });
    }, intervalMs);
  };
  schedule();

  const stop = (): void => {
    stopped = true;
    if (timer) clearTimeout(timer);
    timer = null;
    // 진행 중 tick이 있으면 끝나길 fire-and-forget으로 기다린 뒤 cleanup
    if (inFlight) {
      inFlight
        .catch(() => {
          // ignore
        })
        .finally(() => cleanup());
    } else {
      cleanup();
    }
  };

  return { stop, tick };
}
