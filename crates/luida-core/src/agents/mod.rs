//! Agent 설정(agents.json) + 행위→런타임/모델 해소(Resolver).
//!
//! 행위(action)별로 어떤 런타임(claude/codex/...)·모델·실행모드를 쓸지 결정한다.
//! 우선순위: projectOverrides > actions > defaults. tier 미지정 시 런타임의 tier별 기본 모델.

mod cancel;
mod config;
mod resolver;
mod runtime;
mod tokenjuice;

pub use cancel::{kill_process_group, pid_alive, process_start_time, runner_alive, CancelToken};
pub use config::{
    default_agents_path, ActionConfig, AgentsConfig, Defaults, RuntimeDef, RuntimeModels,
};
pub use resolver::{resolve, runtime_available, ResolvedAgent};
pub use runtime::{
    detect_escalation, finalize_outcome, fold_outcome, parse_claude_stream_line,
    AgentEvent, AgentInvocation, AgentOutcome, AgentRuntime, ScriptedRuntime,
    ESCALATION_CATEGORIES,
};
pub use tokenjuice::compress_context;
