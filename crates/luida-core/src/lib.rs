//! luida-core — tavern.db 스키마·연결·repository (v2 Rust).
//!
//! project-centric. campaigns/quests/inmail/events/relationships + agents resolver.

pub mod agents;
pub mod db;
pub mod handoff;
pub mod models;
pub mod repo;

pub use db::{default_db_path, migrate, now_ms, open_db, open_memory};
pub use rusqlite::Connection;
pub use models::{
    Campaign, Event, Inmail, MemoryChunk, Project, Quest, Relationship, EpochMs,
};
pub use repo::{
    CampaignRepo, EnqueueResult, EventRepo, InmailRepo, MemoryChunkRepo, NewCampaign, NewEvent,
    NewInmail, NewMemoryChunk, NewQuest, NewRelationship, ProjectRepo, QuestInsert, QuestRepo,
    RelationshipRepo,
};
pub use agents::{
    default_agents_path, resolve, runtime_available, AgentsConfig, ResolvedAgent,
};
pub use handoff::{
    machine_id, resume_bundle, suspend_campaign, HandoffBundle,
};

/// 데모 모드 여부 — `LUIDA_FAKE=1`(또는 true)이면 외부 LLM/repo 없이 결정적 fake 런타임 사용.
pub fn is_fake() -> bool {
    std::env::var("LUIDA_FAKE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// db 열고 마이그레이션 + agents.json 로드 (CLI·TUI 공용 부팅).
pub fn open_ready(db_path: &std::path::Path) -> anyhow::Result<(Connection, AgentsConfig)> {
    let mut conn = open_db(db_path)?;
    migrate(&mut conn)?;
    let cfg = AgentsConfig::load_or_default(&default_agents_path())?;
    Ok((conn, cfg))
}
