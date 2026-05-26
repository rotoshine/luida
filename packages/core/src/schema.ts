// Luida tavern.db row types. Mirrors migrations/0001_init.sql.
//
// 정책:
//   - 모든 *_at 컬럼은 epoch milliseconds (UTC). `Date.now()` 가정.
//   - SQLite enabled 컬럼은 0|1로 좁힘 (CHECK 제약과 일치).
//   - EventKind는 TS-side alias일 뿐, DB의 events.kind는 free-form (CHECK 없음).
//     알려진 kind인지 좁히려면 isKnownEventKind() 사용.

export type EpochMs = number;

export type AdventurerRole = 'main' | 'worker' | 'brain';
export type AdventurerStatus = 'idle' | 'busy' | 'offline';

export type Adventurer = {
  name: string;
  workspace_id: string;
  surface_id: string;
  repo_path: string | null;
  role: AdventurerRole;
  status: AdventurerStatus;
  pid: number | null;
  last_seen: EpochMs;
  registered_at: EpochMs;
};

export type QuestStatus =
  | 'pending'
  | 'running'
  | 'reviewing'
  | 'needs_approval'
  | 'pr_ready'
  | 'completed'
  | 'failed'
  | 'aborted';

export const QUEST_TERMINAL_STATUSES = ['completed', 'failed', 'aborted'] as const;

export type Quest = {
  id: number;
  dispatched_by: string;
  dispatched_to: string;
  brief: string;
  branch: string | null;
  worktree_path: string | null;
  status: QuestStatus;
  progress: string | null;
  pr_url: string | null;
  log_path: string | null;
  parent_quest_id: number | null;
  source_inmail_id: number | null;
  created_at: EpochMs;
  updated_at: EpochMs;
  completed_at: EpochMs | null;
};

export type InmailKind =
  | 'dispatch'
  | 'progress'
  | 'ack'
  | 'proposal'
  | 'alert'
  | 'info';

export type Inmail = {
  id: number;
  from_session: string;
  to_session: string; // '@all', '@workers' 같은 broadcast 주소 가능
  reply_to: number | null;
  quest_id: number | null;
  kind: InmailKind;
  payload: string; // JSON 문자열
  dedupe_key: string | null;
  created_at: EpochMs;
  delivered_at: EpochMs | null;
  handled_at: EpochMs | null;
};

export type EventKind =
  | 'quest_dispatched'
  | 'tool_used'
  | 'pr_created'
  | 'review_passed'
  | 'review_failed'
  | 'conflict'
  | 'user_approved'
  | 'user_rejected'
  | 'pattern_proposed';

const KNOWN_EVENT_KINDS: ReadonlySet<string> = new Set([
  'quest_dispatched',
  'tool_used',
  'pr_created',
  'review_passed',
  'review_failed',
  'conflict',
  'user_approved',
  'user_rejected',
  'pattern_proposed',
] satisfies EventKind[]);

export function isKnownEventKind(kind: string): kind is EventKind {
  return KNOWN_EVENT_KINDS.has(kind);
}

export type LuidaEvent = {
  id: number;
  quest_id: number | null;
  actor: string;
  kind: string; // free-form; isKnownEventKind()로 좁힘
  payload: string;
  occurred_at: EpochMs;
};

export type RelationshipAction = 'auto_dispatch' | 'propose';
export type RelationshipTriggerKind =
  | 'path_changed'
  | 'quest_completed'
  | 'tag_pushed';
export type RelationshipSource = 'human' | 'learned-promoted';

export type Relationship = {
  id: number;
  name: string | null;
  from_session: string;
  trigger_kind: RelationshipTriggerKind;
  trigger_config: string; // JSON 문자열
  to_session: string;
  action: RelationshipAction;
  brief_template: string | null;
  enabled: 0 | 1;
  source: RelationshipSource;
  confidence: number | null;
  created_at: EpochMs;
};

export function isEnabled(rel: Pick<Relationship, 'enabled'>): boolean {
  return rel.enabled === 1;
}

/** epoch ms → ISO-8601 문자열 */
export function toIso(ms: EpochMs): string {
  return new Date(ms).toISOString();
}

/** 현재 시각을 epoch ms로 반환. Date.now()의 의미 alias. */
export function nowMs(): EpochMs {
  return Date.now();
}
