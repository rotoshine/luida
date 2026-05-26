import { describe, expect, test } from 'bun:test';
import type { Inmail } from '@luida/core';
import { renderInmailPrompt } from './render';

function makeInmail(over: Partial<Inmail>): Inmail {
  return {
    id: 1,
    from_session: 'luida',
    to_session: 'agora',
    reply_to: null,
    quest_id: null,
    kind: 'info',
    payload: '{}',
    dedupe_key: null,
    created_at: 0,
    delivered_at: null,
    handled_at: null,
    ...over,
  };
}

describe('renderInmailPrompt', () => {
  test('includes id, sender, kind in header', () => {
    const out = renderInmailPrompt(
      makeInmail({ id: 42, from_session: 'luida', kind: 'info' }),
    );
    expect(out).toContain('#42');
    expect(out).toContain('luida');
    expect(out).toContain('info');
  });

  test('dispatch kind renders brief and instructions', () => {
    const out = renderInmailPrompt(
      makeInmail({
        kind: 'dispatch',
        payload: JSON.stringify({
          brief: 'schema 마이그레이션',
          branch: 'feat/x',
        }),
      }),
    );
    expect(out).toContain('새 의뢰');
    expect(out).toContain('schema 마이그레이션');
    expect(out).toContain('feat/x');
    expect(out).toContain('완료 후');
  });

  test('dispatch without brief in payload still renders header', () => {
    const out = renderInmailPrompt(
      makeInmail({
        kind: 'dispatch',
        payload: JSON.stringify({ note: 'no brief' }),
      }),
    );
    expect(out).toContain('새 의뢰');
  });

  test('non-dispatch kinds render JSON block', () => {
    const out = renderInmailPrompt(
      makeInmail({
        kind: 'ack',
        payload: JSON.stringify({ success: true, pr_url: 'https://x/pr/1' }),
      }),
    );
    expect(out).toContain('```json');
    expect(out).toContain('"success"');
    expect(out).toContain('https://x/pr/1');
  });

  test('invalid JSON payload falls back to string', () => {
    const out = renderInmailPrompt(
      makeInmail({ kind: 'info', payload: 'raw text not json' }),
    );
    expect(out).toContain('raw text not json');
  });
});
