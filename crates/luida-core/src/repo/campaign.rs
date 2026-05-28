use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::now_ms;
use crate::models::Campaign;

/// 새 원정 생성 입력.
pub struct NewCampaign<'a> {
    pub title: &'a str,
    pub prompt: &'a str,
    pub plan_json: &'a str,
    pub status: &'a str,
}

pub struct CampaignRepo<'a> {
    conn: &'a Connection,
}

impl<'a> CampaignRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, c: NewCampaign) -> Result<i64> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO campaigns (title, prompt, plan_json, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![c.title, c.prompt, c.plan_json, c.status, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get(&self, id: i64) -> Result<Option<Campaign>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, title, prompt, plan_json, status, report_path, owner_machine,
                        handoff_state, created_at, updated_at, completed_at
                 FROM campaigns WHERE id = ?1",
                params![id],
                Self::map_row,
            )
            .optional()?)
    }

    pub fn list_active(&self) -> Result<Vec<Campaign>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, prompt, plan_json, status, report_path, owner_machine,
                    handoff_state, created_at, updated_at, completed_at
             FROM campaigns
             WHERE status NOT IN ('completed', 'failed', 'aborted')
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn set_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE campaigns SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now_ms(), id],
        )?;
        Ok(())
    }

    pub fn set_plan(&self, id: i64, plan_json: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE campaigns SET plan_json = ?1, updated_at = ?2 WHERE id = ?3",
            params![plan_json, now_ms(), id],
        )?;
        Ok(())
    }

    /// hand-off (suspend/resume) 상태 + owner 머신 갱신.
    pub fn set_handoff(&self, id: i64, state: &str, owner_machine: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE campaigns SET handoff_state = ?1, owner_machine = ?2, updated_at = ?3 WHERE id = ?4",
            params![state, owner_machine, now_ms(), id],
        )?;
        Ok(())
    }

    /// 완료 마감 없이 report 경로만 기록 (실패/부분 완료 원정의 사후 보고).
    pub fn set_report_path(&self, id: i64, report_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE campaigns SET report_path = ?1, updated_at = ?2 WHERE id = ?3",
            params![report_path, now_ms(), id],
        )?;
        Ok(())
    }

    pub fn mark_completed(&self, id: i64, report_path: Option<&str>) -> Result<()> {
        let now = now_ms();
        self.conn.execute(
            "UPDATE campaigns SET status = 'completed', report_path = ?1, updated_at = ?2, completed_at = ?2 WHERE id = ?3",
            params![report_path, now, id],
        )?;
        Ok(())
    }

    fn map_row(r: &Row) -> rusqlite::Result<Campaign> {
        Ok(Campaign {
            id: r.get(0)?,
            title: r.get(1)?,
            prompt: r.get(2)?,
            plan_json: r.get(3)?,
            status: r.get(4)?,
            report_path: r.get(5)?,
            owner_machine: r.get(6)?,
            handoff_state: r.get(7)?,
            created_at: r.get(8)?,
            updated_at: r.get(9)?,
            completed_at: r.get(10)?,
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

    fn new_campaign() -> NewCampaign<'static> {
        NewCampaign {
            title: "스키마 동기화 원정",
            prompt: "agora schema를 admin에 반영",
            plan_json: "{}",
            status: "planning",
        }
    }

    #[test]
    fn insert_and_get() {
        let conn = setup();
        let repo = CampaignRepo::new(&conn);
        let id = repo.insert(new_campaign()).unwrap();
        let c = repo.get(id).unwrap().unwrap();
        assert_eq!(c.title, "스키마 동기화 원정");
        assert_eq!(c.status, "planning");
        assert_eq!(c.handoff_state, "active");
    }

    #[test]
    fn status_transitions() {
        let conn = setup();
        let repo = CampaignRepo::new(&conn);
        let id = repo.insert(new_campaign()).unwrap();
        repo.set_status(id, "confirmed").unwrap();
        assert_eq!(repo.get(id).unwrap().unwrap().status, "confirmed");
        repo.set_status(id, "running").unwrap();
        assert_eq!(repo.get(id).unwrap().unwrap().status, "running");
    }

    #[test]
    fn invalid_status_rejected() {
        let conn = setup();
        let repo = CampaignRepo::new(&conn);
        let id = repo.insert(new_campaign()).unwrap();
        assert!(repo.set_status(id, "bogus").is_err());
    }

    #[test]
    fn list_active_excludes_terminal() {
        let conn = setup();
        let repo = CampaignRepo::new(&conn);
        let a = repo.insert(new_campaign()).unwrap();
        let b = repo.insert(new_campaign()).unwrap();
        repo.mark_completed(b, Some("/r.md")).unwrap();
        let active = repo.list_active().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, a);
    }

    #[test]
    fn handoff_state() {
        let conn = setup();
        let repo = CampaignRepo::new(&conn);
        let id = repo.insert(new_campaign()).unwrap();
        repo.set_handoff(id, "suspended", Some("home-mac")).unwrap();
        let c = repo.get(id).unwrap().unwrap();
        assert_eq!(c.handoff_state, "suspended");
        assert_eq!(c.owner_machine.as_deref(), Some("home-mac"));
        assert!(repo.set_handoff(id, "bogus", None).is_err());
    }

    #[test]
    fn set_report_path_without_completing() {
        let conn = setup();
        let repo = CampaignRepo::new(&conn);
        let id = repo.insert(new_campaign()).unwrap();
        repo.set_status(id, "running").unwrap();
        repo.set_report_path(id, "/r/postmortem.md").unwrap();
        let c = repo.get(id).unwrap().unwrap();
        assert_eq!(c.report_path.as_deref(), Some("/r/postmortem.md"));
        assert_eq!(c.status, "running"); // 완료 마감 안 함
        assert!(c.completed_at.is_none());
    }

    #[test]
    fn mark_completed_sets_fields() {
        let conn = setup();
        let repo = CampaignRepo::new(&conn);
        let id = repo.insert(new_campaign()).unwrap();
        repo.mark_completed(id, Some("/reports/0001.md")).unwrap();
        let c = repo.get(id).unwrap().unwrap();
        assert_eq!(c.status, "completed");
        assert_eq!(c.report_path.as_deref(), Some("/reports/0001.md"));
        assert!(c.completed_at.is_some());
    }
}
