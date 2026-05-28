use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::now_ms;
use crate::models::Relationship;

pub struct NewRelationship<'a> {
    pub name: Option<&'a str>,
    pub from_project: &'a str,
    pub trigger_kind: &'a str,
    pub trigger_config: &'a str,
    pub to_project: &'a str,
    pub action: &'a str,
    pub brief_template: Option<&'a str>,
    pub enabled: bool,
    pub source: &'a str,
    pub confidence: Option<f64>,
}

pub struct RelationshipRepo<'a> {
    conn: &'a Connection,
}

impl<'a> RelationshipRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, r: NewRelationship) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO relationships
               (name, from_project, trigger_kind, trigger_config, to_project,
                action, brief_template, enabled, source, confidence, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                r.name, r.from_project, r.trigger_kind, r.trigger_config, r.to_project,
                r.action, r.brief_template, r.enabled as i64, r.source, r.confidence, now_ms()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// name 기준 upsert (yaml 재싱크 SOT). name 없으면 단순 insert.
    pub fn upsert_by_name(&self, r: NewRelationship) -> Result<(i64, bool)> {
        if let Some(name) = r.name {
            if let Some(existing) = self.find_by_name(name)? {
                self.conn.execute(
                    "UPDATE relationships SET
                       from_project = ?1, trigger_kind = ?2, trigger_config = ?3,
                       to_project = ?4, action = ?5, brief_template = ?6,
                       enabled = ?7, source = ?8, confidence = ?9
                     WHERE name = ?10",
                    params![
                        r.from_project, r.trigger_kind, r.trigger_config, r.to_project,
                        r.action, r.brief_template, r.enabled as i64, r.source,
                        r.confidence, name
                    ],
                )?;
                return Ok((existing.id, false));
            }
        }
        Ok((self.insert(r)?, true))
    }

    pub fn find_by_name(&self, name: &str) -> Result<Option<Relationship>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, from_project, trigger_kind, trigger_config, to_project,
                        action, brief_template, enabled, source, confidence, created_at
                 FROM relationships WHERE name = ?1",
                params![name],
                Self::map_row,
            )
            .optional()?)
    }

    pub fn list_enabled(&self) -> Result<Vec<Relationship>> {
        self.query_many(
            "SELECT id, name, from_project, trigger_kind, trigger_config, to_project,
                    action, brief_template, enabled, source, confidence, created_at
             FROM relationships WHERE enabled = 1 ORDER BY id",
            params![],
        )
    }

    /// 전체 관계 (활성·비활성 모두) — 관리 UI/CLI용.
    pub fn list_all(&self) -> Result<Vec<Relationship>> {
        self.query_many(
            "SELECT id, name, from_project, trigger_kind, trigger_config, to_project,
                    action, brief_template, enabled, source, confidence, created_at
             FROM relationships ORDER BY id",
            params![],
        )
    }

    pub fn list_by_from(&self, from_project: &str) -> Result<Vec<Relationship>> {
        self.query_many(
            "SELECT id, name, from_project, trigger_kind, trigger_config, to_project,
                    action, brief_template, enabled, source, confidence, created_at
             FROM relationships WHERE from_project = ?1 AND enabled = 1 ORDER BY id",
            params![from_project],
        )
    }

    pub fn set_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE relationships SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, id],
        )?;
        Ok(())
    }

    fn query_many(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Relationship>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(p, Self::map_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn map_row(r: &Row) -> rusqlite::Result<Relationship> {
        Ok(Relationship {
            id: r.get(0)?,
            name: r.get(1)?,
            from_project: r.get(2)?,
            trigger_kind: r.get(3)?,
            trigger_config: r.get(4)?,
            to_project: r.get(5)?,
            action: r.get(6)?,
            brief_template: r.get(7)?,
            enabled: r.get(8)?,
            source: r.get(9)?,
            confidence: r.get(10)?,
            created_at: r.get(11)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate, open_memory};
    use crate::repo::ProjectRepo;

    fn setup() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        let p = ProjectRepo::new(&conn);
        p.add("agora", "/a", "main", None).unwrap();
        p.add("admin", "/b", "main", None).unwrap();
        p.add("kontrol", "/c", "main", None).unwrap();
        conn
    }

    fn rel<'a>(name: &'a str, from: &'a str, to: &'a str, action: &'a str) -> NewRelationship<'a> {
        NewRelationship {
            name: Some(name),
            from_project: from,
            trigger_kind: "path_changed",
            trigger_config: r#"{"paths":["prisma/**"]}"#,
            to_project: to,
            action,
            brief_template: Some("{files} 반영"),
            enabled: true,
            source: "human",
            confidence: None,
        }
    }

    #[test]
    fn insert_and_find() {
        let conn = setup();
        let repo = RelationshipRepo::new(&conn);
        repo.insert(rel("r1", "agora", "admin", "auto_dispatch")).unwrap();
        let r = repo.find_by_name("r1").unwrap().unwrap();
        assert_eq!(r.from_project, "agora");
        assert_eq!(r.to_project, "admin");
        assert!(r.is_enabled());
    }

    #[test]
    fn fk_rejects_unknown_project() {
        let conn = setup();
        let repo = RelationshipRepo::new(&conn);
        assert!(repo.insert(rel("r", "ghost", "admin", "auto_dispatch")).is_err());
    }

    #[test]
    fn invalid_action_rejected() {
        let conn = setup();
        let repo = RelationshipRepo::new(&conn);
        assert!(repo.insert(rel("r", "agora", "admin", "send_troops")).is_err());
    }

    #[test]
    fn upsert_by_name() {
        let conn = setup();
        let repo = RelationshipRepo::new(&conn);
        let (_, created1) = repo.upsert_by_name(rel("dup", "agora", "admin", "auto_dispatch")).unwrap();
        assert!(created1);
        let (_, created2) = repo.upsert_by_name(rel("dup", "agora", "kontrol", "propose")).unwrap();
        assert!(!created2);
        let r = repo.find_by_name("dup").unwrap().unwrap();
        assert_eq!(r.to_project, "kontrol");
        assert_eq!(r.action, "propose");
    }

    #[test]
    fn list_enabled_and_toggle() {
        let conn = setup();
        let repo = RelationshipRepo::new(&conn);
        let id = repo.insert(rel("r1", "agora", "admin", "auto_dispatch")).unwrap();
        repo.insert(rel("r2", "agora", "kontrol", "propose")).unwrap();
        assert_eq!(repo.list_enabled().unwrap().len(), 2);
        repo.set_enabled(id, false).unwrap();
        assert_eq!(repo.list_enabled().unwrap().len(), 1);
        assert_eq!(repo.list_by_from("agora").unwrap().len(), 1);
    }

    #[test]
    fn list_all_includes_disabled() {
        let conn = setup();
        let repo = RelationshipRepo::new(&conn);
        let id = repo.insert(rel("r1", "agora", "admin", "auto_dispatch")).unwrap();
        repo.insert(rel("r2", "agora", "kontrol", "propose")).unwrap();
        repo.set_enabled(id, false).unwrap();
        assert_eq!(repo.list_enabled().unwrap().len(), 1);
        assert_eq!(repo.list_all().unwrap().len(), 2); // 비활성 포함
    }

    #[test]
    fn enabled_check_constraint() {
        let conn = setup();
        // enabled는 0/1만 — 직접 2 삽입 시도
        let r = conn.execute(
            "INSERT INTO relationships (from_project, trigger_kind, trigger_config, to_project, action, enabled, source, created_at)
             VALUES ('agora','path_changed','{}','admin','propose',2,'human',0)",
            [],
        );
        assert!(r.is_err());
    }
}
