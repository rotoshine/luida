//! luida-runtimes — 실제 CLI(claude/codex) spawn 기반 AgentRuntime 구현.
//!
//! 로컬 CLI 전제 (ADR-0001): claude·codex가 PATH에 설치됨.
//! API 기반(openai-compatible)은 backlog.
//!
//! 견고성:
//!  - stderr를 별도 스레드로 drain → 파이프 버퍼 가득참 데드락 방지 (review C1)
//!  - 종료 시 child kill+wait 보장 → 좀비/누수 방지 (review M7)
//!  - cwd는 spawn 전 디렉터리 검증 (review M6)

mod fake;
pub use fake::{fake_runtime_for, FakeRuntime};
pub use luida_core::CancelToken;

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use anyhow::{bail, Context, Result};
use luida_core::agents::{
    finalize_outcome, fold_outcome, parse_claude_stream_line, AgentEvent, AgentInvocation,
    AgentOutcome, AgentRuntime,
};

/// child가 drop될 때 반드시 kill+reap (좀비 방지).
struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(c: Child) -> Self {
        Self(Some(c))
    }
    fn take(&mut self) -> Option<Child> {
        self.0.take()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// 공통 CLI 스트리밍 실행. stdout NDJSON을 parse_claude_stream_line로 파싱.
/// `cancel` 이 주어지면: spawn 직후 자식 PID 등록, 루프마다 취소 확인, 취소 시 중단.
fn run_cli_streaming(
    mut cmd: Command,
    cwd: Option<&Path>,
    cancel: Option<&CancelToken>,
    on_event: &mut dyn FnMut(&AgentEvent),
) -> Result<AgentOutcome> {
    // 이미 취소됐으면 자식을 띄우지 않고 즉시 중단(원정 중간 취소 시 다음 quest 스폰 방지).
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Ok(AgentOutcome { cancelled: true, ..Default::default() });
    }
    if let Some(dir) = cwd {
        if !dir.is_dir() {
            bail!("worktree 경로가 디렉터리가 아님: {dir:?}");
        }
        cmd.current_dir(dir);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().context("CLI 실행 실패 (설치/PATH 확인)")?;
    // 취소 대상 PID 등록 (취소 시 SIGKILL). 등록 시점에 이미 취소됐다면 즉시 kill.
    if let Some(c) = cancel {
        c.register_child(child.id());
    }

    // stderr를 별도 스레드로 끝까지 drain (버퍼 가득참 데드락 방지). 마지막 일부 캡처.
    let stderr = child.stderr.take();
    let stderr_handle = stderr.map(|mut e| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = e.read_to_string(&mut buf);
            buf
        })
    });

    let stdout = child.stdout.take().context("stdout 파이프 없음")?;
    let mut guard = ChildGuard::new(child);

    let mut outcome = AgentOutcome::default();
    {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            // 취소 협조 — kill 된 자식은 곧 EOF 로 끝나지만, 출력 사이 취소도 즉시 반영.
            if cancel.is_some_and(|c| c.is_cancelled()) {
                break;
            }
            match line {
                Ok(l) => {
                    if let Some(ev) = parse_claude_stream_line(&l) {
                        fold_outcome(&mut outcome, &ev);
                        on_event(&ev);
                    }
                }
                // read 에러(비정상 UTF-8 등)는 스트림 중단으로 처리하되 child 정리는 guard가 보장
                Err(_) => break,
            }
        }
    }

    let cancelled = cancel.is_some_and(|c| c.is_cancelled());
    // 자식 PID 등록 해제 — 이 시점 이후 늦은 cancel()이 (이미 종료·회수됐거나 재사용된) PID 를
    // SIGKILL 하지 않도록. 취소 경로에서도 자식 정리는 guard(소유한 Child 핸들)의 kill+wait 가
    // 보장하므로, PID 아토믹을 미리 0으로 비워도 안전하다(=PID 재사용 TOCTOU 창 최소화).
    if let Some(c) = cancel {
        c.clear_child();
    }
    if cancelled {
        // 취소: child 정리는 guard(drop)에 맡기고 결과만 표시(중단).
        return Ok(AgentOutcome { cancelled: true, ..outcome });
    }
    // 정상 경로: guard에서 child를 꺼내 명시적으로 wait (kill 없이).
    let status = match guard.take() {
        Some(mut c) => c.wait()?,
        None => bail!("child 이미 정리됨"),
    };
    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    // 실패 + result 없음 → stderr 꼬리를 summary로 첨부 (진단성)
    if !status.success() && !outcome.saw_result {
        let tail: String = stderr_text.chars().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect();
        if !tail.trim().is_empty() {
            outcome.summary = Some(format!("CLI 실패: {}", tail.trim()));
        }
    }

    Ok(finalize_outcome(outcome, status.success()))
}

