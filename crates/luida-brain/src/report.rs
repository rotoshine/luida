//! 원정 완료 보고 — `campaign.report`(LLM) → 모험의 서 기록 (spec §7.5).

use anyhow::{Context, Result};
use serde_json::json;

use luida_core::agents::{AgentInvocation, AgentRuntime, ResolvedAgent};
use luida_core::{
    resolve, AgentsConfig, CampaignRepo, Connection, EventRepo, NewEvent, Quest, QuestRepo,
};

use crate::memory::{sanitize_filename, MemoryVault};

/// 원정 보고서를 생성·기록하고 campaign.report_path를 채운다.
///
/// 모든 quest가 completed면 campaign을 `completed`로 마감, 아니면 report_path만 기록(사후 보고).
/// 기록 경로를 반환.
pub fn report_campaign<F>(
    conn: &mut Connection,
    cfg: &AgentsConfig,
    campaign_id: i64,
    vault: &MemoryVault,
    runtime_factory: F,
) -> Result<std::path::PathBuf>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    let campaign = CampaignRepo::new(conn)
        .get(campaign_id)?
        .with_context(|| format!("campaign {campaign_id} 없음"))?;
    let quests = QuestRepo::new(conn).list_for_campaign(campaign_id)?;
    let all_completed = !quests.is_empty() && quests.iter().all(|q| q.status == "completed");

    let resolved = resolve(cfg, "campaign.report", None)?;
    let inv = AgentInvocation {
        prompt: build_report_prompt(&campaign.title, &campaign.prompt, &quests),
        ..Default::default()
    };
    let runtime = runtime_factory(&resolved).context("report 런타임 생성 실패")?;
    let outcome = runtime.run(&resolved.model, &inv, &mut |_| {})?;
    let body = outcome
        .summary
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| fallback_report(&campaign.title, &quests));

    // ── vault 기록 ───────────────────────────────────────────────────────────────
    let slug = sanitize_filename(&campaign.title);
    let frontmatter = format!(
        "---\ntype: campaign-report\ncampaign_id: {}\nstatus: {}\nquests: {}\ncompleted: {}\n---",
        campaign_id,
        campaign.status,
        quests.len(),
        all_completed
    );
    let path = vault.write_campaign_report(campaign_id, &slug, &frontmatter, &body)?;
    let report_link = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("report");
    vault.append_chronicle(&format!(
        "- [[{}]] {} — quest {}건 ({})",
        report_link,
        campaign.title,
        quests.len(),
        if all_completed { "완료" } else { &campaign.status }
    ))?;

    // ── DB 반영 (원자) ───────────────────────────────────────────────────────────
    let path_str = path.to_string_lossy().to_string();
    let tx = conn.transaction()?;
    if all_completed {
        CampaignRepo::new(&tx).mark_completed(campaign_id, Some(&path_str))?;
    } else {
        CampaignRepo::new(&tx).set_report_path(campaign_id, &path_str)?;
    }
    EventRepo::new(&tx).record(NewEvent {
        campaign_id: Some(campaign_id),
        quest_id: None,
        actor: "luida",
        kind: "campaign_reported",
        payload: &json!({ "report_path": path_str, "all_completed": all_completed }).to_string(),
    })?;
    tx.commit()?;

    Ok(path)
}

fn build_report_prompt(title: &str, prompt: &str, quests: &[Quest]) -> String {
    let mut lines = String::new();
    for q in quests {
        lines.push_str(&format!(
            "- [{}] {} ({}): {}\n",
            q.status,
            q.project,
            q.branch.as_deref().unwrap_or("-"),
            q.brief
        ));
    }
    format!(
        "당신은 Luida의 원정 기록자입니다. 완료된 원정을 '모험의 서'에 남길 보고서로 정리하세요.\n\n\
원정: {title}\n원래 요청: {prompt}\n\nquest 목록:\n{lines}\n\
간결한 Markdown 보고서(요약·각 모험지 변경·후속 제안)를 작성하세요. 본문만 출력."
    )
}

