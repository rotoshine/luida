//! Quest 디스패치 — resolve → worktree → worker 실행 → events/status/escalation 배선.
//!
//! runtime은 **factory 주입**으로 받아 실제 CLI 없이 `ScriptedRuntime`으로 테스트한다.
//!
//! 견고성(review V2-P2):
//!  - status='running' 이후 어떤 실패(런타임 생성/실행)도 quest를 'failed'로 되돌림 → 좀비 방지
//!  - 시작·판정의 다중 쓰기(status+inmail+event)는 트랜잭션으로 원자화 → 부분 실패 불일치 방지
//!  - 스트림 이벤트 기록 실패는 무시하되 관찰 가능하게 경고

use std::cell::Cell;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::json;

use luida_core::agents::{AgentEvent, AgentInvocation, AgentRuntime, ResolvedAgent};
use luida_core::models::QUEST_TERMINAL;
use luida_core::{
    resolve, AgentsConfig, Connection, EventRepo, InmailRepo, NewEvent, NewInmail, ProjectRepo,
    QuestRepo,
};

use crate::worktree::WorktreeProvider;

/// worker brief에 주입하는 escalation 마커 규약 (headless 신호 수단, spec §5.6).
pub const ESCALATION_PROTOCOL: &str = "\n\n---\n[Luida 규약] 판단이 필요하면 즉시 멈추고 아래 마커로 질문하라:\n<<LUIDA_ASK category=<system_error|ambiguous_spec|design_mismatch|dangerous_op>>>\n사용자에게 물을 질문\n<<END>>\n다른 경우엔 작업을 끝까지 수행하라.\n";

/// 디스패치 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchOutcome {
    /// 정상 완료.
    Completed { summary: Option<String> },
    /// escalation 발생 → 사용자 입력 대기(needs_input).
    NeedsInput { category: String, question: String },
    /// 실패(비정상 종료/result 없음).
    Failed { summary: Option<String> },
}

