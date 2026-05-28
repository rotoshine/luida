//! luida-core — tavern.db 스키마·연결·repository (v2 Rust).
//!
//! project-centric. campaigns/quests/inmail/events/relationships + agents resolver.

pub mod agents;
pub mod db;
pub mod models;
pub mod repo;

pub use db::{default_db_path, migrate, now_ms, open_db, open_memory};
pub use models::{
    Campaign, Event, Inmail, Project, Quest, Relationship, EpochMs,
};
pub use repo::{
    CampaignRepo, EnqueueResult, EventRepo, InmailRepo, NewCampaign, NewEvent,
    NewInmail, NewQuest, NewRelationship, ProjectRepo, QuestInsert, QuestRepo,
    RelationshipRepo,
};
pub use agents::{
    default_agents_path, resolve, runtime_available, AgentsConfig, ResolvedAgent,
};
