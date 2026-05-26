import type { Database, Statement } from 'bun:sqlite';
import {
  type Adventurer,
  type AdventurerRole,
  type AdventurerStatus,
  nowMs,
} from '../schema';

export type AdventurerUpsert = {
  name: string;
  workspace_id: string;
  surface_id: string;
  repo_path?: string | null;
  role: AdventurerRole;
  status?: AdventurerStatus;
  pid?: number | null;
};

export class AdventurerRepo {
  private readonly stmtUpsert: Statement<unknown, any[]>;
  private readonly stmtFindByName: Statement<Adventurer, [string]>;
  private readonly stmtList: Statement<Adventurer, []>;
  private readonly stmtUpdateStatus: Statement<
    unknown,
    [AdventurerStatus, number, string]
  >;

  constructor(db: Database) {
    this.stmtUpsert = db.prepare(`
      INSERT INTO adventurers
        (name, workspace_id, surface_id, repo_path, role, status, pid, last_seen, registered_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(name) DO UPDATE SET
        workspace_id  = excluded.workspace_id,
        surface_id    = excluded.surface_id,
        repo_path     = excluded.repo_path,
        role          = excluded.role,
        status        = excluded.status,
        pid           = excluded.pid,
        last_seen     = excluded.last_seen
    `);
    this.stmtFindByName = db.prepare('SELECT * FROM adventurers WHERE name = ?');
    this.stmtList = db.prepare('SELECT * FROM adventurers ORDER BY name');
    this.stmtUpdateStatus = db.prepare(
      'UPDATE adventurers SET status = ?, last_seen = ? WHERE name = ?',
    );
  }

  upsert(input: AdventurerUpsert): void {
    const now = nowMs();
    this.stmtUpsert.run(
      input.name,
      input.workspace_id,
      input.surface_id,
      input.repo_path ?? null,
      input.role,
      input.status ?? 'idle',
      input.pid ?? null,
      now,
      now,
    );
  }

  findByName(name: string): Adventurer | null {
    return this.stmtFindByName.get(name) ?? null;
  }

  list(): Adventurer[] {
    return this.stmtList.all();
  }

  updateStatus(name: string, status: AdventurerStatus): void {
    this.stmtUpdateStatus.run(status, nowMs(), name);
  }

  close(): void {
    this.stmtUpsert.finalize();
    this.stmtFindByName.finalize();
    this.stmtList.finalize();
    this.stmtUpdateStatus.finalize();
  }
}
