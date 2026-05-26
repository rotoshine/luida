import { existsSync, mkdirSync } from 'node:fs';
import { readFile, readdir } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { Database, SQLiteError } from 'bun:sqlite';

/**
 * `~/.luida/tavern.db` 기본 경로. `LUIDA_DB_PATH` 환경변수로 override 가능.
 * 함수로 노출되어 매 호출마다 env를 다시 읽으므로 테스트에서도 동적 변경 가능.
 */
export function getDefaultDbPath(): string {
  return process.env.LUIDA_DB_PATH ?? join(homedir(), '.luida', 'tavern.db');
}

/** 마이그레이션 SQL 파일들이 있는 디렉터리. `LUIDA_MIGRATIONS_DIR`로 override 가능. */
export function getMigrationsDir(): string {
  return (
    process.env.LUIDA_MIGRATIONS_DIR ?? join(import.meta.dir, '..', 'migrations')
  );
}

/** 마이그레이션 파일 이름 규약: `NNNN_<slug>.sql` (4자리 zero-padded prefix) */
const MIGRATION_FILE_PATTERN = /^\d{4}_[A-Za-z0-9_-]+\.sql$/;

/**
 * tavern.db 연결을 열고 표준 PRAGMA를 적용한다.
 *  - journal_mode=WAL          : 멀티 reader + 1 writer
 *  - synchronous=NORMAL        : WAL과 함께 안전·빠름 (전원 손실 시 마지막 tx만 유실)
 *  - busy_timeout=5000ms       : 동시 접근 충돌을 대기로 흡수
 *  - foreign_keys=ON           : FK 제약 강제
 *  - wal_autocheckpoint=1000   : WAL 파일이 무제한 커지지 않게 주기 체크포인트
 *  - cache_size=-20000         : 약 20MB 페이지 캐시 (음수=KB)
 */
export function openDb(path: string = getDefaultDbPath()): Database {
  const dir = dirname(path);
  if (dir && dir !== '.' && !existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }
  const db = new Database(path);
  db.exec('PRAGMA journal_mode = WAL');
  db.exec('PRAGMA synchronous = NORMAL');
  db.exec('PRAGMA busy_timeout = 5000');
  db.exec('PRAGMA foreign_keys = ON');
  db.exec('PRAGMA wal_autocheckpoint = 1000');
  db.exec('PRAGMA cache_size = -20000');
  return db;
}

/**
 * 1회성 사용을 위한 안전 래퍼. 에러 발생시에도 db.close()를 보장한다.
 *
 * @example
 *   await withDb(async (db) => {
 *     await migrate(db);
 *   });
 */
export async function withDb<T>(
  fn: (db: Database) => T | Promise<T>,
  path: string = getDefaultDbPath(),
): Promise<T> {
  const db = openDb(path);
  try {
    return await fn(db);
  } finally {
    db.close();
  }
}

export type MigrationResult = {
  applied: string[];
  alreadyApplied: string[];
};

/**
 * `migrations` 디렉터리의 `NNNN_*.sql`을 사전순으로 적용한다.
 *
 * 보증:
 *  - schema_migrations 테이블에 적용 이력을 남겨 idempotent
 *  - 전체 처리를 단일 IMMEDIATE 트랜잭션으로 묶어 동시 실행 race를 차단
 *    (concurrent migrate 호출은 busy_timeout 동안 대기 후 직렬화됨)
 *  - 한 파일의 SQL 실행이 실패하면 그 파일 + 이후 파일은 모두 롤백되고
 *    schema_migrations에 row도 남지 않음 (atomicity)
 *  - 파일 이름 규약(`/^\d{4}_.+\.sql$/`)에 맞지 않는 파일은 무시
 */
export async function migrate(
  db: Database,
  migrationsDir: string = getMigrationsDir(),
): Promise<MigrationResult> {
  // 부트스트랩은 트랜잭션 바깥. IF NOT EXISTS라 idempotent.
  db.exec(`
    CREATE TABLE IF NOT EXISTS schema_migrations (
      id          TEXT PRIMARY KEY,
      applied_at  INTEGER NOT NULL
    )
  `);

  const allFiles = await readdir(migrationsDir);
  const files = allFiles
    .filter((f) => MIGRATION_FILE_PATTERN.test(f))
    .sort();

  // I/O를 트랜잭션 밖에서 미리 완료해 lock hold 시간 최소화.
  const fileEntries: Array<{ name: string; sql: string }> = [];
  for (const name of files) {
    const sql = await readFile(join(migrationsDir, name), 'utf8');
    fileEntries.push({ name, sql });
  }

  const applied: string[] = [];
  const alreadyApplied: string[] = [];

  const runAll = db.transaction(() => {
    const appliedRows = db
      .query<{ id: string }, []>('SELECT id FROM schema_migrations')
      .all();
    const appliedSet = new Set(appliedRows.map((r) => r.id));

    const insertStmt = db.prepare(
      'INSERT INTO schema_migrations (id, applied_at) VALUES (?, ?)',
    );

    for (const { name, sql } of fileEntries) {
      if (appliedSet.has(name)) {
        alreadyApplied.push(name);
        continue;
      }
      db.exec(sql);
      insertStmt.run(name, Date.now());
      applied.push(name);
    }
  });

  // IMMEDIATE: BEGIN IMMEDIATE — 다른 writer 차단 (concurrent migrate 직렬화)
  runAll.immediate();

  return { applied, alreadyApplied };
}

/** SqliteError를 사람이 읽기 좋은 메시지로 변환 */
export function formatDbError(err: unknown): string {
  if (err instanceof SQLiteError) {
    return `SQLite ${err.code ?? 'ERROR'}: ${err.message}`;
  }
  if (err instanceof Error) {
    return err.message;
  }
  return String(err);
}
