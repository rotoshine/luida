import { describe, expect, test } from 'bun:test';
import { parseStreamLine } from './worker';

describe('parseStreamLine', () => {
  test('returns null on invalid JSON', () => {
    expect(parseStreamLine('not json')).toBeNull();
    expect(parseStreamLine('')).toBeNull();
    expect(parseStreamLine('{broken')).toBeNull();
  });

  test('returns null on unknown type', () => {
    expect(parseStreamLine('{"type":"unknown"}')).toBeNull();
    expect(parseStreamLine('{}')).toBeNull();
  });

  test('returns null on partial text/tool_use', () => {
    expect(parseStreamLine('{"type":"text"}')).toBeNull();
    expect(parseStreamLine('{"type":"tool_use"}')).toBeNull();
  });

  test('parses text', () => {
    expect(parseStreamLine('{"type":"text","text":"hi"}')).toEqual({
      kind: 'text',
      text: 'hi',
    });
  });

  test('parses tool_use', () => {
    expect(parseStreamLine('{"type":"tool_use","name":"Edit","input":{"x":1}}')).toEqual({
      kind: 'tool_use',
      name: 'Edit',
      input: { x: 1 },
    });
  });

  test('parses result (default success=true)', () => {
    expect(parseStreamLine('{"type":"result"}')).toEqual({
      kind: 'result',
      success: true,
      summary: undefined,
    });
  });

  test('parses result with is_error=true', () => {
    expect(parseStreamLine('{"type":"result","is_error":true}')).toEqual({
      kind: 'result',
      success: false,
      summary: undefined,
    });
  });

  test('parses result with subtype=error', () => {
    expect(parseStreamLine('{"type":"result","subtype":"error"}')).toEqual({
      kind: 'result',
      success: false,
      summary: undefined,
    });
  });

  test('parses result.summary from summary or result field', () => {
    expect(
      parseStreamLine('{"type":"result","summary":"ok"}')?.kind === 'result' &&
        (parseStreamLine('{"type":"result","summary":"ok"}') as any).summary,
    ).toBe('ok');
    expect(
      (parseStreamLine('{"type":"result","result":"done"}') as any).summary,
    ).toBe('done');
  });

  test('parses error', () => {
    expect(parseStreamLine('{"type":"error","message":"boom"}')).toEqual({
      kind: 'error',
      message: 'boom',
    });
  });

  test('parses system', () => {
    const ev = parseStreamLine('{"type":"system","subtype":"init"}');
    expect(ev?.kind).toBe('system');
    if (ev?.kind === 'system') {
      expect(ev.subtype).toBe('init');
    }
  });
});
