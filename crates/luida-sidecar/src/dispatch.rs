//! Quest 디스패치 — resolve → worktree → worker 실행 → events/status/escalation 배선.
//!
//! runtime은 **factory 주입**으로 받아 실제 CLI 없이 `ScriptedRuntime`으로 테스트한다.
//!
//! 견고성(review V2-P2):
//!  - status='running' 이후 어떤 실패(런타임 생성/실행)도 quest를 'failed'로 되돌림 → 좀비 방지
//!  - 시작·판정의 다중 쓰기(status+event)는 트랜잭션으로 원자화
//!  - 스트림 이벤트 기록 실패는 무시하되 관찰 가능하게 경고
//!
//! 사용자 알림(@user inmail)은 dispatcher가 아니라 triage/orchestrator가 게이트한다(spec §7.4).

use std::cell::Cell;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::json;

use luida_core::agents::{AgentEvent, AgentInvocation, AgentOutcome, AgentRuntime, ResolvedAgent};
use luida_core::models::QUEST_TERMINAL;
use luida_core::{
    resolve, AgentsConfig, Connection, EventRepo, NewEvent, ProjectRepo, QuestRepo,
};

use luida_core::machine_id;

use crate::worktree::{Worktree, WorktreeProvider};

/// worker brief에 주입하는 escalation 마커 규약 (headless 신호 수단, spec §5.6).
pub const ESCALATION_PROTOCOL: &str = "\n\n---\n[Luida 규약] 판단이 필요하면 즉시 멈추고 아래 마커로 질문하라:\n<<LUIDA_ASK category=<system_error|ambiguous_spec|design_mismatch|dangerous_op>>>\n사용자에게 물을 질문\n<<END>>\n다른 경우엔 작업을 끝까지 수행하라.\n";

/// 자동 재개 최대 횟수 (orchestrator에서 사용 — 무한 escalation 루프 방지).
pub const MAX_AUTO_RESUME: u32 = 2;

/// 디스패치/재개 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchOutcome {
    Completed { summary: Option<String> },
    NeedsInput { category: String, question: String },
    Failed { summary: Option<String> },
    /// 사용자 취소(TUI 종료)로 중단됨 — quest 는 'pending'(이어받기 가능)으로 되돌아간다.
    Interrupted,
}

/// quest 하나를 새로 실행한다(신규 worktree + 새 세션).
pub fn dispatch_quest<F>(
    conn: &mut Connection,
    cfg: &AgentsConfig,
    quest_id: i64,
    worktree: &dyn WorktreeProvider,
    runtime_factory: F,
) -> Result<DispatchOutcome>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    let quest = QuestRepo::new(conn)
        .get(quest_id)?
        .with_context(|| format!("quest {quest_id} 없음"))?;
    if QUEST_TERMINAL.contains(&quest.status.as_str()) {
        bail!("quest {quest_id}는 이미 종료 상태({})", quest.status);
    }

    let project = ProjectRepo::new(conn)
        .get(&quest.project)?
        .with_context(|| format!("프로젝트 '{}' 미등록", quest.project))?;

    let resolved = resolve(cfg, "quest.execute", Some(&quest.project))?;

    // 중단 후 재개(worktree_path 가 **실재**)면 기존 worktree 재사용 + --resume 으로 직전 세션을
    // 이어받는다. 신규이거나 기존 worktree 가 사라졌으면 새로 만들고 처음부터(resume=false).
    let reuse = quest
        .worktree_path
        .as_deref()
        .filter(|p| Path::new(p).is_dir());
    let (wt, is_resume) = match reuse {
        Some(existing) => (
            Worktree {
                branch: quest.branch.clone().unwrap_or_else(|| format!("luida/q{quest_id}")),
                path: PathBuf::from(existing),
            },
            true,
        ),
        None => {
            let codename = quest
                .branch
                .clone()
                .unwrap_or_else(|| format!("luida/q{quest_id}"));
            let wt = worktree
                .create(Path::new(&project.repo_path), &codename)
                .context("worktree 생성 실패")?;
            (wt, false)
        }
    };

    let campaign_id = quest.campaign_id;
    let actor = quest.project.clone();
    let started_at = luida_core::process_start_time(std::process::id());

    // 시작: worktree + running + runner 리스(고아 재조정용) + dispatched 이벤트 (원자)
    {
        let tx = conn.transaction()?;
        {
            let qr = QuestRepo::new(&tx);
            qr.set_worktree(quest_id, &wt.branch, &wt.path.to_string_lossy())?;
            qr.set_runner(quest_id, std::process::id() as i64, &machine_id(), started_at)?;
            qr.set_status(quest_id, "running")?;
        }
        EventRepo::new(&tx).record(NewEvent {
            campaign_id,
            quest_id: Some(quest_id),
            actor: &actor,
            kind: "quest_dispatched",
            payload: &json!({
                "runtime": resolved.runtime,
                "model": resolved.model,
                "mode": resolved.mode,
                "branch": wt.branch,
                "resume": is_resume,
            })
            .to_string(),
        })?;
        tx.commit()?;
    }

    let inv = AgentInvocation {
        prompt: format!("{}{}", quest.brief, ESCALATION_PROTOCOL),
        cwd: Some(wt.path),
        session_id: Some(format!("luida-q{quest_id}")),
        system_context: project.context_path.clone(),
        resume: is_resume,
    };

    run_worker(conn, quest_id, campaign_id, &actor, &resolved, &inv, runtime_factory)
}

