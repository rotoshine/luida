-- Luida v2 tavern.db — initial schema (Rust).
-- v2-P0 범위: projects(모험지). quests/campaigns/inmail/events는 후속 Phase 마이그레이션.
-- 주의: PRAGMA는 여기 두지 않음 (open_db에서 connection-time 적용).

CREATE TABLE IF NOT EXISTS projects (
  name             TEXT PRIMARY KEY,
  repo_path        TEXT NOT NULL,
  base_branch      TEXT NOT NULL DEFAULT 'main',
  description      TEXT,
  context_path     TEXT,
  registered_at    INTEGER NOT NULL,
  last_ingested_at INTEGER
);

CREATE INDEX IF NOT EXISTS ix_projects_registered
  ON projects(registered_at DESC);
