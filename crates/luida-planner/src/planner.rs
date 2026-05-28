//! 원정 계획·실행 — `campaign.plan` 행위로 DAG 생성 후 의존성 순으로 디스패치.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};
use serde_json::json;

use luida_core::agents::{AgentInvocation, AgentRuntime, ResolvedAgent};
use luida_core::{
    resolve, AgentsConfig, CampaignRepo, Connection, EventRepo, NewCampaign, NewEvent, NewQuest,
    ProjectRepo, QuestRepo,
};
use luida_sidecar::{
    dispatch_quest, notify_user_escalation, resume_quest, triage_escalation, DispatchOutcome,
    WorktreeProvider, MAX_AUTO_RESUME,
};

use crate::plan::CampaignPlan;

/// 원정 실행 결과 요약.
#[derive(Debug, Default, PartialEq)]
pub struct CampaignRunReport {
    pub completed: Vec<i64>,
    pub needs_input: Vec<i64>,
    pub failed: Vec<i64>,
    /// 원정의 모든 quest가 completed인가 (report 단계 진입 조건).
    pub all_completed: bool,
}

/// 사용자 프롬프트 → `campaign.plan`(LLM) → 검증된 DAG를 campaigns/quests에 영속.
/// 생성된 campaign id 반환 (status=planning).
pub fn plan_campaign<F>(
    conn: &mut Connection,
    cfg: &AgentsConfig,
    prompt: &str,
    runtime_factory: F,
) -> Result<i64>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    let known: HashSet<String> = ProjectRepo::new(conn)
        .list()?
        .into_iter()
        .map(|p| p.name)
        .collect();
    if known.is_empty() {
        bail!("등록된 모험지가 없습니다. 먼저 `luida project add`로 등록하세요.");
    }

    let resolved = resolve(cfg, "campaign.plan", None)?;
    let inv = AgentInvocation {
        prompt: build_plan_prompt(prompt, &known),
        cwd: None,
        session_id: None,
        system_context: None,
        resume: false,
    };
    let runtime = runtime_factory(&resolved).context("플래너 런타임 생성 실패")?;
    let outcome = runtime.run(&resolved.model, &inv, &mut |_| {})?;
    if !outcome.success {
        bail!("campaign.plan 실패: {:?}", outcome.summary);
    }
    let raw = outcome
        .summary
        .context("플래너가 계획(plan_json)을 반환하지 않음")?;

    let plan = CampaignPlan::parse(&raw)?;
    let order = plan.validate(&known)?;
    let plan_json = serde_json::to_string(&plan)?;

    // ── 영속 (원자) ──────────────────────────────────────────────────────────────
    let tx = conn.transaction()?;
    let cid = CampaignRepo::new(&tx).insert(NewCampaign {
        title: &plan.title,
        prompt,
        plan_json: &plan_json,
        status: "planning",
    })?;

    let by_key: HashMap<&str, &crate::plan::PlannedQuest> =
        plan.quests.iter().map(|q| (q.key.as_str(), q)).collect();
    let mut key_to_id: HashMap<String, i64> = HashMap::new();

    // 위상정렬 순서로 insert → 의존성 id가 항상 먼저 존재.
    {
        let qr = QuestRepo::new(&tx);
        for key in &order {
            let pq = by_key[key.as_str()];
            // 대표 의존(back-compat) = 첫 의존성.
            let primary_dep = pq
                .depends_on
                .first()
                .and_then(|d| key_to_id.get(d).copied());
            let id = qr.insert(NewQuest {
                campaign_id: Some(cid),
                project: &pq.project,
                brief: &pq.brief,
                branch: pq.branch.as_deref(),
                status: "pending",
                depends_on_quest_id: primary_dep,
                source_inmail_id: None,
            })?;
            // 모든 의존성을 quest_deps에 기록 (다중 의존 DAG).
            for dep_key in &pq.depends_on {
                let dep_id = *key_to_id
                    .get(dep_key)
                    .expect("위상순 보장 — 의존성이 먼저 insert됨");
                qr.add_dependency(id, dep_id)?;
            }
            key_to_id.insert(key.clone(), id);
        }
    }

    EventRepo::new(&tx).record(NewEvent {
        campaign_id: Some(cid),
        quest_id: None,
        actor: "luida",
        kind: "campaign_planned",
        payload: &json!({ "title": plan.title, "quests": plan.quests.len() }).to_string(),
    })?;
    tx.commit()?;

    Ok(cid)
}

