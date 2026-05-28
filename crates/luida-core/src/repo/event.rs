use anyhow::Result;
use rusqlite::{params, Connection, Row};

use crate::db::now_ms;
use crate::models::Event;

pub struct NewEvent<'a> {
    pub campaign_id: Option<i64>,
    pub quest_id: Option<i64>,
    pub actor: &'a str,
    pub kind: &'a str,
    pub payload: &'a str,
}

pub struct EventRepo<'a> {
    conn: &'a Connection,
}

impl<'a> EventRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn record(&self, e: NewEvent) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO events (campaign_id, quest_id, actor, kind, payload, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![e.campaign_id, e.quest_id, e.actor, e.kind, e.payload, now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn recent_since(&self, since_ms: i64, limit: i64) -> Result<Vec<Event>> {
        self.query_many(
            "SELECT id, campaign_id, quest_id, actor, kind, payload, occurred_at
             FROM events WHERE occurred_at >= ?1 ORDER BY occurred_at DESC LIMIT ?2",
            params![since_ms, limit],
        )
    }

    pub fn by_kind(&self, kind: &str, limit: i64) -> Result<Vec<Event>> {
        self.query_many(
            "SELECT id, campaign_id, quest_id, actor, kind, payload, occurred_at
             FROM events WHERE kind = ?1 ORDER BY occurred_at DESC LIMIT ?2",
            params![kind, limit],
        )
    }

    fn query_many(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(p, Self::map_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn map_row(r: &Row) -> rusqlite::Result<Event> {
        Ok(Event {
            id: r.get(0)?,
            campaign_id: r.get(1)?,
            quest_id: r.get(2)?,
            actor: r.get(3)?,
            kind: r.get(4)?,
            payload: r.get(5)?,
            occurred_at: r.get(6)?,
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

    fn ev<'a>(actor: &'a str, kind: &'a str) -> NewEvent<'a> {
        NewEvent {
            campaign_id: None,
            quest_id: None,
            actor,
            kind,
            payload: "{}",
        }
    }

    #[test]
    fn record_and_recent() {
        let conn = setup();
        let repo = EventRepo::new(&conn);
        repo.record(ev("agora", "quest_dispatched")).unwrap();
        repo.record(ev("admin", "pr_created")).unwrap();
        let recent = repo.recent_since(0, 10).unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn by_kind_filters() {
        let conn = setup();
        let repo = EventRepo::new(&conn);
        repo.record(ev("a", "quest_dispatched")).unwrap();
        repo.record(ev("b", "quest_dispatched")).unwrap();
        repo.record(ev("c", "pr_created")).unwrap();
        assert_eq!(repo.by_kind("quest_dispatched", 10).unwrap().len(), 2);
        assert_eq!(repo.by_kind("pr_created", 10).unwrap().len(), 1);
    }

    #[test]
    fn recent_since_window() {
        let conn = setup();
        let repo = EventRepo::new(&conn);
        repo.record(ev("a", "x")).unwrap();
        // 미래 시점 기준이면 0건
        let future = now_ms() + 1_000_000;
        assert_eq!(repo.recent_since(future, 10).unwrap().len(), 0);
    }

    #[test]
    fn free_form_kind_allowed() {
        let conn = setup();
        let repo = EventRepo::new(&conn);
        // events.kind는 CHECK 없음 — 자유 형식
        assert!(repo.record(ev("a", "custom_kind_xyz")).is_ok());
    }
}
