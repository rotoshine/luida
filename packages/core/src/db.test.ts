import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Database } from 'bun:sqlite';
import { getDefaultDbPath, migrate, openDb, withDb } from './db';

let tempDir: string;
let dbPath: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-test-'));
  dbPath = join(tempDir, 'test.db');
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

describe('openDb / PRAGMAs', () => {
  test('creates parent directory if missing', async () => {
    const deep = join(tempDir, 'a', 'b', 'c', 'tavern.db');
    const db = openDb(deep);
    await migrate(db);
    db.close();
    const db2 = openDb(deep);
    const tables = db2
      .query<{ name: string }, []>(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='adventurers'",
      )
      .all();
    expect(tables.length).toBe(1);
    db2.close();
  });

  test('WAL mode is enabled and core PRAGMAs applied', () => {
    const db = openDb(dbPath);
    const mode = db
      .query<{ journal_mode: string }, []>('PRAGMA journal_mode')
      .get();
    expect(mode?.journal_mode.toLowerCase()).toBe('wal');

    const sync = db.query<{ synchronous: number }, []>('PRAGMA synchronous').get();
    expect(sync?.synchronous).toBe(1); // NORMAL

    const fk = db.query<{ foreign_keys: number }, []>('PRAGMA foreign_keys').get();
    expect(fk?.foreign_keys).toBe(1); // ON

    db.close();
  });

  test('getDefaultDbPath reflects LUIDA_DB_PATH dynamically', () => {
    const saved = process.env.LUIDA_DB_PATH;
    try {
      process.env.LUIDA_DB_PATH = '/tmp/luida-explicit.db';
      expect(getDefaultDbPath()).toBe('/tmp/luida-explicit.db');
      delete process.env.LUIDA_DB_PATH;
      expect(getDefaultDbPath()).toContain('.luida/tavern.db');
    } finally {
      if (saved === undefined) delete process.env.LUIDA_DB_PATH;
      else process.env.LUIDA_DB_PATH = saved;
    }
  });
});

describe('withDb', () => {
  test('closes db after success', async () => {
    const result = await withDb(async (db) => {
      await migrate(db);
      return db
        .query<{ c: number }, []>("SELECT COUNT(*) AS c FROM sqlite_master")
        .get()?.c;
    }, dbPath);
    expect(result).toBeGreaterThan(0);
  });

  test('closes db even when fn throws', async () => {
    await expect(
      withDb(async () => {
        throw new Error('boom');
      }, dbPath),
    ).rejects.toThrow('boom');
    // 재오픈 가능 = 잠금이 풀려 있다는 신호
    const db = openDb(dbPath);
    expect(db).toBeDefined();
    db.close();
  });
});

describe('migrate — basic', () => {
  test('creates all expected tables', async () => {
    const db = openDb(dbPath);
    const result = await migrate(db);

    expect(result.applied).toEqual([
      '0001_init.sql',
      '0002_quest_source_inmail.sql',
    ]);

    const tables = db
      .query<{ name: string }, []>(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
      )
      .all();
    const tableNames = tables.map((t) => t.name);

    for (const expected of [
      'adventurers',
      'events',
      'inmail',
      'quests',
      'relationships',
      'schema_migrations',
    ]) {
      expect(tableNames).toContain(expected);
    }

    db.close();
  });

  test('creates all expected indexes', async () => {
    const db = openDb(dbPath);
    await migrate(db);

    const indexes = db
      .query<{ name: string }, []>(
        "SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name",
      )
      .all();
    const indexNames = indexes.map((i) => i.name);

    for (const expected of [
      'ix_events_kind',
      'ix_events_recent',
      'ix_inmail_pending',
      'ix_quest_active',
      'ix_quest_to',
      'ux_inmail_dedupe',
    ]) {
      expect(indexNames).toContain(expected);
    }

    db.close();
  });

  test('is idempotent on repeated calls', async () => {
    const db = openDb(dbPath);
    const first = await migrate(db);
    const second = await migrate(db);

    expect(first.applied.length).toBeGreaterThanOrEqual(1);
    expect(second.applied.length).toBe(0);
    expect(second.alreadyApplied).toContain('0001_init.sql');
    expect(second.alreadyApplied).toContain('0002_quest_source_inmail.sql');

    db.close();
  });
});