/// needs_input quest를 사용자/자동 답변으로 재개한다(기존 worktree + `--resume`).
pub fn resume_quest<F>(
    conn: &mut Connection,
    cfg: &AgentsConfig,
    quest_id: i64,
    answer: &str,
    runtime_factory: F,
) -> Result<DispatchOutcome>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    let quest = QuestRepo::new(conn)
        .get(quest_id)?
        .with_context(|| format!("quest {quest_id} 없음"))?;
    if quest.status != "needs_input" {
        bail!("quest {quest_id}는 needs_input 상태가 아님({})", quest.status);
    }
    let worktree_path = quest
        .worktree_path
        .clone()
        .context("resume할 worktree 경로가 없음")?;

    let resolved = resolve(cfg, "quest.execute", Some(&quest.project))?;
    let campaign_id = quest.campaign_id;
    let actor = quest.project.clone();

    {
        let tx = conn.transaction()?;
        {
            let qr = QuestRepo::new(&tx);
            let started_at = luida_core::process_start_time(std::process::id());
            qr.set_runner(quest_id, std::process::id() as i64, &machine_id(), started_at)?;
            qr.set_status(quest_id, "running")?;
        }
        EventRepo::new(&tx).record(NewEvent {
            campaign_id,
            quest_id: Some(quest_id),
            actor: &actor,
            kind: "quest_resumed",
            payload: &json!({ "answer_len": answer.len() }).to_string(),
        })?;
        tx.commit()?;
    }

    let inv = AgentInvocation {
        prompt: answer.to_string(),
        cwd: Some(PathBuf::from(worktree_path)),
        session_id: Some(format!("luida-q{quest_id}")),
        system_context: None,
        resume: true,
    };

    run_worker(conn, quest_id, campaign_id, &actor, &resolved, &inv, runtime_factory)
}

/// 런타임 생성 → 실행 → 스트림 기록 → outcome 판정. dispatch/resume 공통.
/// 실패 시 quest를 failed로 회수(좀비 방지)한 뒤 전파.
fn run_worker<F>(
    conn: &mut Connection,
    quest_id: i64,
    campaign_id: Option<i64>,
    actor: &str,
    resolved: &ResolvedAgent,
    inv: &AgentInvocation,
    runtime_factory: F,
) -> Result<DispatchOutcome>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    let runtime = match runtime_factory(resolved) {
        Ok(r) => r,
        Err(e) => {
            finalize_failed(conn, quest_id, campaign_id, actor, &format!("런타임 생성 실패: {e}"))?;
            return Err(e.context("런타임 생성 실패"));
        }
    };

    let event_err = Cell::new(false);
    let run_result = {
        let cref: &Connection = conn;
        let mut on_event = |ev: &AgentEvent| {
            if write_stream_event(cref, campaign_id, quest_id, actor, ev).is_err() {
                event_err.set(true);
            }
        };
        runtime.run(&resolved.model, inv, &mut on_event)
    };
    if event_err.get() {
        eprintln!("⚠ quest {quest_id}: 스트림 이벤트 일부 기록 실패 (DB 오류) — outcome은 유효");
    }

    let outcome = match run_result {
        Ok(o) => o,
        Err(e) => {
            finalize_failed(conn, quest_id, campaign_id, actor, &format!("worker 실행 에러: {e}"))?;
            return Err(e);
        }
    };

    settle_outcome(conn, quest_id, campaign_id, actor, outcome)
}

