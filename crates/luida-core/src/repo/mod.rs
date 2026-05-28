//! Repository 계층 — 각 엔티티별 CRUD. 모두 `&Connection`을 빌려 사용.

mod campaign;
mod event;
mod inmail;
mod project;
mod quest;
mod relationship;

pub use campaign::{CampaignRepo, NewCampaign};
pub use event::{EventRepo, NewEvent};
pub use inmail::{EnqueueResult, InmailRepo, NewInmail};
pub use project::ProjectRepo;
pub use quest::{NewQuest, QuestInsert, QuestRepo};
pub use relationship::{NewRelationship, RelationshipRepo};
