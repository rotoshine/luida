//! luida-sidecar — quest 실행 오케스트레이션.
//!
//! - `worktree`: worker용 격리 작업공간 provisioning (worktrunk 기반)
//! - `dispatch`: resolve → worktree → worker 실행 → events/status/escalation 배선 + resume
//! - `escalation`: escalation triage(분류) + 사용자 알림 게이트
//!
//! runtime은 factory 주입이라 실제 CLI 없이 테스트 가능하다.

mod dispatch;
mod escalation;
mod worktree;

pub use dispatch::{
    dispatch_quest, resume_quest, DispatchOutcome, ESCALATION_PROTOCOL, MAX_AUTO_RESUME,
};
pub use escalation::{notify_user_escalation, triage_escalation, TriageDecision};
pub use worktree::{Worktree, WorktreeProvider, WorktrunkProvider};
