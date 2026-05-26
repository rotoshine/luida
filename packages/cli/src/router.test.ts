import { describe, expect, test } from 'bun:test';
import { Router } from './router';

function buildRouter(): Router {
  return new Router()
    .register({
      key: 'db init',
      desc: 'init',
      handler: async () => {},
    })
    .register({
      key: 'sidecar',
      desc: 'run sidecar',
      handler: async () => {},
    });
}

describe('Router', () => {
  test('matches simple command', () => {
    const r = buildRouter().resolve(['sidecar']);
    expect(r?.command.key).toBe('sidecar');
  });

  test('matches multi-word command', () => {
    const r = buildRouter().resolve(['db', 'init']);
    expect(r?.command.key).toBe('db init');
  });

  test('parses --key=value', () => {
    const r = buildRouter().resolve(['sidecar', '--me=agora']);
    expect(r?.ctx.options.me).toBe('agora');
  });

  test('parses --key value', () => {
    const r = buildRouter().resolve(['sidecar', '--me', 'agora']);
    expect(r?.ctx.options.me).toBe('agora');
  });

  test('parses -k value', () => {
    const r = buildRouter().resolve(['sidecar', '-n', 'agora']);
    expect(r?.ctx.options.n).toBe('agora');
  });

  test('boolean flag without value', () => {
    const r = buildRouter().resolve(['sidecar', '--once']);
    expect(r?.ctx.options.once).toBe(true);
  });

  test('returns null on no match', () => {
    const r = buildRouter().resolve(['bogus']);
    expect(r).toBeNull();
  });

  test('prefers longest match', () => {
    const router = new Router()
      .register({ key: 'db', desc: '', handler: async () => {} })
      .register({ key: 'db init', desc: '', handler: async () => {} });
    expect(router.resolve(['db', 'init'])?.command.key).toBe('db init');
    expect(router.resolve(['db'])?.command.key).toBe('db');
  });

  test('formatHelp lists registered commands', () => {
    const help = buildRouter().formatHelp();
    expect(help).toContain('db init');
    expect(help).toContain('sidecar');
  });
});
