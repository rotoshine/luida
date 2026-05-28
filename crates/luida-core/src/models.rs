//! tavern.db row 타입 (v2). status/kind 등은 String + DB CHECK로 유효성 강제.
//! 알려진 값 상수는 각 모듈 상단에 둔다.

use serde::{Deserialize, Serialize};

/// epoch ms.
pub type EpochMs = i64;

/// 모험지(Project) — 등록된 repo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub repo_path: String,
    pub base_branch: String,
    pub description: Option<String>,
    pub context_path: Option<String>,
    pub registered_at: EpochMs,
    pub last_ingested_at: Option<EpochMs>,
}

/// 원정(Campaign) — 다중 프로젝트 계획.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Campaign {
    pub id: i64,
    pub title: String,
    pub prompt: String,
    pub plan_json: String,
    pub status: String,
    pub report_path: Option<String>,
    pub owner_machine: Option<String>,
    pub handoff_state: String,
    pub created_at: EpochMs,
    pub updated_at: EpochMs,
    pub completed_at: Option<EpochMs>,
}

pub const CAMPAIGN_STATUSES: &[&str] = &[
    "planning",
    "confirmed",
    "running",
    "needs_input",
    "completed",
    "failed",
    "aborted",
];
pub const HANDOFF_STATES: &[&str] = &["active", "suspended", "resumed"];

/// 모험(Quest) — 한 프로젝트의 작업 단위.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quest {
    pub id: i64,
    pub campaign_id: Option<i64>,
    pub project: String,
    pub brief: String,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub status: String,
    pub progress: Option<String>,
    pub pr_url: Option<String>,
    pub log_path: Option<String>,
    pub depends_on_quest_id: Option<i64>,
    pub source_inmail_id: Option<i64>,
    pub created_at: EpochMs,
    pub updated_at: EpochMs,
    pub completed_at: Option<EpochMs>,
}

pub const QUEST_STATUSES: &[&str] = &[
    "pending",
    "running",
    "reviewing",
    "needs_input",
    "needs_approval",
    "pr_ready",
    "completed",
    "failed",
    "aborted",
];
pub const QUEST_TERMINAL: &[&str] = &["completed", "failed", "aborted"];

/// inmail 메시지.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inmail {
    pub id: i64,
    pub from_session: String,
    pub to_session: String,
    pub reply_to: Option<i64>,
    pub quest_id: Option<i64>,
    pub campaign_id: Option<i64>,
    pub kind: String,
    pub payload: String,
    pub dedupe_key: Option<String>,
    pub created_at: EpochMs,
    pub delivered_at: Option<EpochMs>,
    pub handled_at: Option<EpochMs>,
}

pub const INMAIL_KINDS: &[&str] = &[
    "dispatch",
    "progress",
    "ack",
    "proposal",
    "alert",
    "info",
    "escalation",
];

/// 학습용 이벤트.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub campaign_id: Option<i64>,
    pub quest_id: Option<i64>,
    pub actor: String,
    pub kind: String,
    pub payload: String,
    pub occurred_at: EpochMs,
}

/// 자동화 룰.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    pub id: i64,
    pub name: Option<String>,
    pub from_project: String,
    pub trigger_kind: String,
    pub trigger_config: String,
    pub to_project: String,
    pub action: String,
    pub brief_template: Option<String>,
    pub enabled: i64,
    pub source: String,
    pub confidence: Option<f64>,
    pub created_at: EpochMs,
}

/// Memory Tree 노드 (계층 요약 트리). level 0=leaf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryChunk {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub level: i64,
    pub score: Option<f64>,
    pub token_estimate: i64,
    pub path: Option<String>,
    pub summary: String,
    pub created_at: EpochMs,
}

pub const RELATIONSHIP_TRIGGERS: &[&str] =
    &["path_changed", "quest_completed", "tag_pushed"];
pub const RELATIONSHIP_ACTIONS: &[&str] = &["auto_dispatch", "propose"];
pub const RELATIONSHIP_SOURCES: &[&str] = &["human", "learned-promoted"];

impl Relationship {
    pub fn is_enabled(&self) -> bool {
        self.enabled == 1
    }
}
