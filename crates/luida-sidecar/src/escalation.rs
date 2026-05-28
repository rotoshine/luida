//! escalation triage + 사용자 알림 (spec §5.6, §7.4).
//!
//! needs_input quest의 escalation을 `escalation.triage`(LLM)로 분류해
//! "자동 해소 가능(기본값)" vs "사용자에게 물어야 함"을 결정한다.
//! 사용자 알림(@user inmail)은 별도 primitive로 분리해 orchestrator가 게이트한다.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;

use luida_core::agents::{AgentInvocation, AgentRuntime, ResolvedAgent};
use luida_core::{
    resolve, AgentsConfig, Connection, EventRepo, InmailRepo, NewEvent, NewInmail, QuestRepo,
};

/// triage 판단 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct TriageDecision {
    /// true면 사용자에게 물어야 함. false면 auto_answer로 자동 재개 가능.
    pub ask_user: bool,
    pub auto_answer: Option<String>,
    pub reason: String,
}

/// LLM이 내는 triage JSON.
#[derive(Deserialize)]
struct RawDecision {
    ask_user: bool,
    #[serde(default)]
    auto_answer: Option<String>,
    #[serde(default)]
    reason: String,
}

/// needs_input quest의 escalation을 분류한다. 결정만 반환(알림은 별도).
pub fn triage_escalation<F>(
    conn: &mut Connection,
    cfg: &AgentsConfig,
    quest_id: i64,
    runtime_factory: F,
) -> Result<TriageDecision>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    let quest = QuestRepo::new(conn)
        .get(quest_id)?
        .with_context(|| format!("quest {quest_id} 없음"))?;
    if quest.status != "needs_input" {
        bail!("quest {quest_id}는 needs_input 상태가 아님({})", quest.status);
    }
    let (category, question) =
        latest_escalation(conn, quest_id)?.context("escalation 정보(needs_input 이벤트) 없음")?;

    let resolved = resolve(cfg, "escalation.triage", Some(&quest.project))?;
    let inv = AgentInvocation {
        prompt: build_triage_prompt(&category, &question, &quest.brief),
        ..Default::default()
    };
    let runtime = runtime_factory(&resolved).context("triage 런타임 생성 실패")?;
    let outcome = runtime.run(&resolved.model, &inv, &mut |_| {})?;
    let raw = outcome
        .summary
        .context("triage가 결정(JSON)을 반환하지 않음")?;
    let decision = parse_decision(&raw)?;

    EventRepo::new(conn).record(NewEvent {
        campaign_id: quest.campaign_id,
        quest_id: Some(quest_id),
        actor: "luida",
        kind: "escalation_triaged",
        payload: &json!({
            "category": category,
            "ask_user": decision.ask_user,
            "reason": decision.reason,
        })
        .to_string(),
    })?;

    Ok(decision)
}

/// needs_input quest에 대해 사용자에게 비방해 알림(@user escalation inmail)을 보낸다.
/// dedupe_key로 멱등 — 이미 보냈으면 false.
pub fn notify_user_escalation(conn: &Connection, quest_id: i64) -> Result<bool> {
    let quest = QuestRepo::new(conn)
        .get(quest_id)?
        .with_context(|| format!("quest {quest_id} 없음"))?;
    let (category, question) =
        latest_escalation(conn, quest_id)?.context("escalation 정보 없음")?;
    let res = InmailRepo::new(conn).enqueue(NewInmail {
        from_session: "luida",
        to_session: "@user",
        kind: "escalation",
        payload: &json!({
            "quest_id": quest_id,
            "project": quest.project,
            "category": category,
            "question": question,
        })
        .to_string(),
        reply_to: None,
        quest_id: Some(quest_id),
        campaign_id: quest.campaign_id,
        dedupe_key: Some(&format!("esc-q{quest_id}")),
    })?;
    Ok(res.inserted)
}

/// 가장 최근 needs_input 이벤트에서 (category, question) 회수.
fn latest_escalation(conn: &Connection, quest_id: i64) -> Result<Option<(String, String)>> {
    let Some(p) = EventRepo::new(conn).latest_payload_for_quest(quest_id, "quest_needs_input")?
    else {
        return Ok(None);
    };
    let v: serde_json::Value = serde_json::from_str(&p).unwrap_or_default();
    let category = v.get("category").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let question = v.get("question").and_then(|x| x.as_str()).unwrap_or("").to_string();
    Ok(Some((category, question)))
}

fn parse_decision(raw: &str) -> Result<TriageDecision> {
    let start = raw.find('{').context("triage JSON 없음")?;
    let end = raw.rfind('}').context("triage JSON 없음")?;
    if end <= start {
        bail!("triage JSON 형식 오류");
    }
    let rd: RawDecision = serde_json::from_str(&raw[start..=end])?;
    Ok(TriageDecision {
        ask_user: rd.ask_user,
        auto_answer: rd.auto_answer.filter(|s| !s.trim().is_empty()),
        reason: rd.reason,
    })
}

