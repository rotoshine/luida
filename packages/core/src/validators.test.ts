import { describe, expect, test } from 'bun:test';
import {
  v,
  validateDispatchPayload,
  validateMemoryRecordInput,
  validateQuestDispatchInput,
  validateQuestGetInput,
} from './validators';

describe('v primitives', () => {
  test('string', () => {
    expect(v.string('hi').ok).toBe(true);
    expect(v.string(123).ok).toBe(false);
    expect(v.string('hi', { min: 3 }).ok).toBe(false);
    expect(v.string('hello', { max: 3 }).ok).toBe(false);
  });
  test('nonEmpty', () => {
    expect(v.nonEmpty('hi').ok).toBe(true);
    expect(v.nonEmpty('   ').ok).toBe(false);
    expect(v.nonEmpty('').ok).toBe(false);
  });
  test('number with coercion', () => {
    expect(v.number(42).ok).toBe(true);
    expect(v.number('42').ok).toBe(true);
    expect(v.number('abc').ok).toBe(false);
    expect(v.number(1.5, { integer: true }).ok).toBe(false);
    expect(v.number(5, { min: 1, max: 10 }).ok).toBe(true);
  });
  test('literal', () => {
    const c = v.literal(['a', 'b', 'c'] as const);
    expect(c('a').ok).toBe(true);
    expect(c('z').ok).toBe(false);
  });
});

describe('validateDispatchPayload', () => {
  test('valid', () => {
    const r = validateDispatchPayload({
      brief: '스키마 작업',
      branch: 'feat/x',
    });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value.brief).toBe('스키마 작업');
  });
  test('missing brief', () => {
    expect(validateDispatchPayload({}).ok).toBe(false);
  });
  test('empty brief', () => {
    expect(validateDispatchPayload({ brief: '   ' }).ok).toBe(false);
  });
  test('non-object', () => {
    expect(validateDispatchPayload(null).ok).toBe(false);
    expect(validateDispatchPayload('string').ok).toBe(false);
  });
});

describe('validateQuestDispatchInput', () => {
  test('valid', () => {
    const r = validateQuestDispatchInput({ to: 'agora', brief: 'x' });
    expect(r.ok).toBe(true);
  });
  test('broadcast 차단', () => {
    const r = validateQuestDispatchInput({ to: '@all', brief: 'x' });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toContain('broadcast');
  });
  test('missing to', () => {
    expect(validateQuestDispatchInput({ brief: 'x' }).ok).toBe(false);
  });
});

describe('validateMemoryRecordInput', () => {
  test('chronicle without name OK', () => {
    expect(
      validateMemoryRecordInput({ type: 'chronicle', content: 'x' }).ok,
    ).toBe(true);
  });
  test('project requires name', () => {
    const r = validateMemoryRecordInput({ type: 'project', content: 'x' });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toContain('name');
  });
  test('invalid type', () => {
    expect(
      validateMemoryRecordInput({ type: 'unknown', content: 'x' }).ok,
    ).toBe(false);
  });
});

describe('validateQuestGetInput', () => {
  test('coerces string id', () => {
    const r = validateQuestGetInput({ id: '42' });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value.id).toBe(42);
  });
  test('rejects 0/negative', () => {
    expect(validateQuestGetInput({ id: 0 }).ok).toBe(false);
    expect(validateQuestGetInput({ id: -1 }).ok).toBe(false);
  });
});
