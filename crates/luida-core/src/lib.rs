//! luida-core — tavern.db 스키마·연결·repository (v2 Rust).
//!
//! v2-P0: projects(모험지)만. 후속 Phase에서 quests/campaigns/inmail/events 추가.

pub mod db;
pub mod models;
pub mod repo;

pub use db::{default_db_path, migrate, now_ms, open_db};
pub use models::Project;
pub use repo::ProjectRepo;