/// `claude -p --output-format stream-json` 어댑터.
pub struct ClaudeCliRuntime {
    bin: String,
    cancel: Option<CancelToken>,
}

impl Default for ClaudeCliRuntime {
    fn default() -> Self {
        Self {
            bin: "claude".to_string(),
            cancel: None,
        }
    }
}

impl ClaudeCliRuntime {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into(), cancel: None }
    }
    /// 취소 토큰 주입 — TUI 종료 시 이 런타임의 자식을 즉시 정리.
    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = Some(cancel);
        self
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
        // resume이면 --resume(직전 맥락 이어받기), 아니면 --session-id(새 세션).
        if let Some(sid) = &inv.session_id {
            if inv.resume {
                cmd.arg("--resume").arg(sid);
            } else {
                cmd.arg("--session-id").arg(sid);
            }
        }
        cmd.arg(&inv.prompt);
        run_cli_streaming(cmd, inv.cwd.as_deref(), self.cancel.as_ref(), on_event)
    }
}

/// `codex exec --model <m>` 어댑터.
/// codex의 정확한 stream 포맷은 실제 통합 시 조정 (현재 claude 파서 재사용 시도).
pub struct CodexCliRuntime {
    bin: String,
    cancel: Option<CancelToken>,
}

impl Default for CodexCliRuntime {
    fn default() -> Self {
        Self {
            bin: "codex".to_string(),
            cancel: None,
        }
    }
}

impl CodexCliRuntime {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into(), cancel: None }
    }
    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = Some(cancel);
        self
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
        run_cli_streaming(cmd, inv.cwd.as_deref(), self.cancel.as_ref(), on_event)
    }
}

/// 런타임 factory — `LUIDA_FAKE`면 결정적 데모 런타임, 아니면 로컬 CLI(claude/codex).
/// CLI·TUI 공용. (클로저는 Clone 불가라 각 호출 사이트에서 생성한다.)
pub fn make_factory() -> impl Fn(&luida_core::ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
    move |r: &luida_core::ResolvedAgent| build_runtime(r, None)
}

/// 취소 가능한 factory — 생성되는 모든 CLI 런타임에 취소 토큰을 주입한다.
/// TUI 가 종료 시 token.cancel() 로 실행 중인 자식을 즉시 정리(고아 방지)할 수 있다.
pub fn make_cancellable_factory(
    cancel: CancelToken,
) -> impl Fn(&luida_core::ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
    move |r: &luida_core::ResolvedAgent| build_runtime(r, Some(&cancel))
}

/// ResolvedAgent → 런타임 박스. fake 모드면 데모 런타임(취소 무관, 즉시 완료).
fn build_runtime(
    r: &luida_core::ResolvedAgent,
    cancel: Option<&CancelToken>,
) -> Result<Box<dyn AgentRuntime>> {
    if luida_core::is_fake() {
        return Ok(fake_runtime_for(&r.action));
    }
    runtime_for_kind_with(&r.kind, r.command.as_deref(), cancel)
}

/// runtime kind 문자열 → AgentRuntime. openai-compatible은 backlog → 명확한 에러.
pub fn runtime_for_kind(kind: &str, command: Option<&str>) -> Result<Box<dyn AgentRuntime>> {
    runtime_for_kind_with(kind, command, None)
}

