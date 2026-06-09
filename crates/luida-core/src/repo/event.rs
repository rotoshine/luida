use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};

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

    /// 특정 quest의 가장 최근 `kind` 이벤트 payload (없으면 None).
    pub fn latest_payload_for_quest(&self, quest_id: i64, kind: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT payload FROM events
                 WHERE quest_id = ?1 AND kind = ?2
                 ORDER BY occurred_at DESC, id DESC LIMIT 1",
                params![quest_id, kind],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn by_kind(&self, kind: &str, limit: i64) -> Result<Vec<Event>> {
        self.query_many(
            "SELECT id, campaign_id, quest_id, actor, kind, payload, occurred_at
             FROM events WHERE kind = ?1 ORDER BY occurred_at DESC LIMIT ?2",
            params![kind, limit],
        )
    }

    /// campaign의 events 타임라인 — 최신 `limit`건을 오래된 순으로. TUI 상세 뷰용.
    /// LIMIT 은 반드시 최신 쪽(DESC)에 걸어야 200건 초과 장시간 원정에서도 tail-follow 가
    /// 최신 이벤트를 따라간다. 안쪽에서 최신 N건을 추린 뒤 바깥에서 ASC 로 되돌려 표시 순서 유지.
    pub fn for_campaign(&self, campaign_id: i64, limit: i64) -> Result<Vec<Event>> {
        self.query_many(
            "SELECT id, campaign_id, quest_id, actor, kind, payload, occurred_at FROM (
                 SELECT id, campaign_id, quest_id, actor, kind, payload, occurred_at
                 FROM events WHERE campaign_id = ?1 ORDER BY occurred_at DESC, id DESC LIMIT ?2
             ) ORDER BY occurred_at ASC, id ASC",
            params![campaign_id, limit],
        )
    }

    /// quest의 events 타임라인 — 최신 `limit`건을 오래된 순으로. TUI 상세 뷰용.
    /// (for_campaign 과 동일하게 LIMIT 을 최신 쪽에 걸어 tail 정확성을 보장.)
    pub fn for_quest(&self, quest_id: i64, limit: i64) -> Result<Vec<Event>> {
        self.query_many(
            "SELECT id, campaign_id, quest_id, actor, kind, payload, occurred_at FROM (
                 SELECT id, campaign_id, quest_id, actor, kind, payload, occurred_at
                 FROM events WHERE quest_id = ?1 ORDER BY occurred_at DESC, id DESC LIMIT ?2
             ) ORDER BY occurred_at ASC, id ASC",
            params![quest_id, limit],
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
    fn for_campaign_and_quest_filter_in_timeline_order() {
        use crate::repo::{CampaignRepo, NewCampaign, NewQuest, ProjectRepo, QuestRepo};
        let conn = setup();
        // events.campaign_id/quest_id 는 FK라 실제 행이 있어야 한다.
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
            .unwrap();
        let qid = QuestRepo::new(&conn)
            .insert(NewQuest {
                campaign_id: Some(cid),
                project: "agora",
                brief: "b",
                branch: None,
                status: "running",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        let cid2 = CampaignRepo::new(&conn)
            .insert(NewCampaign { title: "t2", prompt: "p", plan_json: "{}", status: "running" })
            .unwrap();

        let repo = EventRepo::new(&conn);
        let mk = |campaign_id, quest_id, kind| NewEvent {
            campaign_id,
            quest_id,
            actor: "x",
            kind,
            payload: "{}",
        };
        repo.record(mk(Some(cid), None, "campaign_planned")).unwrap();
        repo.record(mk(Some(cid), Some(qid), "quest_dispatched")).unwrap();
        repo.record(mk(Some(cid), Some(qid), "quest_completed")).unwrap();
        repo.record(mk(Some(cid2), None, "campaign_planned")).unwrap();

        let camp = repo.for_campaign(cid, 100).unwrap();
        assert_eq!(camp.len(), 3);
        assert_eq!(camp[0].kind, "campaign_planned"); // ASC 타임라인 순(가장 먼저 기록된 것)
        assert!(camp.iter().all(|e| e.campaign_id == Some(cid)));

        let q = repo.for_quest(qid, 100).unwrap();
        assert_eq!(q.len(), 2);
        assert!(q.iter().all(|e| e.quest_id == Some(qid)));
    }

    #[test]
    fn for_campaign_returns_latest_window_in_ascending_order() {
        // LIMIT 초과(250건 > 200) 시 '최초'가 아닌 '최신' 200건을 오래된 순으로 돌려줘야
        // TUI tail-follow 가 장시간 원정에서도 최신 이벤트를 따라간다(회귀 방지).
        use crate::repo::{CampaignRepo, NewCampaign, ProjectRepo};
        let conn = setup();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
            .unwrap();
        let repo = EventRepo::new(&conn);
        for i in 0..250 {
            let payload = format!("{{\"n\":{i}}}");
            repo.record(NewEvent {
                campaign_id: Some(cid),
                quest_id: None,
                actor: "x",
                kind: "tick",
                payload: &payload,
            })
            .unwrap();
        }
        let evs = repo.for_campaign(cid, 200).unwrap();
        assert_eq!(evs.len(), 200, "limit 만큼만");
        // 윈도우는 최신 200건(n=50..=249), 표시는 오래된 순.
        assert!(evs[0].payload.contains("\"n\":50}"), "첫 원소=윈도우 시작(n=50): {}", evs[0].payload);
        assert!(evs[199].payload.contains("\"n\":249}"), "마지막 원소=최신(n=249): {}", evs[199].payload);
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
    fn latest_payload_for_quest_picks_most_recent() {
        use crate::repo::{NewQuest, ProjectRepo, QuestRepo};
        let conn = setup();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let qid = QuestRepo::new(&conn)
            .insert(NewQuest {
                campaign_id: None,
                project: "agora",
                brief: "x",
                branch: None,
                status: "pending",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        let repo = EventRepo::new(&conn);
        repo.record(NewEvent {
            campaign_id: None,
            quest_id: Some(qid),
            actor: "a",
            kind: "quest_needs_input",
            payload: r#"{"category":"old"}"#,
        })
        .unwrap();
        repo.record(NewEvent {
            campaign_id: None,
            quest_id: Some(qid),
            actor: "a",
            kind: "quest_needs_input",
            payload: r#"{"category":"new"}"#,
        })
        .unwrap();
        let p = repo
            .latest_payload_for_quest(qid, "quest_needs_input")
            .unwrap()
            .unwrap();
        assert!(p.contains("new"));
        assert!(repo.latest_payload_for_quest(qid, "nope").unwrap().is_none());
    }

    #[test]
    fn free_form_kind_allowed() {
        let conn = setup();
        let repo = EventRepo::new(&conn);
        // events.kind는 CHECK 없음 — 자유 형식
        assert!(repo.record(ev("a", "custom_kind_xyz")).is_ok());
    }
}
