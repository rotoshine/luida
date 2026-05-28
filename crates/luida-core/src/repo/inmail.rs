use anyhow::Result;
use rusqlite::{params, Connection, Row};

use crate::db::now_ms;
use crate::models::{Inmail, INMAIL_KINDS};

/// 새 inmail (payload는 직렬화된 JSON 문자열).
pub struct NewInmail<'a> {
    pub from_session: &'a str,
    pub to_session: &'a str,
    pub kind: &'a str,
    pub payload: &'a str,
    pub reply_to: Option<i64>,
    pub quest_id: Option<i64>,
    pub campaign_id: Option<i64>,
    pub dedupe_key: Option<&'a str>,
}

pub struct EnqueueResult {
    pub inserted: bool,
    pub id: Option<i64>,
}

pub struct InmailRepo<'a> {
    conn: &'a Connection,
}

impl<'a> InmailRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// 발행. dedupe_key 충돌 시 무시(INSERT OR IGNORE).
    /// broadcast(@) 주소에 dispatch는 거부 (다중 처리 사고 방지).
    pub fn enqueue(&self, m: NewInmail) -> Result<EnqueueResult> {
        // INSERT OR IGNORE는 UNIQUE(dedupe)뿐 아니라 CHECK 위반도 삼키므로
        // kind는 Rust 측에서 명시 검증 (DB CHECK에만 의존하지 않음).
        if !INMAIL_KINDS.contains(&m.kind) {
            anyhow::bail!("알 수 없는 inmail kind: {}", m.kind);
        }
        if m.to_session.starts_with('@') && m.kind == "dispatch" {
            anyhow::bail!("dispatch는 broadcast 주소({})에 보낼 수 없음", m.to_session);
        }
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO inmail
               (from_session, to_session, reply_to, quest_id, campaign_id, kind, payload, dedupe_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                m.from_session, m.to_session, m.reply_to, m.quest_id, m.campaign_id,
                m.kind, m.payload, m.dedupe_key, now_ms()
            ],
        )?;
        if changed > 0 {
            Ok(EnqueueResult {
                inserted: true,
                id: Some(self.conn.last_insert_rowid()),
            })
        } else {
            Ok(EnqueueResult {
                inserted: false,
                id: None,
            })
        }
    }

    /// me 앞으로 온 미배달 메시지 + broadcast(자기 발신 제외).
    pub fn pending_for(&self, me: &str) -> Result<Vec<Inmail>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_session, to_session, reply_to, quest_id, campaign_id,
                    kind, payload, dedupe_key, created_at, delivered_at, handled_at
             FROM inmail
             WHERE delivered_at IS NULL
               AND (to_session = ?1 OR (to_session LIKE '@%' AND from_session != ?1))
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![me], Self::map_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn mark_delivered(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE inmail SET delivered_at = ?1 WHERE id = ?2",
            params![now_ms(), id],
        )?;
        Ok(())
    }

    pub fn mark_handled(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE inmail SET handled_at = ?1 WHERE id = ?2",
            params![now_ms(), id],
        )?;
        Ok(())
    }

    pub fn tail(&self, limit: i64) -> Result<Vec<Inmail>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_session, to_session, reply_to, quest_id, campaign_id,
                    kind, payload, dedupe_key, created_at, delivered_at, handled_at
             FROM inmail ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], Self::map_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn map_row(r: &Row) -> rusqlite::Result<Inmail> {
        Ok(Inmail {
            id: r.get(0)?,
            from_session: r.get(1)?,
            to_session: r.get(2)?,
            reply_to: r.get(3)?,
            quest_id: r.get(4)?,
            campaign_id: r.get(5)?,
            kind: r.get(6)?,
            payload: r.get(7)?,
            dedupe_key: r.get(8)?,
            created_at: r.get(9)?,
            delivered_at: r.get(10)?,
            handled_at: r.get(11)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate, open_memory};

    fn setup() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn msg<'a>(from: &'a str, to: &'a str, kind: &'a str) -> NewInmail<'a> {
        NewInmail {
            from_session: from,
            to_session: to,
            kind,
            payload: "{}",
            reply_to: None,
            quest_id: None,
            campaign_id: None,
            dedupe_key: None,
        }
    }

    #[test]
    fn enqueue_and_tail() {
        let conn = setup();
        let repo = InmailRepo::new(&conn);
        let r = repo.enqueue(msg("luida", "agora", "info")).unwrap();
        assert!(r.inserted);
        assert_eq!(repo.tail(10).unwrap().len(), 1);
    }

    #[test]
    fn dedupe_per_recipient_sender() {
        let conn = setup();
        let repo = InmailRepo::new(&conn);
        let mut m1 = msg("luida", "agora", "info");
        m1.dedupe_key = Some("k1");
        assert!(repo.enqueue(m1).unwrap().inserted);

        let mut m2 = msg("luida", "agora", "info");
        m2.dedupe_key = Some("k1");
        assert!(!repo.enqueue(m2).unwrap().inserted); // 중복

        // 다른 발신자 같은 key → 허용
        let mut m3 = msg("brain", "agora", "info");
        m3.dedupe_key = Some("k1");
        assert!(repo.enqueue(m3).unwrap().inserted);
    }

    #[test]
    fn dispatch_to_broadcast_rejected() {
        let conn = setup();
        let repo = InmailRepo::new(&conn);
        assert!(repo.enqueue(msg("luida", "@all", "dispatch")).is_err());
        // 비-dispatch broadcast는 허용
        assert!(repo.enqueue(msg("luida", "@all", "alert")).unwrap().inserted);
    }

    #[test]
    fn invalid_kind_rejected() {
        let conn = setup();
        let repo = InmailRepo::new(&conn);
        assert!(repo.enqueue(msg("luida", "agora", "shout")).is_err());
    }

    #[test]
    fn pending_for_direct_and_broadcast() {
        let conn = setup();
        let repo = InmailRepo::new(&conn);
        repo.enqueue(msg("luida", "agora", "info")).unwrap();
        repo.enqueue(msg("luida", "@all", "alert")).unwrap();
        repo.enqueue(msg("agora", "@all", "info")).unwrap(); // 자기 broadcast

        let pending = repo.pending_for("agora").unwrap();
        // direct(agora) + luida broadcast = 2, 자기(agora) broadcast 제외
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|m| m.from_session != "agora"));
    }

    #[test]
    fn mark_delivered_removes_from_pending() {
        let conn = setup();
        let repo = InmailRepo::new(&conn);
        let r = repo.enqueue(msg("luida", "agora", "info")).unwrap();
        repo.mark_delivered(r.id.unwrap()).unwrap();
        assert_eq!(repo.pending_for("agora").unwrap().len(), 0);
    }
}
