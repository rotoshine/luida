// Phase 5: 학습 루프 — reflect.
//
// 최근 events를 분석해 "사람 정의 룰로 승격할만한 패턴 후보"를 찾는다.
// 현재 휴리스틱(MVP):
//   1. 같은 (from→to) 쌍에서 quest_dispatched가 N회 이상 연속 발생
//      → "(from)이 끝나면 (to)에게도 자동 dispatch" 룰 후보
//   2. 같은 (from→to)에서 pr_created가 함께 발생 → 신뢰도 boost
//
// 출력:
//   - ~/.luida/memory/patterns/YYYY-MM-DD-<topic>.md
//   - inmail kind='proposal' to='luida' (사용자 승인 게이트)

import {
  type LuidaEvent,
  type RelationshipRepo,
  type Repos,
} from '@luida/core';
import { MemoryStore } from './memory';

export type PatternCandidate = {
  id: string; // 파일명에 들어갈 안정 키
  topic: string;
  from: string;
  to: string;
  confidence: number; // 0.0 ~ 1.0
  evidence: number; // 신뢰도 산출 근거가 된 이벤트 수
  proposedBriefTemplate: string;
};

export type ReflectOpts = {
  /** 분석 윈도우 (ms). 기본 7일 */
  windowMs?: number;
  /** 최소 신뢰도. 미달 후보는 출력 안 함 */
  minConfidence?: number;
  /** 발화 최소 횟수. 1이면 1회만 봐도 후보. 기본 3 */
  minSamples?: number;
  /** 현재 시각 주입 */
  now?: () => number;
};

export type ReflectResult = {
  candidates: PatternCandidate[];
  written: string[]; // 새로 작성된 pattern 파일명
  proposed: number; // 새로 발행된 proposal inmail 개수
};

const DEFAULT_WINDOW = 7 * 24 * 60 * 60 * 1000;

export type AnalyzeOpts = {
  minSamples?: number;
  /** 알려진 adventurer 이름 집합. 지정 시 모르는 from/to는 노이즈로 차단 (M5). */
  knownAdventurers?: ReadonlySet<string>;
};

export function analyzeEvents(
  events: LuidaEvent[],
  optsOrMin: number | AnalyzeOpts = 3,
): PatternCandidate[] {
  const minSamples =
    typeof optsOrMin === 'number' ? optsOrMin : optsOrMin.minSamples ?? 3;
  const known =
    typeof optsOrMin === 'number' ? undefined : optsOrMin.knownAdventurers;
  // (from→to) 쌍별로 quest_dispatched 카운트, pr_created로 신뢰도 부스트
  const dispatched = new Map<string, { from: string; to: string; count: number }>();
  const completed = new Map<string, number>();

  for (const e of events) {
    if (e.kind === 'quest_dispatched') {
      const payload = parsePayload(e.payload);
      // 타입 가드 (M5) — payload.from은 unknown
      const fromRaw = (payload as { from?: unknown }).from;
      if (typeof fromRaw !== 'string' || !fromRaw) continue;
      const from = fromRaw;
      const to = e.actor;
      if (!to) continue;
      if (known && (!known.has(from) || !known.has(to))) continue;
      const key = `${from}->${to}`;
      const prev = dispatched.get(key);
      if (prev) prev.count += 1;
      else dispatched.set(key, { from, to, count: 1 });
    } else if (e.kind === 'pr_created') {
      // pr_created는 quest_id가 같은 dispatched 쌍의 신뢰도를 올려줌
      // 단순화: actor 단독으로 카운트 (정확한 매칭은 Phase 5+에서 quest_id join)
      completed.set(e.actor, (completed.get(e.actor) ?? 0) + 1);
    }
  }

  const candidates: PatternCandidate[] = [];
  for (const [, info] of dispatched) {
    if (info.count < minSamples) continue;
    const prBoost = (completed.get(info.to) ?? 0) > 0 ? 0.2 : 0;
    const base = Math.min(0.85, info.count / 10);
    const confidence = Math.min(1, base + prBoost);
    candidates.push({
      id: `${slug(info.from)}-to-${slug(info.to)}`,
      topic: `${info.from} → ${info.to} 연쇄`,
      from: info.from,
      to: info.to,
      confidence,
      evidence: info.count,
      proposedBriefTemplate: `${info.from} 작업 완료에 따라 ${info.to}에서 후속 작업을 수행해주세요`,
    });
  }

  return candidates.sort((a, b) => b.confidence - a.confidence);
}

export function renderPatternMarkdown(c: PatternCandidate, now: number): string {
  const date = new Date(now).toISOString().slice(0, 10);
  return [
    `# 패턴 후보: ${c.topic}`,
    '',
    `- 발견: ${date}`,
    `- 신뢰도: ${(c.confidence * 10).toFixed(1)} / 10`,
    `- 근거 이벤트: ${c.evidence}건`,
    '',
    '## 제안 룰',
    '',
    '```yaml',
    `- name: ${c.id}`,
    `  from: ${c.from}`,
    `  to: ${c.to}`,
    `  action: auto_dispatch`,
    `  trigger:`,
    `    kind: quest_completed`,
    `    status: completed`,
    `  brief_template: "${c.proposedBriefTemplate}"`,
    `  enabled: false  # 승급 직후 기본 disabled — 'luida promote-pattern <id> --activate'로 켜기`,
    '```',
    '',
    '## 승급',
    '',
    '`luida promote-pattern ' + c.id + '`',
    '',
  ].join('\n');
}

