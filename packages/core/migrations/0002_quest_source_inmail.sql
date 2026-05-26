-- Phase 3: dispatch 멱등성 컬럼 추가.
-- 같은 inmail.id에 대해 quest는 최대 1건만 생성되도록 UNIQUE 보장.
-- sidecar crash 시 같은 inmail이 두 sidecar 또는 같은 sidecar의 두 번째 polling에서
-- 중복 처리되어도 quest는 한 번만 만들어진다.

ALTER TABLE quests ADD COLUMN source_inmail_id INTEGER
  REFERENCES inmail(id) ON UPDATE CASCADE ON DELETE SET NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ux_quests_source
  ON quests(source_inmail_id)
  WHERE source_inmail_id IS NOT NULL;