describe('migrate — atomicity & ordering', () => {
  test('failed migration rolls back AND leaves no schema_migrations row', async () => {
    const customDir = join(tempDir, 'mig-broken');
    await rm(customDir, { recursive: true, force: true });
    const { mkdir } = await import('node:fs/promises');
    await mkdir(customDir, { recursive: true });
    await writeFile(
      join(customDir, '0001_ok.sql'),
      'CREATE TABLE t1 (id INTEGER PRIMARY KEY);',
    );
    await writeFile(
      join(customDir, '0002_broken.sql'),
      'CREATE TABLE this is not valid sql;',
    );

    const db = openDb(dbPath);
    await expect(migrate(db, customDir)).rejects.toThrow();

    // 단일 트랜잭션이므로 0001도 함께 롤백되어야 한다.
    const t1 = db
      .query<{ name: string }, []>(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='t1'",
      )
      .all();
    expect(t1.length).toBe(0);

    const rows = db
      .query<{ id: string }, []>('SELECT id FROM schema_migrations ORDER BY id')
      .all();
    expect(rows.map((r) => r.id)).toEqual([]);

    db.close();
  });

  test('applies migrations in lexicographic order', async () => {
    const customDir = join(tempDir, 'mig-order');
    const { mkdir } = await import('node:fs/promises');
    await mkdir(customDir, { recursive: true });
    await writeFile(
      join(customDir, '0010_third.sql'),
      'CREATE TABLE ord_c (n INTEGER);',
    );
    await writeFile(
      join(customDir, '0001_first.sql'),
      'CREATE TABLE ord_a (n INTEGER);',
    );
    await writeFile(
      join(customDir, '0002_second.sql'),
      'CREATE TABLE ord_b (n INTEGER);',
    );

    const db = openDb(dbPath);
    const result = await migrate(db, customDir);
    expect(result.applied).toEqual([
      '0001_first.sql',
      '0002_second.sql',
      '0010_third.sql',
    ]);
    db.close();
  });

  test('ignores files not matching naming pattern', async () => {
    const customDir = join(tempDir, 'mig-pattern');
    const { mkdir } = await import('node:fs/promises');
    await mkdir(customDir, { recursive: true });
    await writeFile(join(customDir, '0001_ok.sql'), 'CREATE TABLE x (n INTEGER);');
    await writeFile(
      join(customDir, 'random_notes.sql'),
      'should be ignored',
    );
    await writeFile(join(customDir, '.DS_Store'), '');
    await writeFile(join(customDir, '01_short.sql'), 'CREATE TABLE bad (n INTEGER);');

    const db = openDb(dbPath);
    const result = await migrate(db, customDir);
    expect(result.applied).toEqual(['0001_ok.sql']);
    db.close();
  });

  test('concurrent migrate calls do not double-apply', async () => {
    // 같은 dbPath로 2개 connection을 열어 migrate를 동시에 시도.
    // IMMEDIATE 트랜잭션 + busy_timeout으로 직렬화되어야 한다.
    const db1 = openDb(dbPath);
    const db2 = openDb(dbPath);

    const [r1, r2] = await Promise.all([migrate(db1), migrate(db2)]);

    // 정확히 N번만 실제로 적용되어야 한다 (N = 마이그레이션 파일 수).
    const totalApplied = r1.applied.length + r2.applied.length;
    const totalFiles = r1.alreadyApplied.length + r1.applied.length;
    expect(totalApplied).toBe(totalFiles);

    const rows = db1
      .query<{ id: string }, []>('SELECT id FROM schema_migrations')
      .all();
    expect(rows.length).toBe(totalFiles);

    db1.close();
    db2.close();
  });
});

describe('schema — adventurers', () => {
  test('round-trip insert and select', async () => {
    const db = openDb(dbPath);
    await migrate(db);

    const now = Date.now();
    db.prepare(
      `INSERT INTO adventurers
       (name, workspace_id, surface_id, role, status, last_seen, registered_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)`,
    ).run('agora', 'ws-1', 'surf-1', 'worker', 'idle', now, now);

    const row = db
      .query<
        { name: string; role: string; status: string },
        [string]
      >('SELECT name, role, status FROM adventurers WHERE name = ?')
      .get('agora');

    expect(row?.name).toBe('agora');
    expect(row?.role).toBe('worker');
    expect(row?.status).toBe('idle');

    db.close();
  });

  test('rejects invalid role', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    expect(() => {
      db.prepare(
        `INSERT INTO adventurers
         (name, workspace_id, surface_id, role, status, last_seen, registered_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)`,
      ).run('x', 'w', 's', 'sorcerer', 'idle', now, now);
    }).toThrow();
    db.close();
  });

  test('rejects invalid status', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    expect(() => {
      db.prepare(
        `INSERT INTO adventurers
         (name, workspace_id, surface_id, role, status, last_seen, registered_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)`,
      ).run('x', 'w', 's', 'worker', 'sleeping', now, now);
    }).toThrow();
    db.close();
  });
});