export async function reflect(
  repos: Repos,
  memory: MemoryStore,
  opts: ReflectOpts = {},
): Promise<ReflectResult> {
  const now = (opts.now ?? Date.now)();
  const windowMs = opts.windowMs ?? DEFAULT_WINDOW;
  const minSamples = opts.minSamples ?? 3;
  const minConfidence = opts.minConfidence ?? 0.4;

  const events = repos.events.recentSince(now - windowMs, 5000);
  const known = new Set(repos.adventurers.list().map((a) => a.name));
  const all = analyzeEvents(events, {
    minSamples,
    knownAdventurers: known,
  });
  const candidates = all.filter((c) => c.confidence >= minConfidence);

  const written: string[] = [];
  for (const c of candidates) {
    const name = `${new Date(now).toISOString().slice(0, 10)}-${c.id}`;
    const finalName = memory.writePattern(name, renderPatternMarkdown(c, now), now);
    written.push(finalName);
  }

  // proposal inmail (사용자 승인 게이트)
  // dedupe key는 (id, YYYY-MM) 버킷 — 같은 후보가 다음 달엔 재제안 가능 (M1)
  let proposed = 0;
  const monthBucket = new Date(now).toISOString().slice(0, 7);
  for (const c of candidates) {
    const r = repos.inmail.enqueue({
      from_session: 'luida-brain',
      to_session: 'luida',
      kind: 'proposal',
      payload: {
        v: 1, // payload schema 버전
        kind: 'pattern_promotion',
        candidate: c,
        instruction: `승인하려면: luida promote-pattern ${c.id}`,
      },
      dedupe_key: `promote-proposal:${c.id}:${monthBucket}`,
    });
    if (r.inserted) proposed += 1;
  }

  if (candidates.length > 0) {
    memory.appendChronicle(
      `💡 reflect: 패턴 후보 ${candidates.length}건 발견 (${candidates.map((c) => c.id).join(', ')})`,
      now,
    );
  }

  return { candidates, written, proposed };
}

function parsePayload(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return {};
  }
}

function slug(s: string): string {
  return s.replace(/[^A-Za-z0-9_-]/g, '-').replace(/-+/g, '-');
}

// =========================================================================
// promote: 패턴 후보를 실제 relationships row로 승격
// =========================================================================

export type PromoteResult = {
  promoted: boolean;
  relationshipId: number;
  candidate: PatternCandidate;
};

/**
 * 패턴 id를 받아 relationships 테이블에 row를 만든다.
 * source='learned-promoted'로 표기 (사람 정의 룰과 구분).
 * 같은 id는 upsertByName으로 멱등.
 *
 * 정책 (C1 대응):
 *  - 기본 enabled=0 (비활성화 상태로 승급) — 사용자가 별도 명령으로 활성화
 *  - action 기본 'propose' (즉시 dispatch 안 함) — 사용자 검토 게이트 보존
 *  - { activate: true } 옵션 시 enabled=1 + action='auto_dispatch'로 완전 활성화
 *
 * 호출자는 보통 사용자 승인 후 (CLI: `luida promote-pattern <id> [--activate]`).
 */
export type PromoteOpts = {
  /** true면 즉시 활성 + auto_dispatch. 기본 false (propose + disabled) */
  activate?: boolean;
};

export function promotePattern(
  rels: RelationshipRepo,
  candidate: PatternCandidate,
  opts: PromoteOpts = {},
): PromoteResult {
  const activate = opts.activate ?? false;
  const r = rels.upsertByName({
    name: candidate.id,
    from_session: candidate.from,
    to_session: candidate.to,
    action: activate ? 'auto_dispatch' : 'propose',
    trigger_kind: 'quest_completed',
    trigger_config: { status: 'completed' },
    brief_template: candidate.proposedBriefTemplate,
    enabled: activate ? 1 : 0,
    source: 'learned-promoted',
    confidence: candidate.confidence,
  });
  return {
    promoted: r.created,
    relationshipId: r.id,
    candidate,
  };
}

/**
 * inmail proposal payload에서 PatternCandidate를 복원한다 (C2 대응).
 * `promote-pattern` 명령이 events 휘발성에 의존하지 않게 함.
 */
export function findCandidateInProposals(
  repos: Repos,
  id: string,
): PatternCandidate | null {
  const tail = repos.inmail.tail(500);
  for (const m of tail) {
    if (m.kind !== 'proposal') continue;
    try {
      const p = JSON.parse(m.payload) as {
        kind?: string;
        candidate?: PatternCandidate;
      };
      if (p?.kind === 'pattern_promotion' && p.candidate?.id === id) {
        return p.candidate;
      }
    } catch {
      // ignore
    }
  }
  return null;
}
