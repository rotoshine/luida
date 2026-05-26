import { describe, expect, test } from 'bun:test';
import type { Adventurer, Quest } from '@luida/core';
import {
  deriveStats,
  firstLine,
  questProgressRatio,
  relativeTime,
} from './stats';

function adv(over: Partial<Adventurer>): Adventurer {
  return {
    name: 'x',
    workspace_id: 'w',
    surface_id: 's',
    repo_path: null,
    role: 'worker',
    status: 'idle',
    pid: null,
    last_seen: 0,
    registered_at: 0,
    ...over,
  };
}

function quest(over: Partial<Quest>): Quest {
  return {
    id: 1,
    dispatched_by: 'a',
    dispatched_to: 'b',
    brief: '',
    branch: null,
    worktree_path: null,
    status: 'pending',
    progress: null,
    pr_url: null,
    log_path: null,
    parent_quest_id: null,
    source_inmail_id: null,
    created_at: 0,
    updated_at: 0,
    completed_at: null,
    ...over,
  };
}

describe('deriveStats', () => {
  test('idle adventurer has full HP', () => {
    const s = deriveStats(adv({ status: 'idle' }), 0);
    expect(s.hp.current).toBe(s.hp.max);
    expect(s.level).toBe(1);
  });

  test('busy adventurer has reduced HP', () => {
    const s = deriveStats(adv({ status: 'busy' }), 0);
    expect(s.hp.current).toBeLessThan(s.hp.max);
    expect(s.hp.current).toBeGreaterThan(0);
  });

  test('offline adventurer has 0 HP', () => {
    expect(deriveStats(adv({ status: 'offline' }), 0).hp.current).toBe(0);
  });

  test('level scales with quest count', () => {
    expect(deriveStats(adv({}), 0).level).toBeLessThan(
      deriveStats(adv({}), 100).level,
    );
  });

  test('role maps to Korean class name', () => {
    expect(deriveStats(adv({ role: 'main' }), 0).className).toBe('술집 주인');
    expect(deriveStats(adv({ role: 'brain' }), 0).className).toBe('현자');
    expect(deriveStats(adv({ role: 'worker' }), 0).className).toBe('모험가');
  });
});

describe('questProgressRatio', () => {
  test('terminal statuses give known values', () => {
    expect(questProgressRatio(quest({ status: 'completed' }))).toBe(1);
    expect(questProgressRatio(quest({ status: 'failed' }))).toBe(0);
    expect(questProgressRatio(quest({ status: 'pending' }))).toBe(0);
  });
  test('intermediate statuses are monotonic', () => {
    expect(questProgressRatio(quest({ status: 'running' }))).toBeLessThan(
      questProgressRatio(quest({ status: 'reviewing' })),
    );
    expect(questProgressRatio(quest({ status: 'reviewing' }))).toBeLessThan(
      questProgressRatio(quest({ status: 'needs_approval' })),
    );
  });
});

describe('firstLine', () => {
  test('returns single line, truncated', () => {
    expect(firstLine('hello world')).toBe('hello world');
    expect(firstLine('hello\nworld')).toBe('hello');
    expect(firstLine('a'.repeat(80), 10)).toContain('…');
  });
  test('handles null/undefined', () => {
    expect(firstLine(null)).toBe('');
    expect(firstLine(undefined)).toBe('');
  });
  test('multibyte safe', () => {
    expect(firstLine('가나다라마바사아자차', 5)).toBe('가나다라…');
  });
});

describe('relativeTime', () => {
  test('seconds/minutes/hours/days', () => {
    const now = 1_000_000_000;
    expect(relativeTime(now, now)).toBe('방금');
    expect(relativeTime(now - 30_000, now)).toBe('30초 전');
    expect(relativeTime(now - 5 * 60_000, now)).toBe('5분 전');
    expect(relativeTime(now - 3 * 3_600_000, now)).toBe('3시간 전');
    expect(relativeTime(now - 2 * 86_400_000, now)).toBe('2일 전');
  });
});