describe('schema — quests', () => {
  function seedAdventurers(db: Database, now: number): void {
    db.prepare(
      `INSERT INTO adventurers
       (name, workspace_id, surface_id, role, status, last_seen, registered_at)
       VALUES ('luida', 'w', 's', 'main', 'idle', ?, ?),
              ('agora', 'w', 's', 'worker', 'idle', ?, ?)`,
    ).run(now, now, now, now);
  }

  test('round-trip with FK', async () => {
    const db = openDb(dbPath);
    await migrate(db);

    const now = Date.now();
    seedAdventurers(db, now);

    const result = db
      .prepare(
        `INSERT INTO quests
         (dispatched_by, dispatched_to, brief, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)`,
      )
      .run('luida', 'agora', '스키마 마이그레이션', 'pending', now, now);

    expect(Number(result.lastInsertRowid)).toBeGreaterThan(0);

    const row = db
      .query<
        { brief: string; status: string },
        []
      >('SELECT brief, status FROM quests ORDER BY id DESC LIMIT 1')
      .get();

    expect(row?.brief).toBe('스키마 마이그레이션');
    expect(row?.status).toBe('pending');

    db.close();
  });

  test('rejects unknown dispatched_to (FK)', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    expect(() => {
      db.prepare(
        `INSERT INTO quests
         (dispatched_by, dispatched_to, brief, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)`,
      ).run('ghost', 'phantom', 'should fail', 'pending', now, now);
    }).toThrow();
    db.close();
  });

  test('rejects invalid status', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    seedAdventurers(db, now);
    expect(() => {
      db.prepare(
        `INSERT INTO quests
         (dispatched_by, dispatched_to, brief, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)`,
      ).run('luida', 'agora', 'x', 'totally_invalid', now, now);
    }).toThrow();
    db.close();
  });

  test('parent_quest_id self-FK is enforced', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    seedAdventurers(db, now);

    // 존재하지 않는 parent → 거부
    expect(() => {
      db.prepare(
        `INSERT INTO quests
         (dispatched_by, dispatched_to, brief, status, parent_quest_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)`,
      ).run('luida', 'agora', 'child', 'pending', 9999, now, now);
    }).toThrow();

    db.close();
  });

  test('adventurer rename cascades to quests via ON UPDATE CASCADE', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    seedAdventurers(db, now);

    db.prepare(
      `INSERT INTO quests
       (dispatched_by, dispatched_to, brief, status, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    ).run('luida', 'agora', 'q1', 'running', now, now);

    db.prepare('UPDATE adventurers SET name = ? WHERE name = ?').run(
      'agora-renamed',
      'agora',
    );

    const row = db
      .query<{ dispatched_to: string }, []>(
        'SELECT dispatched_to FROM quests WHERE brief = "q1"',
      )
      .get();
    expect(row?.dispatched_to).toBe('agora-renamed');

    db.close();
  });
});

describe('schema — inmail', () => {
  test('dedupe is per (recipient, sender, key)', async () => {
    const db = openDb(dbPath);
    await migrate(db);

    const now = Date.now();
    db.prepare(
      `INSERT INTO inmail
       (from_session, to_session, kind, payload, dedupe_key, created_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    ).run('agora', 'admin', 'dispatch', '{}', 'abc', now);

    // 같은 (sender, recipient, key) → 거부 (OR IGNORE로 흡수)
    db.prepare(
      `INSERT OR IGNORE INTO inmail
       (from_session, to_session, kind, payload, dedupe_key, created_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    ).run('agora', 'admin', 'dispatch', '{}', 'abc', now + 1);

    // 다른 sender, 같은 key → 허용
    db.prepare(
      `INSERT INTO inmail
       (from_session, to_session, kind, payload, dedupe_key, created_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    ).run('kontrol', 'admin', 'dispatch', '{}', 'abc', now + 2);

    // 같은 sender, 다른 recipient, 같은 key → 허용
    db.prepare(
      `INSERT INTO inmail
       (from_session, to_session, kind, payload, dedupe_key, created_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    ).run('agora', 'kontrol', 'dispatch', '{}', 'abc', now + 3);

    const count =
      db
        .query<{ c: number }, []>(
          "SELECT COUNT(*) AS c FROM inmail WHERE dedupe_key = 'abc'",
        )
        .get()?.c ?? 0;
    expect(count).toBe(3);

    db.close();
  });

  test('dedupe_key NULL allows unlimited duplicates', async () => {
    const db = openDb(dbPath);
    await migrate(db);

    const now = Date.now();
    const stmt = db.prepare(
      `INSERT INTO inmail
       (from_session, to_session, kind, payload, dedupe_key, created_at)
       VALUES (?, ?, ?, ?, NULL, ?)`,
    );
    for (let i = 0; i < 5; i++) {
      stmt.run('agora', 'admin', 'info', '{}', now + i);
    }
    const count =
      db.query<{ c: number }, []>('SELECT COUNT(*) AS c FROM inmail').get()?.c ?? 0;
    expect(count).toBe(5);
    db.close();
  });

  test('rejects invalid kind', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    expect(() => {
      db.prepare(
        `INSERT INTO inmail
         (from_session, to_session, kind, payload, created_at)
         VALUES (?, ?, ?, ?, ?)`,
      ).run('agora', 'admin', 'shout', '{}', now);
    }).toThrow();
    db.close();
  });

  test('broadcast address (@all) allowed in to_session (no FK)', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    db.prepare(
      `INSERT INTO inmail
       (from_session, to_session, kind, payload, created_at)
       VALUES (?, ?, ?, ?, ?)`,
    ).run('luida', '@all', 'alert', '{}', now);
    const row = db
      .query<{ to_session: string }, []>(
        'SELECT to_session FROM inmail ORDER BY id DESC LIMIT 1',
      )
      .get();
    expect(row?.to_session).toBe('@all');
    db.close();
  });
});

describe('schema — relationships', () => {
  function seedAdv(db: Database, now: number): void {
    db.prepare(
      `INSERT INTO adventurers
       (name, workspace_id, surface_id, role, status, last_seen, registered_at)
       VALUES ('agora', 'w', 's', 'worker', 'idle', ?, ?),
              ('admin', 'w', 's', 'worker', 'idle', ?, ?)`,
    ).run(now, now, now, now);
  }

  test('rejects invalid action', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    seedAdv(db, now);
    expect(() => {
      db.prepare(
        `INSERT INTO relationships
         (name, from_session, trigger_kind, trigger_config, to_session, action, source, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run('bad', 'agora', 'path_changed', '{}', 'admin', 'send-troops', 'human', now);
    }).toThrow();
    db.close();
  });

  test('rejects invalid source', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    seedAdv(db, now);
    expect(() => {
      db.prepare(
        `INSERT INTO relationships
         (name, from_session, trigger_kind, trigger_config, to_session, action, source, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run('bad', 'agora', 'path_changed', '{}', 'admin', 'auto_dispatch', 'guessed', now);
    }).toThrow();
    db.close();
  });

  test('rejects invalid trigger_kind', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    seedAdv(db, now);
    expect(() => {
      db.prepare(
        `INSERT INTO relationships
         (name, from_session, trigger_kind, trigger_config, to_session, action, source, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run('bad', 'agora', 'star_aligned', '{}', 'admin', 'auto_dispatch', 'human', now);
    }).toThrow();
    db.close();
  });

  test('rejects invalid enabled (not 0/1)', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    seedAdv(db, now);
    expect(() => {
      db.prepare(
        `INSERT INTO relationships
         (name, from_session, trigger_kind, trigger_config, to_session, action, enabled, source, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run('bad', 'agora', 'path_changed', '{}', 'admin', 'auto_dispatch', 2, 'human', now);
    }).toThrow();
    db.close();
  });

  test('FK rejects unknown from_session/to_session', async () => {
    const db = openDb(dbPath);
    await migrate(db);
    const now = Date.now();
    // adventurers 미시드 상태
    expect(() => {
      db.prepare(
        `INSERT INTO relationships
         (name, from_session, trigger_kind, trigger_config, to_session, action, source, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run('bad', 'ghost', 'path_changed', '{}', 'phantom', 'auto_dispatch', 'human', now);
    }).toThrow();
    db.close();
  });
});

describe('schema — events', () => {
  test('accepts free-form kind by design', async () => {
    const db = openDb(dbPath);
    await migrate(db);

    const now = Date.now();
    db.prepare(
      `INSERT INTO events (actor, kind, payload, occurred_at)
       VALUES (?, ?, ?, ?)`,
    ).run('agora', 'custom_event_kind_xyz', '{}', now);

    const row = db
      .query<{ kind: string }, []>(
        'SELECT kind FROM events ORDER BY id DESC LIMIT 1',
      )
      .get();
    expect(row?.kind).toBe('custom_event_kind_xyz');
    db.close();
  });
});
