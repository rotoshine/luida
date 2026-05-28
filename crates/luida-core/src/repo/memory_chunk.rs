use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::now_ms;
use crate::models::MemoryChunk;

/// 새 memory chunk 입력.
pub struct NewMemoryChunk<'a> {
    pub parent_id: Option<i64>,
    pub level: i64,
    pub score: Option<f64>,
    pub token_estimate: i64,
    pub path: Option<&'a str>,
    pub summary: &'a str,
}

pub struct MemoryChunkRepo<'a> {
    conn: &'a Connection,
}

impl<'a> MemoryChunkRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, c: NewMemoryChunk) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO memory_chunks (parent_id, level, score, token_estimate, path, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![c.parent_id, c.level, c.score, c.token_estimate, c.path, c.summary, now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get(&self, id: i64) -> Result<Option<MemoryChunk>> {
        Ok(self
            .conn
            .query_row(SELECT_ONE, params![id], Self::map_row)
            .optional()?)
    }

    /// parent_id를 갱신 (트리 연결).
    pub fn set_parent(&self, id: i64, parent_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE memory_chunks SET parent_id = ?1 WHERE id = ?2",
            params![parent_id, id],
        )?;
        Ok(())
    }

    /// 직접 자식들.
    pub fn children(&self, parent_id: i64) -> Result<Vec<MemoryChunk>> {
        self.query_many(
            "SELECT id, parent_id, level, score, token_estimate, path, summary, created_at
             FROM memory_chunks WHERE parent_id = ?1 ORDER BY id",
            params![parent_id],
        )
    }

    /// 특정 레벨 노드.
    pub fn by_level(&self, level: i64) -> Result<Vec<MemoryChunk>> {
        self.query_many(
            "SELECT id, parent_id, level, score, token_estimate, path, summary, created_at
             FROM memory_chunks WHERE level = ?1 ORDER BY id",
            params![level],
        )
    }

    /// 루트(부모 없음) 노드 — 최상위 요약. reflect/plan이 여기서 시작.
    pub fn roots(&self) -> Result<Vec<MemoryChunk>> {
        self.query_many(
            "SELECT id, parent_id, level, score, token_estimate, path, summary, created_at
             FROM memory_chunks WHERE parent_id IS NULL ORDER BY level DESC, id",
            params![],
        )
    }

    fn query_many(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<MemoryChunk>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(p, Self::map_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn map_row(r: &Row) -> rusqlite::Result<MemoryChunk> {
        Ok(MemoryChunk {
            id: r.get(0)?,
            parent_id: r.get(1)?,
            level: r.get(2)?,
            score: r.get(3)?,
            token_estimate: r.get(4)?,
            path: r.get(5)?,
            summary: r.get(6)?,
            created_at: r.get(7)?,
        })
    }
}

const SELECT_ONE: &str =
    "SELECT id, parent_id, level, score, token_estimate, path, summary, created_at
     FROM memory_chunks WHERE id = ?1";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate, open_memory};

    fn setup() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn leaf<'a>(summary: &'a str) -> NewMemoryChunk<'a> {
        NewMemoryChunk {
            parent_id: None,
            level: 0,
            score: None,
            token_estimate: summary.len() as i64,
            path: None,
            summary,
        }
    }

    #[test]
    fn insert_get_and_parent() {
        let conn = setup();
        let repo = MemoryChunkRepo::new(&conn);
        let a = repo.insert(leaf("청크 A")).unwrap();
        let b = repo.insert(leaf("청크 B")).unwrap();
        let parent = repo
            .insert(NewMemoryChunk {
                parent_id: None,
                level: 1,
                score: Some(0.9),
                token_estimate: 5,
                path: None,
                summary: "요약",
            })
            .unwrap();
        repo.set_parent(a, parent).unwrap();
        repo.set_parent(b, parent).unwrap();

        assert_eq!(repo.get(a).unwrap().unwrap().parent_id, Some(parent));
        assert_eq!(repo.children(parent).unwrap().len(), 2);
        assert_eq!(repo.by_level(0).unwrap().len(), 2);
        assert_eq!(repo.by_level(1).unwrap().len(), 1);
        // 루트 = parent (자식 leaf는 parent 있음)
        let roots = repo.roots().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, parent);
    }

    #[test]
    fn invalid_level_rejected() {
        let conn = setup();
        let repo = MemoryChunkRepo::new(&conn);
        let r = repo.insert(NewMemoryChunk {
            parent_id: None,
            level: -1,
            score: None,
            token_estimate: 0,
            path: None,
            summary: "x",
        });
        assert!(r.is_err());
    }

    #[test]
    fn cascade_delete_children() {
        let conn = setup();
        let repo = MemoryChunkRepo::new(&conn);
        let parent = repo.insert(leaf("p")).unwrap();
        let child = repo.insert(leaf("c")).unwrap();
        repo.set_parent(child, parent).unwrap();
        conn.execute("DELETE FROM memory_chunks WHERE id = ?1", params![parent]).unwrap();
        // ON DELETE CASCADE로 자식도 삭제
        assert!(repo.get(child).unwrap().is_none());
    }
}
