import { describe, expect, test } from 'bun:test';
import { extractSummary } from './EventLogLine';

describe('extractSummary', () => {
  test('picks brief first', () => {
    expect(
      extractSummary(JSON.stringify({ brief: 'b1', summary: 's1' })),
    ).toBe('b1');
  });
  test('falls back to summary', () => {
    expect(extractSummary(JSON.stringify({ summary: 's2' }))).toBe('s2');
  });
  test('falls back to msg', () => {
    expect(extractSummary(JSON.stringify({ msg: 'm1' }))).toBe('m1');
  });
  test('falls back to question', () => {
    expect(extractSummary(JSON.stringify({ question: 'OK?' }))).toBe('OK?');
  });
  test('JSON without known fields → JSON string', () => {
    expect(extractSummary(JSON.stringify({ a: 1 }))).toContain('"a"');
  });
  test('non-JSON → raw substring', () => {
    expect(extractSummary('not json text here')).toContain('not json');
  });
});
