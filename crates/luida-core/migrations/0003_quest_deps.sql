-- 0003: quest 다중 의존성 (DAG). 0002의 quests.depends_on_quest_id는 단일 의존만
-- 표현 가능 → 다이아몬드 등 다중 의존을 위해 조인 테이블 도입.
-- depends_on_quest_id는 "대표 의존(back-compat)"으로 유지하되, ready 판정은 quest_deps도 본다.

CREATE TABLE IF NOT EXISTS quest_deps (
  quest_id            INTEGER NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
  depends_on_quest_id INTEGER NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
  PRIMARY KEY (quest_id, depends_on_quest_id),
  CHECK (quest_id <> depends_on_quest_id)
);

CREATE INDEX IF NOT EXISTS idx_quest_deps_quest ON quest_deps(quest_id);
