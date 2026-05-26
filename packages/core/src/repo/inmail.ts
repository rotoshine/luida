import type { Database, Statement } from 'bun:sqlite';
import { type Inmail, type InmailKind, nowMs } from '../schema';

export type InmailEnqueue = {
  from_session: string;
  to_session: string;
  kind: InmailKind;
  payload: unknown; // JSON serializable
  reply_to?: number | null;
  quest_id?: number | null;
  dedupe_key?: string | null;
};

export type InmailEnqueueResult = {
  inserted: boolean;
  id: number | null;
};

export class InmailRepo {
  private readonly stmtEnqueue: Statement<{ id: number }, any[]>;
  private readonly stmtPending: Statement<Inmail, [string]>;
  private readonly stmtPendingBroadcast: Statement<Inmail, []>;
  private readonly stmtMarkDelivered: Statement<unknown, [number, number]>;
  private readonly stmtMarkHandled: Statement<unknown, [number, number]>;
  private readonly stmtTail: Statement<Inmail, [number]>;

  constructor(db: Database) {
    this.stmtEnqueue = db.prepare(`
      INSERT OR IGNORE INTO inmail
        (from_session, to_session, reply_to, quest_id, kind, payload, dedupe_key, created_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      RETURNING id
    `);
    this.stmtPending = db.prepare(`
      SELECT * FROM inmail
      WHERE to_session = ? AND delivered_at IS NULL
      ORDER BY id ASC
    `);
    this.stmtPendingBroadcast = db.prepare(`
      SELECT * FROM inmail
      WHERE to_session LIKE '@%' AND delivered_at IS NULL
      ORDER BY id ASC
    `);
    this.stmtMarkDelivered = db.prepare(
      'UPDATE inmail SET delivered_at = ? WHERE id = ?',
    );
    this.stmtMarkHandled = db.prepare(
      'UPDATE inmail SET handled_at = ? WHERE id = ?',
    );
    this.stmtTail = db.prepare(
      'SELECT * FROM inmail ORDER BY id DESC LIMIT ?',
    );
  }

  /**
   * inmail 발행. broadcast 주소(`@all`)에 dispatch kind는 거부한다
   * (여러 sidecar가 같은 quest를 중복 처리하는 사고 방지 — Phase 1 리뷰 C4).
   */
  enqueue(msg: InmailEnqueue): InmailEnqueueResult {
    if (msg.to_session.startsWith('@') && msg.kind === 'dispatch') {
      throw new Error(
        `dispatch kind를 broadcast 주소(${msg.to_session})에 보낼 수 없습니다`,
      );
    }
    const row = this.stmtEnqueue.get(
      msg.from_session,
      msg.to_session,
      msg.reply_to ?? null,
      msg.quest_id ?? null,
      msg.kind,
      JSON.stringify(msg.payload),
      msg.dedupe_key ?? null,
      nowMs(),
    );
    if (row) return { inserted: true, id: row.id };
    return { inserted: false, id: null };
  }

  pendingFor(me: string): Inmail[] {
    // me 자신 앞으로 온 것 + broadcast 둘 다 가져옴 (자기 broadcast는 제외)
    const direct = this.stmtPending.all(me);
    const broadcast = this.stmtPendingBroadcast
      .all()
      .filter((m) => m.from_session !== me);
    return [...direct, ...broadcast].sort((a, b) => a.id - b.id);
  }

  markDelivered(id: number): void {
    this.stmtMarkDelivered.run(nowMs(), id);
  }

  markHandled(id: number): void {
    this.stmtMarkHandled.run(nowMs(), id);
  }

  tail(limit = 50): Inmail[] {
    return this.stmtTail.all(limit);
  }

  close(): void {
    this.stmtEnqueue.finalize();
    this.stmtPending.finalize();
    this.stmtPendingBroadcast.finalize();
    this.stmtMarkDelivered.finalize();
    this.stmtMarkHandled.finalize();
    this.stmtTail.finalize();
  }
}
