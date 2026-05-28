-- Luida v2 core 스키마 — campaigns / quests / inmail / events / relationships.
-- project-centric (v1의 adventurers 개념 폐기). PRAGMA는 open_db에서.

-- =====================================================================
-- campaigns: 사용자 프롬프트 1개 → 다중 프로젝트 원정 계획 (DAG)
-- =====================================================================
CREATE TABLE IF NOT EXISTS campaigns (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  title          TEXT NOT NULL,
  prompt         TEXT NOT NULL,
  plan_json      TEXT NOT NULL DEFAULT '{}',
  status         TEXT NOT NULL CHECK (status IN (
                   'planning', 'confirmed', 'running',
                   'needs_input', 'completed', 'failed', 'aborted'
                 )),
  report_path    TEXT,
  owner_machine  TEXT,
  handoff_state  TEXT NOT NULL DEFAULT 'active' CHECK (handoff_state IN (
                   'active', 'suspended', 'resumed'
                 )),
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  completed_at   INTEGER
);

CREATE INDEX IF NOT EXISTS ix_campaigns_active
  ON campaigns(status, updated_at DESC)
  WHERE status NOT IN ('completed', 'failed', 'aborted');

-- =====================================================================
-- quests: 원정의 한 노드. 특정 모험지(project)에서 1개 worktree 작업
-- =====================================================================
CREATE TABLE IF NOT EXISTS quests (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  campaign_id         INTEGER REFERENCES campaigns(id) ON DELETE CASCADE,
  project             TEXT NOT NULL
                        REFERENCES projects(name)
                        ON UPDATE CASCADE ON DELETE RESTRICT,
  brief               TEXT NOT NULL,
  branch              TEXT,
  worktree_path       TEXT,
  status              TEXT NOT NULL CHECK (status IN (
                        'pending', 'running', 'reviewing', 'needs_input',
                        'needs_approval', 'pr_ready',
                        'completed', 'failed', 'aborted'
                      )),
  progress            TEXT,
  pr_url              TEXT,
  log_path            TEXT,
  depends_on_quest_id INTEGER REFERENCES quests(id) ON DELETE SET NULL,
  source_inmail_id    INTEGER,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  completed_at        INTEGER
);

CREATE INDEX IF NOT EXISTS ix_quests_active
  ON quests(status, updated_at DESC)
  WHERE status NOT IN ('completed', 'failed', 'aborted');

CREATE INDEX IF NOT EXISTS ix_quests_campaign ON quests(campaign_id);
CREATE INDEX IF NOT EXISTS ix_quests_project ON quests(project, status);

CREATE UNIQUE INDEX IF NOT EXISTS ux_quests_source
  ON quests(source_inmail_id)
  WHERE source_inmail_id IS NOT NULL;

-- =====================================================================
-- inmail: 일회성 메시지 (dispatch/ack/proposal/escalation/...)
--   from_session/to_session 자유 텍스트 ('luida', project명, 'luida-brain', '@all')
-- =====================================================================
CREATE TABLE IF NOT EXISTS inmail (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  from_session  TEXT NOT NULL,
  to_session    TEXT NOT NULL,
  reply_to      INTEGER REFERENCES inmail(id) ON DELETE SET NULL,
  quest_id      INTEGER REFERENCES quests(id) ON DELETE SET NULL,
  campaign_id   INTEGER REFERENCES campaigns(id) ON DELETE SET NULL,
  kind          TEXT NOT NULL CHECK (kind IN (
                  'dispatch', 'progress', 'ack',
                  'proposal', 'alert', 'info', 'escalation'
                )),
  payload       TEXT NOT NULL,
  dedupe_key    TEXT,
  created_at    INTEGER NOT NULL,
  delivered_at  INTEGER,
  handled_at    INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_inmail_dedupe
  ON inmail(to_session, from_session, dedupe_key)
  WHERE dedupe_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS ix_inmail_pending
  ON inmail(to_session, delivered_at)
  WHERE delivered_at IS NULL;

-- =====================================================================
-- events: 학습용 영속 로그
-- =====================================================================
CREATE TABLE IF NOT EXISTS events (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  campaign_id  INTEGER REFERENCES campaigns(id) ON DELETE SET NULL,
  quest_id     INTEGER REFERENCES quests(id) ON DELETE SET NULL,
  actor        TEXT NOT NULL,
  kind         TEXT NOT NULL,
  payload      TEXT NOT NULL,
  occurred_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS ix_events_recent ON events(occurred_at DESC);
CREATE INDEX IF NOT EXISTS ix_events_kind ON events(kind, occurred_at DESC);

-- =====================================================================
-- relationships: 프로젝트 간 자동화 룰 (사람 정의 + 학습 승격)
-- =====================================================================
CREATE TABLE IF NOT EXISTS relationships (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  name           TEXT UNIQUE,
  from_project   TEXT NOT NULL
                   REFERENCES projects(name)
                   ON UPDATE CASCADE ON DELETE RESTRICT,
  trigger_kind   TEXT NOT NULL CHECK (trigger_kind IN (
                   'path_changed', 'quest_completed', 'tag_pushed'
                 )),
  trigger_config TEXT NOT NULL,
  to_project     TEXT NOT NULL
                   REFERENCES projects(name)
                   ON UPDATE CASCADE ON DELETE RESTRICT,
  action         TEXT NOT NULL CHECK (action IN ('auto_dispatch', 'propose')),
  brief_template TEXT,
  enabled        INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  source         TEXT NOT NULL CHECK (source IN ('human', 'learned-promoted')),
  confidence     REAL,
  created_at     INTEGER NOT NULL
);
