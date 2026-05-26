-- Luida tavern.db — initial schema (v0.1)
-- 5 core entities + schema_migrations.
-- PRAGMAs are NOT set here; openDb() applies them at connection time
-- (some PRAGMAs are silently ignored when run inside a transaction).

-- =====================================================================
-- adventurers: cmux pane에 붙은 Claude 세션 1개당 1행
--   PK: natural key (name). 이름 변경 시 자식 FK는 ON UPDATE CASCADE.
-- =====================================================================
CREATE TABLE IF NOT EXISTS adventurers (
  name           TEXT PRIMARY KEY,
  workspace_id   TEXT NOT NULL,
  surface_id     TEXT NOT NULL,
  repo_path      TEXT,
  role           TEXT NOT NULL CHECK (role IN ('main', 'worker', 'brain')),
  status         TEXT NOT NULL CHECK (status IN ('idle', 'busy', 'offline')),
  pid            INTEGER,
  last_seen      INTEGER NOT NULL,
  registered_at  INTEGER NOT NULL
);

-- =====================================================================
-- quests: 장기 작업 상태
--   FK: 모험가 삭제는 차단(RESTRICT) — soft-delete(status='offline')만 허용
--   parent_quest_id: 부모 삭제 시 child는 고아로 보존(SET NULL)
-- =====================================================================
CREATE TABLE IF NOT EXISTS quests (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  dispatched_by   TEXT NOT NULL
                    REFERENCES adventurers(name)
                    ON UPDATE CASCADE ON DELETE RESTRICT,
  dispatched_to   TEXT NOT NULL
                    REFERENCES adventurers(name)
                    ON UPDATE CASCADE ON DELETE RESTRICT,
  brief           TEXT NOT NULL,
  branch          TEXT,
  worktree_path   TEXT,
  status          TEXT NOT NULL CHECK (status IN (
                    'pending', 'running', 'reviewing',
                    'needs_approval', 'pr_ready',
                    'completed', 'failed', 'aborted'
                  )),
  progress        TEXT,
  pr_url          TEXT,
  log_path        TEXT,
  parent_quest_id INTEGER
                    REFERENCES quests(id)
                    ON UPDATE CASCADE ON DELETE SET NULL,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  completed_at    INTEGER
);

CREATE INDEX IF NOT EXISTS ix_quest_active
  ON quests(status, updated_at DESC)
  WHERE status NOT IN ('completed', 'failed', 'aborted');

CREATE INDEX IF NOT EXISTS ix_quest_to
  ON quests(dispatched_to, status);

-- =====================================================================
-- inmail: 일회성 메시지 (이벤트, 라우팅, 알림)
--   from_session/to_session 의도적으로 FK 없음.
--     이유: '@all', '@workers' 같은 broadcast 주소를 허용하기 위함.
--     ghost row 위험은 sidecar 송신 helper에서 검증으로 막는다 (Phase 1).
--   reply_to, quest_id: 참조 대상 사라지면 SET NULL (메시지 자체는 보존)
-- =====================================================================
CREATE TABLE IF NOT EXISTS inmail (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  from_session  TEXT NOT NULL,
  to_session    TEXT NOT NULL,
  reply_to      INTEGER REFERENCES inmail(id) ON DELETE SET NULL,
  quest_id      INTEGER REFERENCES quests(id) ON DELETE SET NULL,
  kind          TEXT NOT NULL CHECK (kind IN (
                  'dispatch', 'progress', 'ack',
                  'proposal', 'alert', 'info'
                )),
  payload       TEXT NOT NULL,
  dedupe_key    TEXT,
  created_at    INTEGER NOT NULL,
  delivered_at  INTEGER,
  handled_at    INTEGER
);

-- dedupe 정책: (recipient, sender, key) 단위로 unique.
--   같은 키여도 송신자가 다르면 별개 메시지로 간주.
--   같은 키여도 수신자가 다르면 별개 메시지로 간주.
--   dedupe_key가 NULL이면 unique 제약 자체가 없음 (자유로운 다중 발송).
CREATE UNIQUE INDEX IF NOT EXISTS ux_inmail_dedupe
  ON inmail(to_session, from_session, dedupe_key)
  WHERE dedupe_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS ix_inmail_pending
  ON inmail(to_session, delivered_at)
  WHERE delivered_at IS NULL;

-- =====================================================================
-- events: 학습용 영속 로그
--   actor: 의도적으로 FK 없음. 'system', '<deleted adventurer>' 같은 값 허용.
--   kind: 의도적으로 CHECK 없음. EventKind union(schema.ts)은 alias일 뿐,
--          DB는 free-form. isKnownEventKind() helper로 좁혀서 사용.
--   quest_id: 참조 quest 삭제되어도 이벤트는 보존 (SET NULL)
-- =====================================================================
CREATE TABLE IF NOT EXISTS events (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  quest_id    INTEGER REFERENCES quests(id) ON DELETE SET NULL,
  actor       TEXT NOT NULL,
  kind        TEXT NOT NULL,
  payload     TEXT NOT NULL,
  occurred_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS ix_events_recent
  ON events(occurred_at DESC);

CREATE INDEX IF NOT EXISTS ix_events_kind
  ON events(kind, occurred_at DESC);

-- =====================================================================
-- relationships: 사람 정의 + 학습 승격된 자동화 룰
--   from_session/to_session은 FK (실제 adventurer를 가리킴)
--   enabled는 0|1로 CHECK (boolean 의미)
-- =====================================================================
CREATE TABLE IF NOT EXISTS relationships (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  name           TEXT UNIQUE,
  from_session   TEXT NOT NULL
                   REFERENCES adventurers(name)
                   ON UPDATE CASCADE ON DELETE RESTRICT,
  trigger_kind   TEXT NOT NULL CHECK (trigger_kind IN (
                   'path_changed', 'quest_completed', 'tag_pushed'
                 )),
  trigger_config TEXT NOT NULL,
  to_session     TEXT NOT NULL
                   REFERENCES adventurers(name)
                   ON UPDATE CASCADE ON DELETE RESTRICT,
  action         TEXT NOT NULL CHECK (action IN ('auto_dispatch', 'propose')),
  brief_template TEXT,
  enabled        INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  source         TEXT NOT NULL CHECK (source IN ('human', 'learned-promoted')),
  confidence     REAL,
  created_at     INTEGER NOT NULL
);
