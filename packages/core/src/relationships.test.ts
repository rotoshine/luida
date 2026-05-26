import { describe, expect, test } from 'bun:test';
import {
  globToRegex,
  parseRelationshipsYaml,
  pathMatchesAny,
  pathsMatchingAny,
} from './relationships';

describe('globToRegex', () => {
  test('simple star matches single segment', () => {
    expect(globToRegex('src/*.ts').test('src/db.ts')).toBe(true);
    expect(globToRegex('src/*.ts').test('src/repo/db.ts')).toBe(false);
  });

  test('double star matches deep paths', () => {
    expect(globToRegex('prisma/**').test('prisma/schema.prisma')).toBe(true);
    expect(globToRegex('prisma/**').test('prisma/nested/sub/file.sql')).toBe(
      true,
    );
    expect(globToRegex('prisma/**').test('other/x.prisma')).toBe(false);
  });

  test('escapes regex special chars', () => {
    expect(globToRegex('a.b/c+d').test('a.b/c+d')).toBe(true);
    expect(globToRegex('a.b/c+d').test('axb/c+d')).toBe(false);
  });

  test('question mark matches single char', () => {
    expect(globToRegex('?.ts').test('x.ts')).toBe(true);
    expect(globToRegex('?.ts').test('xy.ts')).toBe(false);
  });
});

describe('pathMatchesAny / pathsMatchingAny', () => {
  test('any pattern matches', () => {
    expect(pathMatchesAny('prisma/schema.prisma', ['src/**', 'prisma/**'])).toBe(
      true,
    );
    expect(pathMatchesAny('docs/readme.md', ['src/**', 'prisma/**'])).toBe(false);
  });

  test('filters changed files', () => {
    const changed = [
      'prisma/schema.prisma',
      'src/api.ts',
      'docs/notes.md',
    ];
    expect(pathsMatchingAny(changed, ['prisma/**'])).toEqual([
      'prisma/schema.prisma',
    ]);
    expect(pathsMatchingAny(changed, ['**/*.ts'])).toEqual(['src/api.ts']);
  });
});

describe('parseRelationshipsYaml', () => {
  test('parses basic rule', () => {
    const yaml = `
relationships:
  - name: agora-schema-to-admin
    from: agora
    trigger:
      kind: path_changed
      paths:
        - "prisma/**"
        - "schema/**"
    to: admin
    action: auto_dispatch
    brief_template: "agora schema 변경을 admin에 반영"
    enabled: true
`;
    const rels = parseRelationshipsYaml(yaml);
    expect(rels.length).toBe(1);
    const r = rels[0]!;
    expect(r.from).toBe('agora');
    expect(r.to).toBe('admin');
    expect(r.action).toBe('auto_dispatch');
    expect(r.enabled).toBe(true);
    expect(r.trigger.kind).toBe('path_changed');
    if (r.trigger.kind === 'path_changed') {
      expect(r.trigger.paths).toEqual(['prisma/**', 'schema/**']);
    }
  });

  test('parses multiple rules', () => {
    const yaml = `
relationships:
  - from: a
    to: b
    action: auto_dispatch
    trigger:
      kind: path_changed
      paths:
        - "**"
  - from: b
    to: c
    action: propose
    trigger:
      kind: quest_completed
`;
    const rels = parseRelationshipsYaml(yaml);
    expect(rels.length).toBe(2);
    expect(rels[1]?.action).toBe('propose');
    expect(rels[1]?.trigger.kind).toBe('quest_completed');
  });

  test('rejects invalid rules silently', () => {
    const yaml = `
relationships:
  - from: a
    to: b
    action: shout_attack
    trigger:
      kind: path_changed
      paths:
        - "**"
`;
    expect(parseRelationshipsYaml(yaml)).toEqual([]);
  });

  test('empty text returns []', () => {
    expect(parseRelationshipsYaml('')).toEqual([]);
  });
});