/// outcome → DB 상태 전이 + DispatchOutcome.
/// escalation은 headless 프로토콜상 worker가 result 없이 의도적으로 멈춘 신호(spec §5.6)라
/// success/saw_result와 무관하게 needs_input. (success=true ⟹ saw_result=true는 finalize가 보장)
fn settle_outcome(
    conn: &mut Connection,
    quest_id: i64,
    campaign_id: Option<i64>,
    actor: &str,
    outcome: AgentOutcome,
) -> Result<DispatchOutcome> {
    // 사용자 취소(TUI 종료)가 최우선 — 'failed'가 아니라 'pending'(이어받기 가능)으로 되돌린다.
    if outcome.cancelled {
        let tx = conn.transaction()?;
        QuestRepo::new(&tx).set_status(quest_id, "pending")?;
        EventRepo::new(&tx).record(NewEvent {
            campaign_id,
            quest_id: Some(quest_id),
            actor,
            kind: "quest_interrupted",
            payload: &json!({ "reason": "user_cancelled" }).to_string(),
        })?;
        tx.commit()?;
        return Ok(DispatchOutcome::Interrupted);
    }

    if let Some((category, question)) = outcome.escalation {
        let tx = conn.transaction()?;
        QuestRepo::new(&tx).set_status(quest_id, "needs_input")?;
        EventRepo::new(&tx).record(NewEvent {
            campaign_id,
            quest_id: Some(quest_id),
            actor,
            kind: "quest_needs_input",
            payload: &json!({ "category": category, "question": question }).to_string(),
        })?;
        tx.commit()?;
        return Ok(DispatchOutcome::NeedsInput { category, question });
    }

    if outcome.success {
        let tx = conn.transaction()?;
        QuestRepo::new(&tx).mark_completed(quest_id, None)?;
        EventRepo::new(&tx).record(NewEvent {
            campaign_id,
            quest_id: Some(quest_id),
            actor,
            kind: "quest_completed",
            payload: &json!({ "summary": outcome.summary }).to_string(),
        })?;
        tx.commit()?;
        Ok(DispatchOutcome::Completed {
            summary: outcome.summary,
        })
    } else {
        finalize_failed(
            conn,
            quest_id,
            campaign_id,
            actor,
            outcome.summary.as_deref().unwrap_or("worker 비정상 종료"),
        )?;
        Ok(DispatchOutcome::Failed {
            summary: outcome.summary,
        })
    }
}

/// quest를 'failed'로 되돌리고 이벤트 기록 (원자). 좀비 방지용 공통 경로.
fn finalize_failed(
    conn: &mut Connection,
    quest_id: i64,
    campaign_id: Option<i64>,
    actor: &str,
    summary: &str,
) -> Result<()> {
    let tx = conn.transaction()?;
    QuestRepo::new(&tx).set_status(quest_id, "failed")?;
    EventRepo::new(&tx).record(NewEvent {
        campaign_id,
        quest_id: Some(quest_id),
        actor,
        kind: "quest_failed",
        payload: &json!({ "summary": summary }).to_string(),
    })?;
    tx.commit()?;
    Ok(())
}

/// 스트림 이벤트 1건을 DB에 반영 (tool_use/escalation→events, text→progress).
fn write_stream_event(
    conn: &Connection,
    campaign_id: Option<i64>,
    quest_id: i64,
    actor: &str,
    ev: &AgentEvent,
) -> Result<()> {
    match ev {
        AgentEvent::ToolUse { name } => {
            EventRepo::new(conn).record(NewEvent {
                campaign_id,
                quest_id: Some(quest_id),
                actor,
                kind: "tool_use",
                payload: name,
            })?;
        }
        AgentEvent::Text { text } => {
            QuestRepo::new(conn).set_progress(quest_id, Some(&first_line(text, 200)))?;
        }
        AgentEvent::Escalation { category, message } => {
            EventRepo::new(conn).record(NewEvent {
                campaign_id,
                quest_id: Some(quest_id),
                actor,
                kind: "escalation",
                payload: &json!({ "category": category, "message": message }).to_string(),
            })?;
        }
        _ => {}
    }
    Ok(())
}

