// 표시용 가상 스탯 계산 (DB 컬럼이 아니라 UI 레이어에서 도출).

import type { Adventurer, Quest } from '@luida/core';

export type AdventurerStats = {
  level: number;
  hp: { current: number; max: number };
  className: string;
};

const CLASS_MAP: Record<string, string> = {
  main: '술집 주인',
  brain: '현자',
  worker: '모험가',
};

export function deriveStats(
  adv: Adventurer,
  questCount: number,
): AdventurerStats {
  const max = 10;
  let current = max;
  if (adv.status === 'busy') current = Math.max(3, max - 4);
  if (adv.status === 'offline') current = 0;
  // 누적 quest를 level로 환산 (간단 공식)
  const level = Math.max(1, Math.floor(Math.sqrt(questCount * 2 + 1)));
  return {
    level,
    hp: { current, max },
    className: CLASS_MAP[adv.role] ?? adv.role,
  };
}

/** quest progress 비율(없으면 status 기반 추정) */
export function questProgressRatio(q: Quest): number {
  switch (q.status) {
    case 'pending':
      return 0;
    case 'running':
      return 0.4;
    case 'reviewing':
      return 0.7;
    case 'needs_approval':
      return 0.9;
    case 'pr_ready':
      return 0.95;
    case 'completed':
      return 1;
    case 'failed':
    case 'aborted':
      return 0;
  }
}

/** 첫 줄만 (요약용) */
export function firstLine(s: string | null | undefined, max = 60): string {
  if (!s) return '';
  const line = s.split('\n')[0] ?? '';
  if (line.length <= max) return line;
  return [...line].slice(0, max - 1).join('') + '…';
}

/** 한국어 친화적 상대시각 */
export function relativeTime(ms: number, now: number = Date.now()): string {
  const diff = now - ms;
  if (diff < 1000) return '방금';
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}초 전`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}분 전`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}시간 전`;
  const d = Math.floor(h / 24);
  return `${d}일 전`;
}