fn build_triage_prompt(category: &str, question: &str, brief: &str) -> String {
    format!(
        "당신은 Luida의 escalation 분류기입니다. worker가 작업 중 멈춰 아래 질문을 했습니다.\n\
사용자를 깨워야 할 만큼 중요한지, 아니면 안전한 기본값으로 자동 진행 가능한지 판단하세요.\n\n\
작업(brief): {brief}\n분류(category): {category}\n질문: {question}\n\n\
아래 JSON으로만 답하세요(설명 없이):\n\
{{\"ask_user\": bool, \"auto_answer\": string|null(자동 진행 시 worker에게 줄 답변), \"reason\": string}}\n\
- 위험(dangerous_op)·설계 충돌(design_mismatch)은 대개 ask_user=true.\n\
- 사소한 모호함은 합리적 기본값으로 auto_answer 제시하고 ask_user=false."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::dispatch_quest;
    use crate::worktree::{Worktree, WorktreeProvider};
    use luida_core::agents::{AgentEvent, ScriptedRuntime};
    use luida_core::{migrate, open_memory, NewQuest, ProjectRepo};

    fn cfg() -> AgentsConfig {
        let json = r#"{
          "version": 1,
          "defaults": { "runtime": "claude", "tier": "simple" },
          "runtimes": {
            "claude": { "kind": "claude-cli", "command": "claude",
              "models": { "complex": "opus", "simple": "sonnet" } }
          },
          "actions": {
            "quest.execute": { "runtime": "claude", "tier": "simple" },
            "escalation.triage": { "runtime": "claude", "tier": "complex" }
          }
        }"#;
        AgentsConfig::from_json(json).unwrap()
    }

    struct FakeWorktree;
    impl WorktreeProvider for FakeWorktree {
        fn create(&self, _r: &std::path::Path, codename: &str) -> Result<Worktree> {
            Ok(Worktree {
                branch: codename.to_string(),
                path: std::path::PathBuf::from("/tmp/x"),
            })
        }
    }

    fn factory(events: Vec<AgentEvent>) -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        move |_| Ok(Box::new(ScriptedRuntime::new(events.clone())) as Box<dyn AgentRuntime>)
    }

    fn result_factory(summary: &str) -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        let s = summary.to_string();
        move |_| {
            Ok(Box::new(ScriptedRuntime::new(vec![AgentEvent::Result {
                success: true,
                summary: Some(s.clone()),
            }])) as Box<dyn AgentRuntime>)
        }
    }

    /// needs_input 상태 quest를 만든다.
    fn needs_input_quest(conn: &mut Connection) -> i64 {
        ProjectRepo::new(conn).add("agora", "/a", "main", None).unwrap();
        let id = QuestRepo::new(conn)
            .insert(NewQuest {
                campaign_id: None,
                project: "agora",
                brief: "작업",
                branch: None,
                status: "pending",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        dispatch_quest(
            conn,
            &cfg(),
            id,
            &FakeWorktree,
            factory(vec![AgentEvent::Escalation {
                category: "dangerous_op".into(),
                message: "DB를 drop해도 될까요?".into(),
            }]),
        )
        .unwrap();
        id
    }

    fn db() -> Connection {
        let mut c = open_memory().unwrap();
        migrate(&mut c).unwrap();
        c
    }

    #[test]
    fn triage_ask_user() {
        let mut conn = db();
        let id = needs_input_quest(&mut conn);
        let d = triage_escalation(
            &mut conn,
            &cfg(),
            id,
            result_factory(r#"{"ask_user": true, "reason": "위험 작업"}"#),
        )
        .unwrap();
        assert!(d.ask_user);
        assert!(d.auto_answer.is_none());
        // triage는 알림을 자동으로 보내지 않음
        assert_eq!(InmailRepo::new(&conn).pending_for("@user").unwrap().len(), 0);
    }

    #[test]
    fn triage_auto_resolve() {
        let mut conn = db();
        let id = needs_input_quest(&mut conn);
        let d = triage_escalation(
            &mut conn,
            &cfg(),
            id,
            result_factory(r#"{"ask_user": false, "auto_answer": "기본 옵션", "reason": "사소"}"#),
        )
        .unwrap();
        assert!(!d.ask_user);
        assert_eq!(d.auto_answer.as_deref(), Some("기본 옵션"));
    }

    #[test]
    fn triage_rejects_non_needs_input() {
        let mut conn = db();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let id = QuestRepo::new(&conn)
            .insert(NewQuest {
                campaign_id: None,
                project: "agora",
                brief: "x",
                branch: None,
                status: "pending",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        assert!(triage_escalation(&mut conn, &cfg(), id, result_factory("{}")).is_err());
    }

    #[test]
    fn notify_is_idempotent() {
        let mut conn = db();
        let id = needs_input_quest(&mut conn);
        assert!(notify_user_escalation(&conn, id).unwrap()); // 첫 알림
        assert!(!notify_user_escalation(&conn, id).unwrap()); // 중복 → false
        let mail = InmailRepo::new(&conn).pending_for("@user").unwrap();
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].kind, "escalation");
    }
}
