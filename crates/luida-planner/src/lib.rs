//! luida-planner — 원정 계획·실행.
//!
//! - `plan`: plan_json 스키마 + 파싱 + 검증(DAG/사이클/프로젝트) + 위상정렬
//! - `planner`: `campaign.plan`(LLM)으로 DAG 생성 → campaigns/quests 영속 → 의존성 순 디스패치
//!
//! runtime은 factory 주입이라 실제 CLI 없이 테스트 가능하다.

mod plan;
mod planner;

pub use plan::{CampaignPlan, PlannedQuest};
pub use planner::{plan_campaign, run_campaign, CampaignRunReport};
