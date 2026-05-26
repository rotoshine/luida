import type { Database, Statement } from 'bun:sqlite';
import { type LuidaEvent, nowMs } from '../schema';

export type EventRecord = {
  quest_id?: number | null;
  actor: string;
  kind: string;
  payload: unknown;
};

export class EventRepo {
  private readonly stmtInsert: Statement<unknown, any[]>;
  private readonly stmtRecent: Statement<LuidaEvent, [number, number]>;
  private readonly stmtByKind: Statement<LuidaEvent, [string, number]>;

  constructor(db: Database) {
    this.stmtInsert = db.prepare(`
      INSERT INTO events (quest_id, actor, kind, payload, occurred_at)
      VALUES (?, ?, ?, ?, ?)
    `);
    this.stmtRecent = db.prepare(`
      SELECT * FROM events
      WHERE occurred_at >= ?
      ORDER BY occurred_at DESC
      LIMIT ?
    `);
    this.stmtByKind = db.prepare(`
      SELECT * FROM events
      WHERE kind = ?
      ORDER BY occurred_at DESC
      LIMIT ?
    `);
  }

  record(e: EventRecord): void {
    this.stmtInsert.run(
      e.quest_id ?? null,
      e.actor,
      e.kind,
      JSON.stringify(e.payload),
      nowMs(),
    );
  }

  recentSince(sinceMs: number, limit = 200): LuidaEvent[] {
    return this.stmtRecent.all(sinceMs, limit);
  }

  byKind(kind: string, limit = 100): LuidaEvent[] {
    return this.stmtByKind.all(kind, limit);
  }

  close(): void {
    this.stmtInsert.finalize();
    this.stmtRecent.finalize();
    this.stmtByKind.finalize();
  }
}
