// relationships.yaml 파서 + glob matcher.
//
// yaml 형식 예:
//   relationships:
//     - name: agora-schema-to-admin
//       from: agora
//       trigger:
//         kind: path_changed
//         paths:
//           - "prisma/**"
//           - "schema/**"
//       to: admin
//       action: auto_dispatch
//       brief_template: "agora schema 변경 ({files})을 admin에 반영"
//       enabled: true
//
// DB의 relationships 테이블에 동기화하려면 syncRelationshipsToDb 사용.

import type {
  RelationshipAction,
  RelationshipSource,
  RelationshipTriggerKind,
} from './schema';

export type YamlTriggerPathChanged = {
  kind: 'path_changed';
  paths: string[];
};

export type YamlTriggerQuestCompleted = {
  kind: 'quest_completed';
  /** 옵션 — 특정 status에서만 발화 */
  status?: 'completed' | 'failed' | 'needs_approval';
};

export type YamlTriggerTagPushed = {
  kind: 'tag_pushed';
  pattern?: string;
};

export type YamlTrigger =
  | YamlTriggerPathChanged
  | YamlTriggerQuestCompleted
  | YamlTriggerTagPushed;

export type YamlRelationship = {
  name?: string;
  from: string;
  trigger: YamlTrigger;
  to: string;
  action: RelationshipAction;
  brief_template?: string;
  enabled?: boolean;
};

export type YamlConfig = {
  relationships?: YamlRelationship[];
};

/**
 * yaml 텍스트 파싱.
 * Bun.YAML(있으면) 사용 → fallback: 매우 제한된 자체 파서.
 * 자체 파서는 docs/examples/relationships.yaml 수준의 구조만 지원.
 */
export function parseRelationshipsYaml(text: string): YamlRelationship[] {
  const parsed = parseYaml(text) as YamlConfig | YamlRelationship[] | null;
  if (!parsed) return [];
  const list = Array.isArray(parsed) ? parsed : parsed.relationships ?? [];
  return list.filter(isValidYamlRelationship);
}

function parseYaml(text: string): unknown {
  // Bun 1.3+ 자체 YAML 파서가 있다면 사용
  const bunAny = (globalThis as { Bun?: { YAML?: { parse?: (s: string) => unknown } } })
    .Bun;
  if (bunAny?.YAML?.parse) {
    return bunAny.YAML.parse(text);
  }
  // Fallback: 제한된 yaml 파서
  return parseSimpleYaml(text);
}

function isValidYamlRelationship(v: unknown): v is YamlRelationship {
  if (typeof v !== 'object' || v === null) return false;
  const r = v as Partial<YamlRelationship>;
  if (typeof r.from !== 'string') return false;
  if (typeof r.to !== 'string') return false;
  if (r.action !== 'auto_dispatch' && r.action !== 'propose') return false;
  if (typeof r.trigger !== 'object' || r.trigger == null) return false;
  const t = r.trigger as { kind?: string; paths?: unknown };
  if (
    t.kind !== 'path_changed' &&
    t.kind !== 'quest_completed' &&
    t.kind !== 'tag_pushed'
  ) {
    return false;
  }
  if (t.kind === 'path_changed' && !Array.isArray(t.paths)) return false;
  return true;
}

// =========================
// Glob matcher
// =========================

/** glob 패턴을 RegExp로 변환. `**`, `*`, `?` 지원 */
export function globToRegex(glob: string): RegExp {
  let out = '^';
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i]!;
    if (c === '*') {
      if (glob[i + 1] === '*') {
        out += '.*';
        i++;
        // **/  → .*  (선택 slash 흡수)
        if (glob[i + 1] === '/') i++;
      } else {
        out += '[^/]*';
      }
    } else if (c === '?') {
      out += '[^/]';
    } else if (/[.+()[\]{}|^$\\]/.test(c)) {
      out += '\\' + c;
    } else {
      out += c;
    }
  }
  out += '$';
  return new RegExp(out);
}