/// 원정의 quest들을 의존성 순으로 실행 (현재 **순차** — 동시 한도는 후속 정책).
///
/// `ready_in_campaign`이 pending + 의존 완료 quest만 주므로, 디스패치 후 상태가
/// 바뀌면 다시 ready되지 않아 루프는 자연 종료한다. needs_input/failed quest의
/// 의존 quest는 영원히 ready되지 않아(차단) 루프가 끝난다.
pub fn run_campaign<F>(
    conn: &mut Connection,
    cfg: &AgentsConfig,
    campaign_id: i64,
    worktree: &dyn WorktreeProvider,
    runtime_factory: F,
) -> Result<CampaignRunReport>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    CampaignRepo::new(conn)
        .get(campaign_id)?
        .with_context(|| format!("campaign {campaign_id} 없음"))?;
    CampaignRepo::new(conn).set_status(campaign_id, "running")?;

    let mut report = CampaignRunReport::default();
    loop {
        let ready = QuestRepo::new(conn).ready_in_campaign(campaign_id)?;
        if ready.is_empty() {
            break;
        }
        for q in ready {
            let mut outcome = dispatch_quest(conn, cfg, q.id, worktree, &runtime_factory)?;
            // escalation → triage로 분류. 사소하면 자동 답변으로 재개(횟수 제한).
            let mut attempts = 0u32;
            while matches!(outcome, DispatchOutcome::NeedsInput { .. }) {
                let decision = triage_escalation(conn, cfg, q.id, &runtime_factory)?;
                if decision.ask_user || attempts >= MAX_AUTO_RESUME {
                    break;
                }
                let Some(answer) = decision.auto_answer.clone() else {
                    break;
                };
                attempts += 1;
                outcome = resume_quest(conn, cfg, q.id, &answer, &runtime_factory)?;
            }
            match outcome {
                DispatchOutcome::Completed { .. } => report.completed.push(q.id),
                DispatchOutcome::NeedsInput { .. } => {
                    // 자동 해소 실패 → 사용자에게 비방해 알림(멱등).
                    notify_user_escalation(conn, q.id)?;
                    report.needs_input.push(q.id);
                }
                DispatchOutcome::Failed { .. } => report.failed.push(q.id),
            }
        }
    }

    let all = QuestRepo::new(conn).list_for_campaign(campaign_id)?;
    report.all_completed = !all.is_empty() && all.iter().all(|q| q.status == "completed");

    let status = if !report.needs_input.is_empty() {
        "needs_input"
    } else if report.all_completed {
        // 완료 마감(completed)은 campaign.report 단계(Phase D)에서.
        "running"
    } else if !report.failed.is_empty() {
        "failed"
    } else {
        "running"
    };
    CampaignRepo::new(conn).set_status(campaign_id, status)?;

    Ok(report)
}

