import type { Database, Statement } from 'bun:sqlite';
import {
  type Relationship,
  type RelationshipAction,
  type RelationshipSource,
  type RelationshipTriggerKind,
  nowMs,
} from '../schema';

export type RelationshipInsert = {
  name?: string | null;
  from_session: string;
  trigger_kind: RelationshipTriggerKind;
  trigger_config: unknown; // JSON-serializable
  to_session: string;
  action: RelationshipAction;
  brief_template?: string | null;
  enabled?: 0 | 1;
  source: RelationshipSource;
  confidence?: number | null;
};

export type RelationshipUpsertResult = {
  id: number;
  created: boolean;
};

export class RelationshipRepo {
  private readonly stmtInsert: Statement<{ id: number }, any[]>;
  private readonly stmtListEnabled: Statement<Relationship, []>;
  private readonly stmtListByFrom: Statement<Relationship, [string]>;
  private readonly stmtFindByName: Statement<Relationship, [string]>;
  private readonly stmtUpdateByName: Statement<unknown, any[]>;
  private readonly stmtToggle: Statement<unknown, [0 | 1, number]>;

  constructor(db: Database) {
    this.stmtInsert = db.prepare(`
      INSERT INTO relationships
        (name, from_session, trigger_kind, trigger_config, to_session,
         action, brief_template, enabled, source, confidence, created_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      RETURNING id
    `);
    this.stmtListEnabled = db.prepare(
      'SELECT * FROM relationships WHERE enabled = 1 ORDER BY id',
    );
    this.stmtListByFrom = db.prepare(
      'SELECT * FROM relationships WHERE from_session = ? AND enabled = 1 ORDER BY id',
    );
    this.stmtFindByName = db.prepare(
      'SELECT * FROM relationships WHERE name = ?',
    );
    this.stmtUpdateByName = db.prepare(`
      UPDATE relationships
      SET from_session = ?, trigger_kind = ?, trigger_config = ?,
          to_session = ?, action = ?, brief_template = ?,
          enabled = ?, source = ?, confidence = ?
      WHERE name = ?
    `);
    this.stmtToggle = db.prepare(
      'UPDATE relationships SET enabled = ? WHERE id = ?',
    );
  }

  insert(input: RelationshipInsert): number {
    const row = this.stmtInsert.get(
      input.name ?? null,
      input.from_session,
      input.trigger_kind,
      JSON.stringify(input.trigger_config ?? {}),
      input.to_session,
      input.action,
      input.brief_template ?? null,
      input.enabled ?? 1,
      input.source,
      input.confidence ?? null,
      nowMs(),
    );
    if (!row) throw new Error('relationship insert did not return id');
    return row.id;
  }

  /**
   * name이 있으면 upsert(name 기준), 없으면 단순 insert.
   * Phase 3: yaml 재싱크 시 룰 수정이 DB에 반영되도록.
   */
  upsertByName(input: RelationshipInsert): RelationshipUpsertResult {
    if (!input.name) {
      return { id: this.insert(input), created: true };
    }
    const existing = this.findByName(input.name);
    if (existing) {
      this.stmtUpdateByName.run(
        input.from_session,
        input.trigger_kind,
        JSON.stringify(input.trigger_config ?? {}),
        input.to_session,
        input.action,
        input.brief_template ?? null,
        input.enabled ?? 1,
        input.source,
        input.confidence ?? null,
        input.name,
      );
      return { id: existing.id, created: false };
    }
    return { id: this.insert(input), created: true };
  }

  findByName(name: string): Relationship | null {
    return this.stmtFindByName.get(name) ?? null;
  }

  listEnabled(): Relationship[] {
    return this.stmtListEnabled.all();
  }

  listByFrom(adventurer: string): Relationship[] {
    return this.stmtListByFrom.all(adventurer);
  }

  setEnabled(id: number, enabled: boolean): void {
    this.stmtToggle.run(enabled ? 1 : 0, id);
  }

  close(): void {
    this.stmtInsert.finalize();
    this.stmtListEnabled.finalize();
    this.stmtListByFrom.finalize();
    this.stmtFindByName.finalize();
    this.stmtUpdateByName.finalize();
    this.stmtToggle.finalize();
  }
}
