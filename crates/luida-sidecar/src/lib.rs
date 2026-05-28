//! luida-sidecar — quest 실행 오케스트레이션.
//!
//! - `worktree`: worker용 격리 작업공간 provisioning (worktrunk 기반)
//! - `dispatch`: resolve → worktree → worker 실행 → events/status/escalation 배선
//!
//! runtime은 factory 주입이라 실제 CLI 없이 테스트 가능하다.

mod dispatch;
mod worktree;

pub use dispatch::{dispatch_quest, DispatchOutcome, ESCALATION_PROTOCOL};
pub use worktree::{Worktree, WorktreeProvider, WorktrunkProvider};