/// 플래너 LLM에 주는 프롬프트 — 등록 모험지 + 출력 스키마 규약.
fn build_plan_prompt(user_prompt: &str, projects: &HashSet<String>) -> String {
    let mut names: Vec<&str> = projects.iter().map(|s| s.as_str()).collect();
    names.sort();
    format!(
        "당신은 Luida의 원정 플래너입니다. 사용자 요청을 등록된 모험지(프로젝트)들에 걸친 \
quest DAG로 분해하세요.\n\n등록된 모험지: {}\n\n사용자 요청:\n{}\n\n\
아래 JSON 스키마로만 답하세요(설명 없이 JSON 객체):\n\
{{\"title\": string, \"quests\": [{{\"key\": string(고유), \"project\": string(등록된 모험지), \
\"brief\": string(수행 작업), \"depends_on\": [key...](선택), \"branch\": string(선택)}}]}}",
        names.join(", "),
        user_prompt
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use luida_core::agents::{AgentEvent, ScriptedRuntime};
    use luida_core::{migrate, open_memory};

    fn cfg() -> AgentsConfig {
        let json = r#"{
          "version": 1,
          "defaults": { "runtime": "claude", "tier": "simple" },
          "runtimes": {
            "claude": { "kind": "claude-cli", "command": "claude",
              "models": { "complex": "opus", "simple": "sonnet" } }
          },
          "actions": {
            "campaign.plan": { "runtime": "claude", "tier": "complex" },
            "quest.execute": { "runtime": "claude", "tier": "simple" }
          }
        }"#;
        AgentsConfig::from_json(json).unwrap()
    }

    fn setup() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        ProjectRepo::new(&conn).add("admin", "/b", "main", None).unwrap();
        conn
    }

    /// Result summary로 주어진 텍스트를 내는 런타임 factory.
    fn result_factory(summary: &str) -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        let summary = summary.to_string();
        move |_| {
            Ok(Box::new(ScriptedRuntime::new(vec![AgentEvent::Result {
                success: true,
                summary: Some(summary.clone()),
            }])) as Box<dyn AgentRuntime>)
        }
    }

    /// 항상 성공(result)하는 quest 실행 factory.
    fn success_factory() -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        |_| {
            Ok(Box::new(ScriptedRuntime::new(vec![AgentEvent::Result {
                success: true,
                summary: Some("done".into()),
            }])) as Box<dyn AgentRuntime>)
        }
    }

    struct FakeWorktree;
    impl WorktreeProvider for FakeWorktree {
        fn create(
            &self,
            _repo: &std::path::Path,
            codename: &str,
        ) -> Result<luida_sidecar::Worktree> {
            Ok(luida_sidecar::Worktree {
                branch: codename.to_string(),
                path: std::path::PathBuf::from("/tmp/x"),
            })
        }
    }

    const PLAN: &str = r#"{"title":"동기화","quests":[
      {"key":"a","project":"agora","brief":"스키마 변경"},
      {"key":"b","project":"admin","brief":"반영","depends_on":["a"]}]}"#;

    #[test]
    fn plan_campaign_persists_dag() {
        let mut conn = setup();
        let cid = plan_campaign(&mut conn, &cfg(), "agora→admin", result_factory(PLAN)).unwrap();
        let c = CampaignRepo::new(&conn).get(cid).unwrap().unwrap();
        assert_eq!(c.title, "동기화");
        assert_eq!(c.status, "planning");

        let quests = QuestRepo::new(&conn).list_for_campaign(cid).unwrap();
        assert_eq!(quests.len(), 2);
        let b = quests.iter().find(|q| q.project == "admin").unwrap();
        let a = quests.iter().find(|q| q.project == "agora").unwrap();
        // b는 a에 의존 (단일 대표 + quest_deps 둘 다)
        assert_eq!(b.depends_on_quest_id, Some(a.id));
        assert_eq!(QuestRepo::new(&conn).dependencies(b.id).unwrap(), vec![a.id]);
    }

    #[test]
    fn plan_campaign_rejects_unknown_project() {
        let mut conn = setup();
        let bad = r#"{"title":"t","quests":[{"key":"a","project":"ghost","brief":"x"}]}"#;
        assert!(plan_campaign(&mut conn, &cfg(), "p", result_factory(bad)).is_err());
    }

    #[test]
    fn plan_campaign_requires_projects() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        assert!(plan_campaign(&mut conn, &cfg(), "p", result_factory(PLAN)).is_err());
    }

    #[test]
    fn run_campaign_completes_all_in_order() {
        let mut conn = setup();
        let cid = plan_campaign(&mut conn, &cfg(), "p", result_factory(PLAN)).unwrap();
        let report = run_campaign(&mut conn, &cfg(), cid, &FakeWorktree, success_factory()).unwrap();
        assert_eq!(report.completed.len(), 2);
        assert!(report.needs_input.is_empty());
        assert!(report.all_completed);
        let quests = QuestRepo::new(&conn).list_for_campaign(cid).unwrap();
        assert!(quests.iter().all(|q| q.status == "completed"));
    }

    #[test]
    fn run_campaign_pauses_on_needs_input_when_triage_asks_user() {
        let mut conn = setup();
        let cid = plan_campaign(&mut conn, &cfg(), "p", result_factory(PLAN)).unwrap();
        // execute는 escalation, triage는 ask_user=true → 자동 재개 안 함, a에서 멈춤.
        let f = |r: &ResolvedAgent| -> Result<Box<dyn AgentRuntime>> {
            let events = if r.action == "escalation.triage" {
                vec![AgentEvent::Result {
                    success: true,
                    summary: Some(r#"{"ask_user": true, "reason": "위험"}"#.into()),
                }]
            } else {
                vec![AgentEvent::Escalation {
                    category: "design_mismatch".into(),
                    message: "어느 것?".into(),
                }]
            };
            Ok(Box::new(ScriptedRuntime::new(events)) as Box<dyn AgentRuntime>)
        };
        let report = run_campaign(&mut conn, &cfg(), cid, &FakeWorktree, f).unwrap();
        assert_eq!(report.needs_input.len(), 1);
        assert!(report.completed.is_empty());
        assert!(!report.all_completed);
        assert_eq!(
            CampaignRepo::new(&conn).get(cid).unwrap().unwrap().status,
            "needs_input"
        );
        // 사용자 알림이 발행됨
        assert_eq!(
            luida_core::InmailRepo::new(&conn).pending_for("@user").unwrap().len(),
            1
        );
        // b는 a 미완으로 pending 유지
        let quests = QuestRepo::new(&conn).list_for_campaign(cid).unwrap();
        assert!(quests.iter().any(|q| q.status == "pending"));
    }

    #[test]
    fn run_campaign_auto_resolves_trivial_escalation() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let mut conn = setup();
        let cid = plan_campaign(&mut conn, &cfg(), "p", result_factory(PLAN)).unwrap();
        // quest.execute: 첫 호출은 escalation, 이후 success. triage: auto_answer 제공.
        let exec_calls = Arc::new(AtomicU32::new(0));
        let f = move |r: &ResolvedAgent| -> Result<Box<dyn AgentRuntime>> {
            let events = if r.action == "escalation.triage" {
                vec![AgentEvent::Result {
                    success: true,
                    summary: Some(r#"{"ask_user": false, "auto_answer": "옵션 A로", "reason": "사소"}"#.into()),
                }]
            } else if exec_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                vec![AgentEvent::Escalation {
                    category: "ambiguous_spec".into(),
                    message: "어느 것?".into(),
                }]
            } else {
                vec![AgentEvent::Result {
                    success: true,
                    summary: Some("done".into()),
                }]
            };
            Ok(Box::new(ScriptedRuntime::new(events)) as Box<dyn AgentRuntime>)
        };
        let report = run_campaign(&mut conn, &cfg(), cid, &FakeWorktree, f).unwrap();
        // a는 escalation→triage(auto)→resume→완료, b도 완료
        assert_eq!(report.completed.len(), 2);
        assert!(report.needs_input.is_empty());
        assert!(report.all_completed);
    }
}
