-- 0004: Memory Tree (spec §6.1) — 계층 요약 트리.
-- leaf(level 0, 원자 청크) → 중간 요약 → 루트. reflect/plan이 상위 노드를 읽어
-- 토큰 효율적으로 장기 기억을 주입한다. 본문(요약)은 여기에, 원본은 vault .md(path).

CREATE TABLE IF NOT EXISTS memory_chunks (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  parent_id      INTEGER REFERENCES memory_chunks(id) ON DELETE CASCADE,
  level          INTEGER NOT NULL DEFAULT 0,    -- 0=leaf, 1+=요약
  score          REAL,
  token_estimate INTEGER NOT NULL DEFAULT 0,
  path           TEXT,                          -- leaf 원본 vault 경로(선택)
  summary        TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  CHECK (level >= 0)
);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_parent ON memory_chunks(parent_id);
CREATE INDEX IF NOT EXISTS idx_memory_chunks_level ON memory_chunks(level);
