//! luida-runtimes — 실제 CLI(claude/codex) spawn 기반 AgentRuntime 구현.
//!
//! 로컬 CLI 전제 (ADR-0001): claude·codex가 PATH에 설치됨.
//! API 기반(openai-compatible)은 backlog.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use luida_core::agents::{
    finalize_outcome, fold_outcome, parse_claude_stream_line, AgentEvent, AgentInvocation,
    AgentOutcome, AgentRuntime,
};

/// `claude -p --output-format stream-json` 어댑터.
pub struct ClaudeCliRuntime {
    bin: String,
}

impl Default for ClaudeCliRuntime {
    fn default() -> Self {
        Self {
            bin: "claude".to_string(),
        }
    }
}

impl ClaudeCliRuntime {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }
}

impl AgentRuntime for ClaudeCliRuntime {
    fn run(
        &self,
        model: &str,
        inv: &AgentInvocation,
        on_event: &mut dyn FnMut(&AgentEvent),
    ) -> Result<AgentOutcome> {
        let mut cmd = Command::new(&self.bin);
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--model")
            .arg(model);
        if let Some(sid) = &inv.session_id {
            cmd.arg("--session-id").arg(sid);
        }
        cmd.arg(&inv.prompt);
        if let Some(cwd) = &inv.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("claude CLI 실행 실패 (bin={})", self.bin))?;

        let stdout = child
            .stdout
            .take()
            .context("claude stdout 파이프 없음")?;
        let reader = BufReader::new(stdout);

        let mut outcome = AgentOutcome::default();
        for line in reader.lines() {
            let line = line?;
            if let Some(ev) = parse_claude_stream_line(&line) {
                fold_outcome(&mut outcome, &ev);
                on_event(&ev);
            }
        }

        let status = child.wait()?;
        Ok(finalize_outcome(outcome, status.success()))
    }
}

/// `codex exec --model <m>` 어댑터 (stream 파싱은 claude와 유사 가정).
/// codex의 정확한 stream 포맷은 실제 통합 시 조정 (현재 claude 파서 재사용 시도).
pub struct CodexCliRuntime {
    bin: String,
}

impl Default for CodexCliRuntime {
    fn default() -> Self {
        Self {
            bin: "codex".to_string(),
        }
    }
}

impl CodexCliRuntime {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }
}

impl AgentRuntime for CodexCliRuntime {
    fn run(
        &self,
        model: &str,
        inv: &AgentInvocation,
        on_event: &mut dyn FnMut(&AgentEvent),
    ) -> Result<AgentOutcome> {
        let mut cmd = Command::new(&self.bin);
        cmd.arg("exec").arg("--model").arg(model).arg(&inv.prompt);
        if let Some(cwd) = &inv.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("codex CLI 실행 실패 (bin={})", self.bin))?;
        let stdout = child.stdout.take().context("codex stdout 파이프 없음")?;
        let reader = BufReader::new(stdout);

        let mut outcome = AgentOutcome::default();
        for line in reader.lines() {
            let line = line?;
            if let Some(ev) = parse_claude_stream_line(&line) {
                fold_outcome(&mut outcome, &ev);
                on_event(&ev);
            }
        }
        let status = child.wait()?;
        Ok(finalize_outcome(outcome, status.success()))
    }
}

/// runtime kind 문자열 → AgentRuntime 인스턴스. (openai-compatible은 backlog → None)
pub fn runtime_for_kind(kind: &str, command: Option<&str>) -> Option<Box<dyn AgentRuntime>> {
    match kind {
        "claude-cli" => Some(Box::new(match command {
            Some(c) => ClaudeCliRuntime::new(c),
            None => ClaudeCliRuntime::default(),
        })),
        "codex-cli" => Some(Box::new(match command {
            Some(c) => CodexCliRuntime::new(c),
            None => CodexCliRuntime::default(),
        })),
        _ => None, // openai-compatible 등은 backlog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_for_kind_maps_cli() {
        assert!(runtime_for_kind("claude-cli", Some("claude")).is_some());
        assert!(runtime_for_kind("codex-cli", None).is_some());
        assert!(runtime_for_kind("openai-compatible", None).is_none());
        assert!(runtime_for_kind("unknown", None).is_none());
    }

    #[test]
    fn claude_runtime_spawn_failure_is_error() {
        // 존재하지 않는 bin → spawn 실패가 Result Err로 전파 (패닉 안 함)
        let rt = ClaudeCliRuntime::new("luida-nonexistent-claude-xyz");
        let res = rt.run(
            "model",
            &AgentInvocation {
                prompt: "hi".into(),
                ..Default::default()
            },
            &mut |_| {},
        );
        assert!(res.is_err());
    }

    #[test]
    fn codex_runtime_spawn_failure_is_error() {
        let rt = CodexCliRuntime::new("luida-nonexistent-codex-xyz");
        let res = rt.run("m", &AgentInvocation::default(), &mut |_| {});
        assert!(res.is_err());
    }
}