/// 첫 줄을 최대 `max` 바이트 경계(char-safe, 최소 1글자)로 잘라 progress 표시용으로.
fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.len() <= max {
        return line.to_string();
    }
    let mut end = 0;
    for (i, c) in line.char_indices() {
        let next = i + c.len_utf8();
        if next > max {
            break;
        }
        end = next;
    }
    if end == 0 {
        end = line.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
    }
    format!("{}…", &line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use luida_core::agents::ScriptedRuntime;
    use luida_core::{migrate, open_memory, EventRepo, NewQuest, ProjectRepo, QuestRepo};

    fn cfg() -> AgentsConfig {
        let json = r#"{
          "version": 1,
          "defaults": { "runtime": "claude", "tier": "simple" },
          "runtimes": {
            "claude": { "kind": "claude-cli", "command": "claude",
              "models": { "complex": "opus", "simple": "sonnet" } }
          },
          "actions": { "quest.execute": { "runtime": "claude", "tier": "simple" } }
        }"#;
        AgentsConfig::from_json(json).unwrap()
    }

    fn setup() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn)
            .add("agora", "/repos/agora", "main", None)
            .unwrap();
        conn
    }

    fn new_quest(conn: &Connection) -> i64 {
        QuestRepo::new(conn)
            .insert(NewQuest {
                campaign_id: None,
                project: "agora",
                brief: "스키마 변경 반영",
                branch: None,
                status: "pending",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap()
    }

    struct FakeWorktree;
    impl WorktreeProvider for FakeWorktree {
        fn create(&self, _repo: &Path, codename: &str) -> Result<crate::worktree::Worktree> {
            Ok(crate::worktree::Worktree {
                branch: codename.to_string(),
                path: PathBuf::from(format!("/tmp/luida-test/{codename}")),
            })
        }
    }

    struct FailWorktree;
    impl WorktreeProvider for FailWorktree {
        fn create(&self, _repo: &Path, _codename: &str) -> Result<crate::worktree::Worktree> {
            bail!("worktree 생성 불가(테스트)")
        }
    }

    fn factory(
        script: Vec<AgentEvent>,
    ) -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        move |_| Ok(Box::new(ScriptedRuntime::new(script.clone())) as Box<dyn AgentRuntime>)
    }

    fn failing_factory() -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        |_| bail!("런타임 생성 실패(테스트)")
    }

    /// 사용자 취소(TUI 종료)를 모사 — cancelled outcome 을 낸다.
    struct CancelledRuntime;
    impl AgentRuntime for CancelledRuntime {
        fn run(
            &self,
            _model: &str,
            _inv: &AgentInvocation,
            _on_event: &mut dyn FnMut(&AgentEvent),
        ) -> Result<AgentOutcome> {
            Ok(AgentOutcome { cancelled: true, ..Default::default() })
        }
    }
    fn cancelled_factory() -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        |_| Ok(Box::new(CancelledRuntime) as Box<dyn AgentRuntime>)
    }

    #[test]
    fn dispatch_success_completes_quest() {
        let mut conn = setup();
        let id = new_quest(&conn);
        let script = vec![
            AgentEvent::Text { text: "작업 시작".into() },
            AgentEvent::ToolUse { name: "Edit".into() },
            AgentEvent::Result {
                success: true,
                summary: Some("완료".into()),
            },
        ];
        let out = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, factory(script)).unwrap();
        assert_eq!(out, DispatchOutcome::Completed { summary: Some("완료".into()) });
        let q = QuestRepo::new(&conn).get(id).unwrap().unwrap();
        assert_eq!(q.status, "completed");
        assert_eq!(q.branch.as_deref(), Some("luida/q1"));
        assert!(q.worktree_path.is_some());
        assert!(q.completed_at.is_some());
        let evs = EventRepo::new(&conn).recent_since(0, 50).unwrap();
        assert!(evs.iter().any(|e| e.kind == "quest_dispatched"));
        assert!(evs.iter().any(|e| e.kind == "tool_use"));
        assert!(evs.iter().any(|e| e.kind == "quest_completed"));
    }

    #[test]
    fn dispatch_escalation_sets_needs_input() {
        let mut conn = setup();
        let id = new_quest(&conn);
        let script = vec![AgentEvent::Escalation {
            category: "design_mismatch".into(),
            message: "어느 스키마를 따를까요?".into(),
        }];
        let out = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, factory(script)).unwrap();
        assert_eq!(
            out,
            DispatchOutcome::NeedsInput {
                category: "design_mismatch".into(),
                question: "어느 스키마를 따를까요?".into()
            }
        );
        assert_eq!(QuestRepo::new(&conn).get(id).unwrap().unwrap().status, "needs_input");
        // dispatcher는 사용자 알림을 만들지 않음 (triage가 게이트)
        let evs = EventRepo::new(&conn).recent_since(0, 50).unwrap();
        assert!(evs.iter().any(|e| e.kind == "quest_needs_input"));
    }

    #[test]
    fn dispatch_failure_marks_failed() {
        let mut conn = setup();
        let id = new_quest(&conn);
        let script = vec![AgentEvent::Error { message: "panic".into() }];
        let out = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, factory(script)).unwrap();
        assert!(matches!(out, DispatchOutcome::Failed { .. }));
        assert_eq!(QuestRepo::new(&conn).get(id).unwrap().unwrap().status, "failed");
    }

    #[test]
    fn cancelled_sets_pending_and_records_runner_lease() {
        // 사용자 취소 → 'failed'가 아니라 'pending'(이어받기 가능) + quest_interrupted 이벤트.
        // 또한 dispatch 시작 시 runner 리스(pid)가 기록됐는지 확인(재조정 입력).
        let mut conn = setup();
        let id = new_quest(&conn);
        let out =
            dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, cancelled_factory()).unwrap();
        assert_eq!(out, DispatchOutcome::Interrupted);
        let q = QuestRepo::new(&conn).get(id).unwrap().unwrap();
        assert_eq!(q.status, "pending"); // 이어받기 가능
        assert!(q.worktree_path.is_some()); // worktree 유지 → 재개 시 재사용
        let evs = EventRepo::new(&conn).recent_since(0, 50).unwrap();
        assert!(evs.iter().any(|e| e.kind == "quest_interrupted"));
        // dispatch 가 runner_pid 를 기록했는지(이 프로세스).
        let pid: i64 = conn
            .query_row("SELECT runner_pid FROM quests WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(pid, std::process::id() as i64);
    }

    /// 실재하는 임시 worktree 디렉터리.
    fn real_worktree_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("luida-wt-{}-{tag}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn redispatch_reuses_existing_worktree_and_resumes() {
        // 중단 후 재개 모사: worktree_path 가 **실재**하는 pending quest 를 다시 dispatch.
        // → 새 worktree 를 만들지 않고(FailWorktree 여도 성공) --resume 으로 이어받는다.
        let mut conn = setup();
        let id = new_quest(&conn);
        let wt = real_worktree_dir("reuse");
        QuestRepo::new(&conn)
            .set_worktree(id, "luida/q1", &wt.to_string_lossy())
            .unwrap();
        let script = vec![AgentEvent::Result {
            success: true,
            summary: Some("이어서 완료".into()),
        }];
        // FailWorktree.create 가 호출되면 실패 → 재사용 경로라야 성공한다.
        let out = dispatch_quest(&mut conn, &cfg(), id, &FailWorktree, factory(script)).unwrap();
        assert!(matches!(out, DispatchOutcome::Completed { .. }), "재사용 실패: {out:?}");
        let evs = EventRepo::new(&conn).recent_since(0, 50).unwrap();
        let disp = evs.iter().find(|e| e.kind == "quest_dispatched").unwrap();
        assert!(disp.payload.contains("\"resume\":true"), "resume=true 여야: {}", disp.payload);
        let _ = std::fs::remove_dir_all(&wt);
    }

    #[test]
    fn redispatch_recreates_when_worktree_gone() {
        // worktree_path 가 기록돼 있지만 디렉터리가 사라진 경우 → 'failed' 가 아니라
        // 새 worktree 를 만들어 처음부터(resume=false) 재시도한다.
        let mut conn = setup();
        let id = new_quest(&conn);
        QuestRepo::new(&conn)
            .set_worktree(id, "luida/q1", "/nonexistent/luida-gone-xyz")
            .unwrap();
        let script = vec![AgentEvent::Result { success: true, summary: Some("처음부터".into()) }];
        // FakeWorktree.create 가 호출돼야(=새로 생성) 성공한다.
        let out = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, factory(script)).unwrap();
        assert!(matches!(out, DispatchOutcome::Completed { .. }), "재생성 실패: {out:?}");
        let evs = EventRepo::new(&conn).recent_since(0, 50).unwrap();
        let disp = evs.iter().find(|e| e.kind == "quest_dispatched").unwrap();
        assert!(disp.payload.contains("\"resume\":false"), "resume=false 여야: {}", disp.payload);
    }

    #[test]
    fn triage_cancelled_returns_interrupted_and_pending() {
        // triage 중 사용자 취소 → 하드 에러 아님. quest 를 pending 으로 되돌리고 interrupted 신호.
        use crate::escalation::triage_escalation;
        let mut conn = setup();
        let id = new_quest(&conn);
        // needs_input 상태 + escalation 이벤트 준비.
        let script = vec![AgentEvent::Escalation {
            category: "ambiguous_spec".into(),
            message: "어느 쪽?".into(),
        }];
        dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, factory(script)).unwrap();
        assert_eq!(QuestRepo::new(&conn).get(id).unwrap().unwrap().status, "needs_input");
        // triage 가 cancelled outcome → interrupted.
        let decision = triage_escalation(&mut conn, &cfg(), id, cancelled_factory()).unwrap();
        assert!(decision.interrupted);
        assert_eq!(QuestRepo::new(&conn).get(id).unwrap().unwrap().status, "pending");
        let evs = EventRepo::new(&conn).recent_since(0, 50).unwrap();
        assert!(evs.iter().any(|e| e.kind == "quest_interrupted"));
    }

    #[test]
    fn runtime_factory_failure_marks_failed_not_zombie() {
        let mut conn = setup();
        let id = new_quest(&conn);
        let r = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, failing_factory());
        assert!(r.is_err());
        assert_eq!(QuestRepo::new(&conn).get(id).unwrap().unwrap().status, "failed");
    }

    #[test]
    fn worktree_failure_leaves_quest_pending() {
        let mut conn = setup();
        let id = new_quest(&conn);
        let r = dispatch_quest(&mut conn, &cfg(), id, &FailWorktree, factory(vec![]));
        assert!(r.is_err());
        assert_eq!(QuestRepo::new(&conn).get(id).unwrap().unwrap().status, "pending");
    }

    #[test]
    fn dispatch_terminal_quest_rejected() {
        let mut conn = setup();
        let id = new_quest(&conn);
        QuestRepo::new(&conn).mark_completed(id, None).unwrap();
        let r = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, factory(vec![]));
        assert!(r.is_err());
    }

    #[test]
    fn dispatch_missing_quest_errors() {
        let mut conn = setup();
        let r = dispatch_quest(&mut conn, &cfg(), 999, &FakeWorktree, factory(vec![]));
        assert!(r.is_err());
    }

    #[test]
    fn resume_completes_needs_input_quest() {
        let mut conn = setup();
        let id = new_quest(&conn);
        // 먼저 escalation으로 needs_input + worktree 세팅
        dispatch_quest(
            &mut conn,
            &cfg(),
            id,
            &FakeWorktree,
            factory(vec![AgentEvent::Escalation {
                category: "ambiguous_spec".into(),
                message: "어느?".into(),
            }]),
        )
        .unwrap();
        // 답변으로 재개 → 성공
        let out = resume_quest(
            &mut conn,
            &cfg(),
            id,
            "옵션 A로 진행",
            factory(vec![AgentEvent::Result { success: true, summary: Some("ok".into()) }]),
        )
        .unwrap();
        assert_eq!(out, DispatchOutcome::Completed { summary: Some("ok".into()) });
        assert_eq!(QuestRepo::new(&conn).get(id).unwrap().unwrap().status, "completed");
        let evs = EventRepo::new(&conn).recent_since(0, 50).unwrap();
        assert!(evs.iter().any(|e| e.kind == "quest_resumed"));
    }

    #[test]
    fn resume_rejects_non_needs_input() {
        let mut conn = setup();
        let id = new_quest(&conn); // pending
        let r = resume_quest(&mut conn, &cfg(), id, "x", factory(vec![]));
        assert!(r.is_err());
    }

    #[test]
    fn first_line_truncates_multibyte_safely() {
        assert_eq!(first_line("한국어 첫 줄입니다\n둘째", 200), "한국어 첫 줄입니다");
        let cut = first_line("가나다라마바사", 7);
        assert!(cut.ends_with('…') && cut.starts_with('가'));
        assert_eq!(first_line("가나다", 1), "가…");
    }
}
