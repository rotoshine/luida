import { describe, expect, test } from 'bun:test';
import type { TavernSnapshot } from './state/load';
import { panelLength } from './App';

function makeSnap(over: Partial<TavernSnapshot>): TavernSnapshot {
  return {
    adventurers: [],
    activeQuests: [],
    recentQuests: [],
    recentInmail: [],
    questCountByAdventurer: new Map(),
    takenAt: 0,
    ...over,
  };
}

describe('panelLength', () => {
  test('returns adventurers length for panel 0', () => {
    const snap = makeSnap({
      adventurers: [{ name: 'a' } as any, { name: 'b' } as any],
    });
    expect(panelLength(0, snap)).toBe(2);
  });

  test('returns recentQuests length for panel 1', () => {
    const snap = makeSnap({ recentQuests: [{ id: 1 } as any] });
    expect(panelLength(1, snap)).toBe(1);
  });

  test('returns recentInmail length for panel 2', () => {
    const snap = makeSnap({
      recentInmail: [{ id: 1 } as any, { id: 2 } as any, { id: 3 } as any],
    });
    expect(panelLength(2, snap)).toBe(3);
  });

  test('panel 3 (chronicle) always 0 — Phase 5 활성화 전 placeholder', () => {
    const snap = makeSnap({
      adventurers: [{ name: 'a' } as any],
      recentQuests: [{ id: 1 } as any],
    });
    expect(panelLength(3, snap)).toBe(0);
  });
});