/// quest 하나를 실행한다.
///
/// 1. `quest.execute` 해소(프로젝트별 override 반영) → 런타임/모델 결정
/// 2. worktree 생성 → quest.worktree_path/branch/status=running 기록(원자)
/// 3. brief + escalation 규약으로 AgentInvocation 구성 → factory가 만든 런타임 실행
/// 4. 스트림 이벤트를 events에 기록·progress 갱신 (best-effort, 실패는 경고)
/// 5. outcome 판정: escalation→needs_input(+inmail), success→completed, else→failed (각 원자)
///
/// status='running' 이후의 모든 실패는 quest를 'failed'로 되돌린 뒤 에러를 전파한다(좀비 방지).
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

    // ── worktree provisioning (status 변경 전 — 실패 시 quest는 pending 유지) ──────
    let codename = quest
        .branch
        .clone()
        .unwrap_or_else(|| format!("luida/q{quest_id}"));
    let wt = worktree
        .create(Path::new(&project.repo_path), &codename)
        .context("worktree 생성 실패")?;

    let campaign_id = quest.campaign_id;
    let actor = quest.project.clone();

    // ── 시작: worktree + running + dispatched 이벤트 (원자) ───────────────────────
    {
        let tx = conn.transaction()?;
        {
            let qr = QuestRepo::new(&tx);
            qr.set_worktree(quest_id, &wt.branch, &wt.path.to_string_lossy())?;
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
            })
            .to_string(),
        })?;
        tx.commit()?;
    }

    // ── worker 실행 ──────────────────────────────────────────────────────────────
    let inv = AgentInvocation {
        prompt: format!("{}{}", quest.brief, ESCALATION_PROTOCOL),
        cwd: Some(wt.path.clone()),
        session_id: Some(format!("luida-q{quest_id}")),
        system_context: project.context_path.clone(),
    };

    // 런타임 생성 실패 → 좀비 방지 위해 failed 처리 후 전파
    let runtime = match runtime_factory(&resolved) {
        Ok(r) => r,
        Err(e) => {
            finalize_failed(conn, quest_id, campaign_id, &actor, &format!("런타임 생성 실패: {e}"))?;
            return Err(e.context("런타임 생성 실패"));
        }
    };

    let event_err = Cell::new(false);
    let run_result = {
        let cref: &Connection = conn;
        let mut on_event = |ev: &AgentEvent| {
            if write_stream_event(cref, campaign_id, quest_id, &actor, ev).is_err() {
                event_err.set(true);
            }
        };
        runtime.run(&resolved.model, &inv, &mut on_event)
    };
    if event_err.get() {
        eprintln!("⚠ quest {quest_id}: 스트림 이벤트 일부 기록 실패 (DB 오류) — outcome은 유효");
    }

    let outcome = match run_result {
        Ok(o) => o,
        Err(e) => {
            finalize_failed(conn, quest_id, campaign_id, &actor, &format!("worker 실행 에러: {e}"))?;
            return Err(e);
        }
    };

    // ── 판정 ─────────────────────────────────────────────────────────────────────
    // escalation은 headless 프로토콜상 worker가 result 없이 의도적으로 멈춘 신호(spec §5.6)
    // 이므로 success/saw_result와 무관하게 needs_input으로 처리한다.
    if let Some((category, question)) = outcome.escalation {
        let tx = conn.transaction()?;
        QuestRepo::new(&tx).set_status(quest_id, "needs_input")?;
        // 사용자에게 비방해 알림 (broadcast escalation — dispatch가 아니라 허용).
        InmailRepo::new(&tx).enqueue(NewInmail {
            from_session: "luida",
            to_session: "@user",
            kind: "escalation",
            payload: &json!({
                "quest_id": quest_id,
                "category": category,
                "question": question,
            })
            .to_string(),
            reply_to: None,
            quest_id: Some(quest_id),
            campaign_id,
            dedupe_key: Some(&format!("esc-q{quest_id}")),
        })?;
        EventRepo::new(&tx).record(NewEvent {
            campaign_id,
            quest_id: Some(quest_id),
            actor: &actor,
            kind: "quest_needs_input",
            payload: &json!({ "category": category }).to_string(),
        })?;
        tx.commit()?;
        return Ok(DispatchOutcome::NeedsInput { category, question });
    }

    // finalize_outcome이 saw_result 없으면 success=false로 강제하므로
    // success=true ⟹ result 관측됨 (별도 saw_result 체크 불필요).
    if outcome.success {
        let tx = conn.transaction()?;
        QuestRepo::new(&tx).mark_completed(quest_id, None)?;
        EventRepo::new(&tx).record(NewEvent {
            campaign_id,
            quest_id: Some(quest_id),
            actor: &actor,
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
            &actor,
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
        // 첫 글자가 max보다 길어도 최소 1글자는 보존
        end = line.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
    }
    format!("{}…", &line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use luida_core::agents::ScriptedRuntime;
    use luida_core::{migrate, open_memory, NewQuest, ProjectRepo, QuestRepo};

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

    /// cwd를 검증하지 않는 가짜 worktree provider (ScriptedRuntime이 cwd 무시).
    struct FakeWorktree;
    impl WorktreeProvider for FakeWorktree {
        fn create(&self, _repo: &Path, codename: &str) -> Result<crate::worktree::Worktree> {
            Ok(crate::worktree::Worktree {
                branch: codename.to_string(),
                path: std::path::PathBuf::from(format!("/tmp/luida-test/{codename}")),
            })
        }
    }

    /// 항상 실패하는 provider.
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
        assert_eq!(
            out,
            DispatchOutcome::Completed {
                summary: Some("완료".into())
            }
        );
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
    fn dispatch_escalation_sets_needs_input_and_inmail() {
        let mut conn = setup();
        let id = new_quest(&conn);
        let script = vec![
            AgentEvent::Escalation {
                category: "design_mismatch".into(),
                message: "어느 스키마를 따를까요?".into(),
            },
            AgentEvent::Result {
                success: true,
                summary: None,
            },
        ];
        let out = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, factory(script)).unwrap();
        assert_eq!(
            out,
            DispatchOutcome::NeedsInput {
                category: "design_mismatch".into(),
                question: "어느 스키마를 따를까요?".into()
            }
        );
        let q = QuestRepo::new(&conn).get(id).unwrap().unwrap();
        assert_eq!(q.status, "needs_input");
        let mail = InmailRepo::new(&conn).pending_for("@user").unwrap();
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].kind, "escalation");
        assert_eq!(mail[0].quest_id, Some(id));
    }

    #[test]
    fn dispatch_failure_marks_failed() {
        let mut conn = setup();
        let id = new_quest(&conn);
        let script = vec![
            AgentEvent::Text { text: "시작".into() },
            AgentEvent::Error { message: "panic".into() },
        ];
        let out = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, factory(script)).unwrap();
        assert!(matches!(out, DispatchOutcome::Failed { .. }));
        assert_eq!(
            QuestRepo::new(&conn).get(id).unwrap().unwrap().status,
            "failed"
        );
    }

    #[test]
    fn runtime_factory_failure_marks_failed_not_zombie() {
        // review Critical 1: running 이후 런타임 생성 실패 시 quest가 failed로 회수돼야 함
        let mut conn = setup();
        let id = new_quest(&conn);
        let r = dispatch_quest(&mut conn, &cfg(), id, &FakeWorktree, failing_factory());
        assert!(r.is_err());
        assert_eq!(
            QuestRepo::new(&conn).get(id).unwrap().unwrap().status,
            "failed"
        );
    }

    #[test]
    fn worktree_failure_leaves_quest_pending() {
        // worktree 실패는 status 변경 전이므로 pending 유지(재디스패치 가능)
        let mut conn = setup();
        let id = new_quest(&conn);
        let r = dispatch_quest(&mut conn, &cfg(), id, &FailWorktree, factory(vec![]));
        assert!(r.is_err());
        assert_eq!(
            QuestRepo::new(&conn).get(id).unwrap().unwrap().status,
            "pending"
        );
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
    fn first_line_truncates_multibyte_safely() {
        let s = "한국어 첫 줄입니다\n둘째 줄";
        assert_eq!(first_line(s, 200), "한국어 첫 줄입니다");
        let cut = first_line("가나다라마바사", 7);
        assert!(cut.ends_with('…'));
        assert!(cut.starts_with('가'));
        // 첫 글자가 max보다 길어도 최소 1글자 + …
        let tiny = first_line("가나다", 1);
        assert_eq!(tiny, "가…");
    }
}
