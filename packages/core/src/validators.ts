// Phase E: 외부 입력면 (inmail payload, MCP tool input) 검증 통일.
// 의존성 0 — 가벼운 자체 schema 함수. 향후 Zod로 교체 가능하지만 표면이 작아 충분.

export type ValidationResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: string };

export type Validator<T> = (input: unknown) => ValidationResult<T>;

function fail<T>(error: string): ValidationResult<T> {
  return { ok: false, error };
}
function ok<T>(value: T): ValidationResult<T> {
  return { ok: true, value };
}

export const v = {
  string(input: unknown, opts: { min?: number; max?: number } = {}): ValidationResult<string> {
    if (typeof input !== 'string') return fail('expected string');
    if (opts.min != null && input.length < opts.min)
      return fail(`string must be ≥ ${opts.min} chars`);
    if (opts.max != null && input.length > opts.max)
      return fail(`string must be ≤ ${opts.max} chars`);
    return ok(input);
  },

  nonEmpty(input: unknown): ValidationResult<string> {
    const s = v.string(input);
    if (!s.ok) return s;
    if (!s.value.trim()) return fail('must be non-empty');
    return ok(s.value);
  },

  number(input: unknown, opts: { min?: number; max?: number; integer?: boolean } = {}): ValidationResult<number> {
    const n = typeof input === 'string' ? Number(input) : input;
    if (typeof n !== 'number' || !Number.isFinite(n))
      return fail('expected number');
    if (opts.integer && !Number.isInteger(n)) return fail('expected integer');
    if (opts.min != null && n < opts.min) return fail(`number must be ≥ ${opts.min}`);
    if (opts.max != null && n > opts.max) return fail(`number must be ≤ ${opts.max}`);
    return ok(n);
  },

  boolean(input: unknown): ValidationResult<boolean> {
    if (typeof input === 'boolean') return ok(input);
    if (input === 'true') return ok(true);
    if (input === 'false') return ok(false);
    return fail('expected boolean');
  },

  literal<T extends string>(allowed: readonly T[]) {
    return (input: unknown): ValidationResult<T> => {
      if (typeof input !== 'string') return fail('expected string literal');
      if (!allowed.includes(input as T))
        return fail(`expected one of ${allowed.join('|')}, got '${input}'`);
      return ok(input as T);
    };
  },

  object(input: unknown): ValidationResult<Record<string, unknown>> {
    if (typeof input !== 'object' || input === null || Array.isArray(input))
      return fail('expected object');
    return ok(input as Record<string, unknown>);
  },

  array(input: unknown): ValidationResult<unknown[]> {
    if (!Array.isArray(input)) return fail('expected array');
    return ok(input);
  },

  optional<T>(validator: Validator<T>) {
    return (input: unknown): ValidationResult<T | undefined> => {
      if (input == null) return ok(undefined);
      return validator(input);
    };
  },
};

// =========================================================================
// 도메인 스키마
// =========================================================================

export type DispatchPayloadValidated = {
  brief: string;
  branch?: string;
  base?: string;
  pr_title?: string;
};

/** dispatch inmail payload 검증 */
export function validateDispatchPayload(
  input: unknown,
): ValidationResult<DispatchPayloadValidated> {
  const obj = v.object(input);
  if (!obj.ok) return obj;

  const briefR = v.nonEmpty(obj.value.brief);
  if (!briefR.ok) return fail(`brief: ${briefR.error}`);

  const branch =
    obj.value.branch != null
      ? v.string(obj.value.branch, { min: 1, max: 200 })
      : ok(undefined);
  if (!branch.ok) return fail(`branch: ${branch.error}`);

  const base =
    obj.value.base != null ? v.string(obj.value.base) : ok(undefined);
  if (!base.ok) return fail(`base: ${base.error}`);

  const pr_title =
    obj.value.pr_title != null ? v.string(obj.value.pr_title) : ok(undefined);
  if (!pr_title.ok) return fail(`pr_title: ${pr_title.error}`);

  return ok({
    brief: briefR.value,
    branch: branch.value,
    base: base.value,
    pr_title: pr_title.value,
  });
}

export type QuestDispatchToolInput = {
  to: string;
  brief: string;
  branch?: string;
  base?: string;
  pr_title?: string;
};

/** MCP quest.dispatch tool input 검증 */
export function validateQuestDispatchInput(
  input: unknown,
): ValidationResult<QuestDispatchToolInput> {
  const obj = v.object(input);
  if (!obj.ok) return obj;

  const toR = v.nonEmpty(obj.value.to);
  if (!toR.ok) return fail(`to: ${toR.error}`);
  if (toR.value.startsWith('@'))
    return fail('to: broadcast 주소(@all)에는 dispatch 불가');

  const briefR = v.nonEmpty(obj.value.brief);
  if (!briefR.ok) return fail(`brief: ${briefR.error}`);

  const branch =
    obj.value.branch != null
      ? v.string(obj.value.branch, { min: 1, max: 200 })
      : ok(undefined);
  if (!branch.ok) return fail(`branch: ${branch.error}`);

  return ok({
    to: toR.value,
    brief: briefR.value,
    branch: branch.value,
    base: typeof obj.value.base === 'string' ? obj.value.base : undefined,
    pr_title:
      typeof obj.value.pr_title === 'string' ? obj.value.pr_title : undefined,
  });
}

export type MemoryRecordToolInput = {
  type: 'chronicle' | 'project' | 'pattern';
  name?: string;
  content: string;
};

export function validateMemoryRecordInput(
  input: unknown,
): ValidationResult<MemoryRecordToolInput> {
  const obj = v.object(input);
  if (!obj.ok) return obj;

  const typeR = v.literal(['chronicle', 'project', 'pattern'] as const)(
    obj.value.type,
  );
  if (!typeR.ok) return fail(`type: ${typeR.error}`);

  const contentR = v.string(obj.value.content);
  if (!contentR.ok) return fail(`content: ${contentR.error}`);

  const name =
    obj.value.name != null ? v.string(obj.value.name) : ok(undefined);
  if (!name.ok) return fail(`name: ${name.error}`);

  if (typeR.value !== 'chronicle' && !name.value)
    return fail(`name 필수 (type='${typeR.value}'일 때)`);

  return ok({ type: typeR.value, name: name.value, content: contentR.value });
}

export type QuestGetToolInput = { id: number };

export function validateQuestGetInput(
  input: unknown,
): ValidationResult<QuestGetToolInput> {
  const obj = v.object(input);
  if (!obj.ok) return obj;
  const idR = v.number(obj.value.id, { integer: true, min: 1 });
  if (!idR.ok) return fail(`id: ${idR.error}`);
  return ok({ id: idR.value });
}
