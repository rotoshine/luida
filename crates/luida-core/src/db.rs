use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

/// 현재 시각 epoch ms.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `~/.luida/tavern.db` 기본 경로. `LUIDA_DB_PATH` env로 override.
pub fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("LUIDA_DB_PATH") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".luida").join("tavern.db")
}

/// tavern.db 연결을 열고 표준 PRAGMA를 적용한다.
pub fn open_db(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).ok();
        }
    }
    let conn = Connection::open(path)
        .with_context(|| format!("tavern.db 열기 실패: {path:?}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

/// 인메모리 연결 (테스트용).
pub fn open_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init.sql", include_str!("../migrations/0001_init.sql")),
    (
        "0002_v2_core.sql",
        include_str!("../migrations/0002_v2_core.sql"),
    ),
    (
        "0003_quest_deps.sql",
        include_str!("../migrations/0003_quest_deps.sql"),
    ),
    (
        "0004_memory_chunks.sql",
        include_str!("../migrations/0004_memory_chunks.sql"),
    ),
    (
        "0005_quest_runner.sql",
        include_str!("../migrations/0005_quest_runner.sql"),
    ),
];

/// 미적용 마이그레이션을 순서대로 적용. 적용된 이름 목록을 반환 (idempotent).
///
/// `&mut Connection`을 받아 `conn.transaction()`으로 borrow-checked 트랜잭션 사용
/// (unchecked_transaction 회피). 행 변환 에러는 `?`로 전파(조용히 삼키지 않음).
pub fn migrate(conn: &mut Connection) -> Result<Vec<String>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            id          TEXT PRIMARY KEY,
            applied_at  INTEGER NOT NULL
        )",
    )?;

    let applied: HashSet<String> = {
        let mut stmt = conn.prepare("SELECT id FROM schema_migrations")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut set = HashSet::new();
        for r in rows {
            set.insert(r?); // 변환 실패 시 전파 — idempotency 보장 (M3)
        }
        set
    };

    let mut fresh = Vec::new();
    for (name, sql) in MIGRATIONS {
        if applied.contains(*name) {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (id, applied_at) VALUES (?1, ?2)",
            params![name, now_ms()],
        )?;
        tx.commit()?;
        fresh.push((*name).to_string());
    }
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_creates_all_tables() {
        let mut conn = open_memory().unwrap();
        let applied = migrate(&mut conn).unwrap();
        assert_eq!(
            applied,
            vec![
                "0001_init.sql".to_string(),
                "0002_v2_core.sql".to_string(),
                "0003_quest_deps.sql".to_string(),
                "0004_memory_chunks.sql".to_string(),
                "0005_quest_runner.sql".to_string(),
            ]
        );

        for table in [
            "projects",
            "campaigns",
            "quests",
            "inmail",
            "events",
            "relationships",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "table {table} missing");
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut conn = open_memory().unwrap();
        let first = migrate(&mut conn).unwrap();
        let second = migrate(&mut conn).unwrap();
        assert_eq!(first.len(), 5);
        assert_eq!(second.len(), 0);
    }
}