/// LLM이 보고서를 안 주면 quest 목록으로 기본 보고서 생성.
fn fallback_report(title: &str, quests: &[Quest]) -> String {
    let mut s = format!("# {title}\n\n## 모험 요약\n\n");
    for q in quests {
        s.push_str(&format!("- **{}** ({}): {}\n", q.project, q.status, q.brief));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryVault;
    use luida_core::agents::{AgentEvent, ScriptedRuntime};
    use luida_core::{migrate, open_memory, NewCampaign, NewQuest, ProjectRepo};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_vault() -> MemoryVault {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        MemoryVault::new(
            std::env::temp_dir().join(format!("luida-report-test-{}-{}", std::process::id(), seq)),
        )
    }

    fn cfg() -> AgentsConfig {
        let json = r#"{
          "version": 1,
          "defaults": { "runtime": "claude", "tier": "simple" },
          "runtimes": {
            "claude": { "kind": "claude-cli", "command": "claude",
              "models": { "complex": "opus", "simple": "sonnet" } }
          },
          "actions": { "campaign.report": { "runtime": "claude", "tier": "simple" } }
        }"#;
        AgentsConfig::from_json(json).unwrap()
    }

    fn report_factory(body: &str) -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        let b = body.to_string();
        move |_| {
            Ok(Box::new(ScriptedRuntime::new(vec![AgentEvent::Result {
                success: true,
                summary: Some(b.clone()),
            }])) as Box<dyn AgentRuntime>)
        }
    }

    /// campaign + quest 1건(상태 지정) 셋업.
    fn setup(quest_status: &str) -> (Connection, i64) {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign {
                title: "스키마 동기화",
                prompt: "agora→admin",
                plan_json: "{}",
                status: "running",
            })
            .unwrap();
        let qid = QuestRepo::new(&conn)
            .insert(NewQuest {
                campaign_id: Some(cid),
                project: "agora",
                brief: "작업",
                branch: None,
                status: "pending",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        if quest_status == "completed" {
            QuestRepo::new(&conn).mark_completed(qid, None).unwrap();
        } else {
            QuestRepo::new(&conn).set_status(qid, quest_status).unwrap();
        }
        (conn, cid)
    }

    #[test]
    fn report_completes_when_all_quests_done() {
        let (mut conn, cid) = setup("completed");
        let vault = temp_vault();
        let path = report_campaign(&mut conn, &cfg(), cid, &vault, report_factory("# 보고서\n끝")).unwrap();
        // 파일 기록
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("보고서"));
        assert!(content.contains("type: campaign-report"));
        // campaign 완료 + report_path
        let c = CampaignRepo::new(&conn).get(cid).unwrap().unwrap();
        assert_eq!(c.status, "completed");
        assert!(c.report_path.is_some());
        // chronicle append
        let chron = std::fs::read_to_string(vault.base().join("chronicle.md")).unwrap();
        assert!(chron.contains("스키마 동기화"));
    }

    #[test]
    fn report_does_not_complete_when_quest_failed() {
        let (mut conn, cid) = setup("failed");
        let vault = temp_vault();
        report_campaign(&mut conn, &cfg(), cid, &vault, report_factory("post-mortem")).unwrap();
        let c = CampaignRepo::new(&conn).get(cid).unwrap().unwrap();
        assert_ne!(c.status, "completed"); // 마감 안 함
        assert!(c.report_path.is_some()); // 보고서는 기록
    }

    #[test]
    fn report_uses_fallback_when_no_summary() {
        let (mut conn, cid) = setup("completed");
        let vault = temp_vault();
        // result에 summary 없음 → fallback 보고서
        let empty_factory = |_: &ResolvedAgent| {
            Ok(Box::new(ScriptedRuntime::new(vec![AgentEvent::Result {
                success: true,
                summary: None,
            }])) as Box<dyn AgentRuntime>)
        };
        let path = report_campaign(&mut conn, &cfg(), cid, &vault, empty_factory).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("모험 요약")); // fallback 헤더
    }
}
