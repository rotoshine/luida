import type { Database, Statement } from 'bun:sqlite';
import { type Quest, type QuestStatus, nowMs } from '../schema';

export type QuestInsert = {
  dispatched_by: string;
  dispatched_to: string;
  brief: string;
  branch?: string | null;
  worktree_path?: string | null;
  status?: QuestStatus;
  parent_quest_id?: number | null;
  source_inmail_id?: number | null;
};

export type QuestInsertResult = {
  id: number;
  /** false면 source_inmail_id 충돌로 기존 quest를 반환 (멱등) */
  created: boolean;
};

export class QuestRepo {
  private readonly stmtInsert: Statement<{ id: number }, any[]>;
  private readonly stmtGet: Statement<Quest, [number]>;
  private readonly stmtListActive: Statement<Quest, []>;
  private readonly stmtListFor: Statement<Quest, [string]>;
  private readonly stmtFindBySource: Statement<Quest, [number]>;
  private readonly stmtUpdateStatus: Statement<
    unknown,
    [QuestStatus, number, number]
  >;
  private readonly stmtUpdateProgress: Statement<
    unknown,
    [string | null, number, number]
  >;
  private readonly stmtUpdatePr: Statement<
    unknown,
    [string | null, QuestStatus, number, number, number]
  >;
  private readonly stmtUpdateWorktree: Statement<
    unknown,
    [string | null, string | null, number, number]
  >;

  constructor(db: Database) {
    this.stmtInsert = db.prepare(`
      INSERT INTO quests
        (dispatched_by, dispatched_to, brief, branch, worktree_path,
         status, parent_quest_id, source_inmail_id, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      RETURNING id
    `);
    this.stmtGet = db.prepare('SELECT * FROM quests WHERE id = ?');
    this.stmtFindBySource = db.prepare(
      'SELECT * FROM quests WHERE source_inmail_id = ?',
    );
    this.stmtListActive = db.prepare(`
      SELECT * FROM quests
      WHERE status NOT IN ('completed', 'failed', 'aborted')
      ORDER BY updated_at DESC
    `);
    this.stmtListFor = db.prepare(`
      SELECT * FROM quests
      WHERE dispatched_to = ?
      ORDER BY id DESC
    `);
    this.stmtUpdateStatus = db.prepare(
      'UPDATE quests SET status = ?, updated_at = ? WHERE id = ?',
    );
    this.stmtUpdateProgress = db.prepare(
      'UPDATE quests SET progress = ?, updated_at = ? WHERE id = ?',
    );
    this.stmtUpdatePr = db.prepare(`
      UPDATE quests
      SET pr_url = ?, status = ?, updated_at = ?, completed_at = ?
      WHERE id = ?
    `);
    this.stmtUpdateWorktree = db.prepare(
      'UPDATE quests SET branch = ?, worktree_path = ?, updated_at = ? WHERE id = ?',
    );
  }

  insert(input: QuestInsert): number {
    const now = nowMs();
    const row = this.stmtInsert.get(
      input.dispatched_by,
      input.dispatched_to,
      input.brief,
      input.branch ?? null,
      input.worktree_path ?? null,
      input.status ?? 'pending',
      input.parent_quest_id ?? null,
      input.source_inmail_id ?? null,
      now,
      now,
    );
    if (!row) throw new Error('quest insert did not return id');
    return row.id;
  }

  /**
   * source_inmail_id 기반 멱등 insert.
   * 이미 존재하면 기존 quest를 반환 (Phase 3: dispatch 중복 차단).
   */
  insertIdempotent(input: QuestInsert): QuestInsertResult {
    if (input.source_inmail_id != null) {
      const existing = this.findBySource(input.source_inmail_id);
      if (existing) return { id: existing.id, created: false };
    }
    try {
      return { id: this.insert(input), created: true };
    } catch (err) {
      // race: 다른 프로세스가 그 사이 insert했을 수 있음
      if (input.source_inmail_id != null) {
        const existing = this.findBySource(input.source_inmail_id);
        if (existing) return { id: existing.id, created: false };
      }
      throw err;
    }
  }

  get(id: number): Quest | null {
    return this.stmtGet.get(id) ?? null;
  }

  findBySource(inmailId: number): Quest | null {
    return this.stmtFindBySource.get(inmailId) ?? null;
  }

  listActive(): Quest[] {
    return this.stmtListActive.all();
  }

  listFor(adventurer: string): Quest[] {
    return this.stmtListFor.all(adventurer);
  }

  updateStatus(id: number, status: QuestStatus): void {
    this.stmtUpdateStatus.run(status, nowMs(), id);
  }

  updateProgress(id: number, progress: string | null): void {
    this.stmtUpdateProgress.run(progress, nowMs(), id);
  }

  updateWorktree(id: number, branch: string | null, path: string | null): void {
    this.stmtUpdateWorktree.run(branch, path, nowMs(), id);
  }

  markCompleted(id: number, prUrl: string | null): void {
    const now = nowMs();
    this.stmtUpdatePr.run(prUrl, 'completed', now, now, id);
  }

  close(): void {
    this.stmtInsert.finalize();
    this.stmtGet.finalize();
    this.stmtFindBySource.finalize();
    this.stmtListActive.finalize();
    this.stmtListFor.finalize();
    this.stmtUpdateStatus.finalize();
    this.stmtUpdateProgress.finalize();
    this.stmtUpdatePr.finalize();
    this.stmtUpdateWorktree.finalize();
  }
}
