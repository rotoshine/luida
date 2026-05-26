// tavern.db에서 UI가 필요한 상태를 한 번에 로드하는 순수 함수.
// React hook은 이 함수를 호출해 polling — 함수 자체는 React 없이 단위 테스트 가능.

import {
  type Adventurer,
  type Inmail,
  type Quest,
  type Repos,
} from '@luida/core';

export type TavernSnapshot = {
  adventurers: Adventurer[];
  /** dispatched_to 별 active quest 카운트 (Level 환산용) */
  questCountByAdventurer: Map<string, number>;
  /** 활성 quest (상태 not in completed/failed/aborted) */
  activeQuests: Quest[];
  /** 최근 quest (active + 완료 일부 섞임), 우상단 의뢰 게시판용 */
  recentQuests: Quest[];
  /** 최근 inmail 50건 */
  recentInmail: Inmail[];
  /** 스냅샷 시각 */
  takenAt: number;
};

export function loadSnapshot(repos: Repos, now: number = Date.now()): TavernSnapshot {
  const adventurers = repos.adventurers.list();
  const activeQuests = repos.quests.listActive();
  const recentInmail = repos.inmail.tail(50);

  const questCountByAdventurer = new Map<string, number>();
  for (const q of activeQuests) {
    const k = q.dispatched_to;
    questCountByAdventurer.set(k, (questCountByAdventurer.get(k) ?? 0) + 1);
  }

  // recentQuests: 활성 + 가장 최근 완료 5건 (UI 게시판용)
  const recentQuests = [...activeQuests];

  return {
    adventurers,
    questCountByAdventurer,
    activeQuests,
    recentQuests,
    recentInmail,
    takenAt: now,
  };
}