export function pathMatchesAny(path: string, patterns: string[]): boolean {
  for (const p of patterns) {
    if (globToRegex(p).test(path)) return true;
  }
  return false;
}

export function pathsMatchingAny(
  paths: string[],
  patterns: string[],
): string[] {
  return paths.filter((p) => pathMatchesAny(p, patterns));
}

// =========================
// Source identifier
// =========================

export type RelationshipSyncSource = RelationshipSource;

// =========================
// Minimal YAML fallback
// =========================
// 한정된 형태만 지원: 들여쓰기 2칸, list는 '- ', flow scalar는 따옴표 또는 평문.
// 충분한 형태로 docs/examples/relationships.yaml 수준만 처리.

function parseSimpleYaml(text: string): unknown {
  // inline flow 형식({...}/[...])은 fallback이 지원 못 함 — Bun.YAML 없으면 명시 에러
  if (/:\s*[{[]/.test(text)) {
    throw new Error(
      'fallback yaml parser는 inline flow({...}, [...])를 지원하지 않습니다. ' +
        'Bun 1.3+ (Bun.YAML)를 사용하거나 block 스타일로 작성하세요.',
    );
  }
  const lines = text
    .replace(/\r\n/g, '\n')
    .split('\n')
    .map((l) => l.replace(/\t/g, '  '));
  type Ctx = { obj: Record<string, unknown> | unknown[]; indent: number };
  const root: Record<string, unknown> = {};
  const stack: Ctx[] = [{ obj: root, indent: -1 }];

  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i]!;
    if (!raw.trim() || raw.trim().startsWith('#')) continue;
    const indent = raw.length - raw.trimStart().length;
    const content = raw.trim();

    while (stack.length > 1 && stack[stack.length - 1]!.indent >= indent) {
      stack.pop();
    }
    const top = stack[stack.length - 1]!;

    if (content.startsWith('- ')) {
      // list item
      const after = content.slice(2).trim();
      let item: unknown;
      if (after.includes(': ')) {
        item = {};
        const idx = after.indexOf(': ');
        const k = after.slice(0, idx).trim();
        const v = parseScalar(after.slice(idx + 2).trim());
        (item as Record<string, unknown>)[k] = v;
        if (!Array.isArray(top.obj)) {
          throw new Error('parser error: list under non-list');
        }
        top.obj.push(item);
        stack.push({ obj: item as Record<string, unknown>, indent });
      } else {
        item = parseScalar(after);
        if (!Array.isArray(top.obj)) {
          throw new Error('parser error: list under non-list');
        }
        top.obj.push(item);
      }
    } else if (content.endsWith(':')) {
      const key = content.slice(0, -1).trim();
      // 다음 줄을 보고 list/object 결정
      const next = lines.slice(i + 1).find((l) => l.trim() !== '' && !l.trim().startsWith('#'));
      const childIndent = next ? next.length - next.trimStart().length : indent + 2;
      const isList = next?.trim().startsWith('- ') ?? false;
      const child: Record<string, unknown> | unknown[] = isList ? [] : {};
      if (Array.isArray(top.obj)) {
        throw new Error('parser error: key under list (not yet supported)');
      }
      top.obj[key] = child;
      stack.push({ obj: child, indent });
      void childIndent;
    } else if (content.includes(': ')) {
      const idx = content.indexOf(': ');
      const k = content.slice(0, idx).trim();
      const v = parseScalar(content.slice(idx + 2).trim());
      if (Array.isArray(top.obj)) {
        throw new Error('parser error: key under list');
      }
      top.obj[k] = v;
    }
  }
  return root;
}

function parseScalar(raw: string): unknown {
  const s = raw.trim();
  if (s === 'true') return true;
  if (s === 'false') return false;
  if (s === 'null' || s === '~') return null;
  if (/^-?\d+(\.\d+)?$/.test(s)) return Number(s);
  if ((s.startsWith('"') && s.endsWith('"')) || (s.startsWith("'") && s.endsWith("'"))) {
    return s.slice(1, -1);
  }
  return s;
}
