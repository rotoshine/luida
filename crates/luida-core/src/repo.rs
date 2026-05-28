use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::now_ms;
use crate::models::Project;

/// projects 테이블 repository.
pub struct ProjectRepo<'a> {
    conn: &'a Connection,
}

impl<'a> ProjectRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// 등록(또는 upsert). 같은 name이면 경로·브랜치·설명을 갱신.
    pub fn add(
        &self,
        name: &str,
        repo_path: &str,
        base_branch: &str,
        description: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects (name, repo_path, base_branch, description, registered_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(name) DO UPDATE SET
               repo_path = excluded.repo_path,
               base_branch = excluded.base_branch,
               description = COALESCE(excluded.description, description)",
            params![name, repo_path, base_branch, description, now_ms()],
        )?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<Option<Project>> {
        let row = self
            .conn
            .query_row(
                "SELECT name, repo_path, base_branch, description, context_path, registered_at, last_ingested_at
                 FROM projects WHERE name = ?1",
                params![name],
                Self::map_row,
            )
            .optional()?;
        Ok(row)
    }

    pub fn list(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, repo_path, base_branch, description, context_path, registered_at, last_ingested_at
             FROM projects ORDER BY name",
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn remove(&self, name: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM projects WHERE name = ?1", params![name])?;
        Ok(n > 0)
    }

    fn map_row(r: &Row) -> rusqlite::Result<Project> {
        Ok(Project {
            name: r.get(0)?,
            repo_path: r.get(1)?,
            base_branch: r.get(2)?,
            description: r.get(3)?,
            context_path: r.get(4)?,
            registered_at: r.get(5)?,
            last_ingested_at: r.get(6)?,
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

    #[test]
    fn add_and_get() {
        let conn = setup();
        let repo = ProjectRepo::new(&conn);
        repo.add("agora", "/repos/agora", "main", Some("community web")).unwrap();
        let p = repo.get("agora").unwrap().unwrap();
        assert_eq!(p.name, "agora");
        assert_eq!(p.repo_path, "/repos/agora");
        assert_eq!(p.base_branch, "main");
        assert_eq!(p.description.as_deref(), Some("community web"));
    }

    #[test]
    fn add_is_upsert() {
        let conn = setup();
        let repo = ProjectRepo::new(&conn);
        repo.add("agora", "/old", "main", Some("orig")).unwrap();
        repo.add("agora", "/new", "develop", Some("updated")).unwrap();
        let p = repo.get("agora").unwrap().unwrap();
        assert_eq!(p.repo_path, "/new");
        assert_eq!(p.base_branch, "develop");
        assert_eq!(p.description.as_deref(), Some("updated"));
        assert_eq!(repo.list().unwrap().len(), 1);
    }

    #[test]
    fn upsert_without_desc_preserves_existing() {
        // m2: 재등록 시 desc 미지정(None)이면 기존 설명 보존 (COALESCE)
        let conn = setup();
        let repo = ProjectRepo::new(&conn);
        repo.add("agora", "/a", "main", Some("커뮤니티 웹")).unwrap();
        repo.add("agora", "/a", "main", None).unwrap();
        assert_eq!(repo.get("agora").unwrap().unwrap().description.as_deref(), Some("커뮤니티 웹"));
    }

    #[test]
    fn list_sorted() {
        let conn = setup();
        let repo = ProjectRepo::new(&conn);
        repo.add("c", "/c", "main", None).unwrap();
        repo.add("a", "/a", "main", None).unwrap();
        repo.add("b", "/b", "main", None).unwrap();
        let names: Vec<_> = repo.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn remove() {
        let conn = setup();
        let repo = ProjectRepo::new(&conn);
        repo.add("agora", "/a", "main", None).unwrap();
        assert!(repo.remove("agora").unwrap());
        assert!(!repo.remove("agora").unwrap());
        assert!(repo.get("agora").unwrap().is_none());
    }

    #[test]
    fn get_missing_is_none() {
        let conn = setup();
        let repo = ProjectRepo::new(&conn);
        assert!(repo.get("nope").unwrap().is_none());
    }
}