fn runtime_for_kind_with(
    kind: &str,
    command: Option<&str>,
    cancel: Option<&CancelToken>,
) -> Result<Box<dyn AgentRuntime>> {
    match kind {
        "claude-cli" => {
            let mut rt = match command {
                Some(c) => ClaudeCliRuntime::new(c),
                None => ClaudeCliRuntime::default(),
            };
            if let Some(c) = cancel {
                rt = rt.with_cancel(c.clone());
            }
            Ok(Box::new(rt))
        }
        "codex-cli" => {
            let mut rt = match command {
                Some(c) => CodexCliRuntime::new(c),
                None => CodexCliRuntime::default(),
            };
            if let Some(c) = cancel {
                rt = rt.with_cancel(c.clone());
            }
            Ok(Box::new(rt))
        }
        "openai-compatible" => {
            bail!("openai-compatible 런타임은 아직 미지원 (backlog). 로컬 CLI(claude/codex)를 사용하세요")
        }
        other => bail!("알 수 없는 runtime kind: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_for_kind_maps_cli() {
        assert!(runtime_for_kind("claude-cli", Some("claude")).is_ok());
        assert!(runtime_for_kind("codex-cli", None).is_ok());
        assert!(runtime_for_kind("openai-compatible", None).is_err());
        assert!(runtime_for_kind("unknown", None).is_err());
    }

    #[test]
    fn claude_runtime_spawn_failure_is_error() {
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

    #[test]
    fn invalid_cwd_is_error() {
        let rt = ClaudeCliRuntime::new("echo");
        let res = rt.run(
            "m",
            &AgentInvocation {
                prompt: "x".into(),
                cwd: Some("/nonexistent/dir/xyz".into()),
                ..Default::default()
            },
            &mut |_| {},
        );
        assert!(res.is_err());
    }

    #[test]
    fn real_command_streams_and_reaps() {
        // 실제 NDJSON을 stdout으로 내는 가짜 worker로 스트리밍·종료 검증 (claude 불필요).
        // printf로 result 라인 출력.
        let rt = ClaudeishRunner;
        let mut events = 0;
        let outcome = rt
            .run("m", &AgentInvocation::default(), &mut |_| events += 1)
            .unwrap();
        assert!(events >= 1);
        assert!(outcome.saw_result);
        assert!(outcome.success);
    }

    /// 테스트 전용 — `printf`로 stream-json 한 줄을 내는 가짜 런타임.
    struct ClaudeishRunner;
    impl AgentRuntime for ClaudeishRunner {
        fn run(
            &self,
            _model: &str,
            inv: &AgentInvocation,
            on_event: &mut dyn FnMut(&AgentEvent),
        ) -> Result<AgentOutcome> {
            let mut cmd = Command::new("printf");
            cmd.arg(r#"{"type":"text","text":"hi"}
{"type":"result","summary":"done"}
"#);
            run_cli_streaming(cmd, inv.cwd.as_deref(), None, on_event)
        }
    }

    #[test]
    fn precancelled_does_not_spawn() {
        // 이미 취소된 토큰이면 자식을 띄우지 않고 즉시 중단 outcome (존재하지 않는 바이너리라도 에러 아님).
        let token = CancelToken::new();
        token.cancel();
        let cmd = Command::new("luida-nonexistent-xyz");
        let outcome = run_cli_streaming(cmd, None, Some(&token), &mut |_| {}).unwrap();
        assert!(outcome.cancelled);
        assert!(!outcome.success);
    }

    #[cfg(unix)]
    #[test]
    fn cancel_kills_inflight_child() {
        use luida_core::pid_alive;
        use std::time::{Duration, Instant};
        // 출력 없이 오래 도는 자식(sleep 30)을 실행 중 취소.
        let token = CancelToken::new();
        let t2 = token.clone();
        let handle = std::thread::spawn(move || {
            let mut cmd = Command::new("sleep");
            cmd.arg("30");
            run_cli_streaming(cmd, None, Some(&t2), &mut |_| {})
        });
        // 자식이 등록될 때까지 폴링(고정 sleep 대신) — 최대 ~3초.
        let mut pid = 0;
        for _ in 0..300 {
            pid = token.registered_pid();
            if pid != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(pid != 0, "자식 PID 가 등록돼야");
        assert!(pid_alive(pid), "취소 전 자식이 살아있어야");

        let t0 = Instant::now();
        token.cancel();
        let outcome = handle.join().unwrap().unwrap();
        // 1) 취소 플래그 반영, 2) sleep 30 을 다 기다리지 않고 즉시 종료, 3) 자식이 실제로 죽음.
        assert!(outcome.cancelled, "취소로 중단됐어야");
        assert!(t0.elapsed() < Duration::from_secs(5), "취소 후 즉시 종료해야(30초 대기 아님)");
        assert!(!pid_alive(pid), "자식이 SIGKILL 로 종료·회수됐어야");
    }
}
