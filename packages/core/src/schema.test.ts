import { describe, expect, test } from 'bun:test';
import { isEnabled, isKnownEventKind, nowMs, toIso } from './schema';

describe('schema helpers', () => {
  test('isEnabled', () => {
    expect(isEnabled({ enabled: 1 })).toBe(true);
    expect(isEnabled({ enabled: 0 })).toBe(false);
  });

  test('isKnownEventKind narrows known kinds', () => {
    expect(isKnownEventKind('quest_dispatched')).toBe(true);
    expect(isKnownEventKind('pr_created')).toBe(true);
    expect(isKnownEventKind('user_approved')).toBe(true);
    expect(isKnownEventKind('totally_unknown')).toBe(false);
    expect(isKnownEventKind('')).toBe(false);
  });

  test('nowMs is a number near Date.now()', () => {
    const before = Date.now();
    const v = nowMs();
    const after = Date.now();
    expect(v).toBeGreaterThanOrEqual(before);
    expect(v).toBeLessThanOrEqual(after);
  });

  test('toIso produces ISO-8601 string', () => {
    const iso = toIso(0);
    expect(iso).toBe('1970-01-01T00:00:00.000Z');
    const fixed = toIso(1748241600000);
    expect(fixed).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
  });
});
