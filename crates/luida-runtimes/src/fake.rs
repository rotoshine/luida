//! FakeRuntime — 외부 LLM 없이 동작을 시연/테스트하는 결정적 런타임 (데모·CI용).
//!
//! action별로 그럴듯한 canned stream-json 이벤트를 낸다. `campaign.plan`/`learning.reflect`는
//! 프롬프트에 주입된 "등록된 모험지: ..." 목록을 파싱해 실제 프로젝트로 계획/제안을 만든다.

use anyhow::Result;
use luida_core::agents::{
    finalize_outcome, fold_outcome, AgentEvent, AgentInvocation, AgentOutcome, AgentRuntime,
};
use serde_json::json;

/// action을 알고 있는 가짜 런타임.
pub struct FakeRuntime {
    action: String,
}

impl FakeRuntime {
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
        }
    }
}

impl AgentRuntime for FakeRuntime {
    fn run(
        &self,
        _model: &str,
        inv: &AgentInvocation,
        on_event: &mut dyn FnMut(&AgentEvent),
    ) -> Result<AgentOutcome> {
        let events = canned_events(&self.action, &inv.prompt);
        let mut outcome = AgentOutcome::default();
        for e in &events {
            fold_outcome(&mut outcome, e);
            on_event(e);
        }
        Ok(finalize_outcome(outcome, true))
    }
}

/// action 문자열 → FakeRuntime 박스.
pub fn fake_runtime_for(action: &str) -> Box<dyn AgentRuntime> {
    Box::new(FakeRuntime::new(action))
}

fn canned_events(action: &str, prompt: &str) -> Vec<AgentEvent> {
    match action {
        "campaign.plan" => {
            let projects = parse_projects(prompt);
            let plan = build_demo_plan(&projects);
            vec![AgentEvent::Result {
                success: true,
                summary: Some(plan),
            }]
        }
        "quest.execute" => vec![
            AgentEvent::Text {
                text: "[데모] 작업을 수행합니다".into(),
            },
            AgentEvent::ToolUse { name: "Edit".into() },
            AgentEvent::Result {
                success: true,
                summary: Some("[데모] 작업 완료".into()),
            },
        ],
        "campaign.report" => vec![AgentEvent::Result {
            success: true,
            summary: Some("# 원정 보고서 (데모)\n\n모든 모험을 완료했습니다.".into()),
        }],
        "project.ingest" => vec![AgentEvent::Result {
            success: true,
            summary: Some("## 프로젝트 맥락 (데모)\n\n데모용 자동 요약입니다.".into()),
        }],
        "escalation.triage" => vec![AgentEvent::Result {
            success: true,
            summary: Some(
                json!({ "ask_user": false, "auto_answer": "기본값으로 진행", "reason": "데모 자동 해소" })
                    .to_string(),
            ),
        }],
        "learning.reflect" => {
            let projects = parse_projects(prompt);
            vec![AgentEvent::Result {
                success: true,
                summary: Some(build_demo_reflection(&projects)),
            }]
        }
        _ => vec![AgentEvent::Result {
            success: true,
            summary: Some("[데모] ok".into()),
        }],
    }
}

/// 프롬프트의 "등록된 모험지: a, b, c" 줄에서 프로젝트 이름 추출.
fn parse_projects(prompt: &str) -> Vec<String> {
    for line in prompt.lines() {
        if let Some(rest) = line.split_once("등록된 모험지:") {
            return rest
                .1
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    Vec::new()
}

/// 프로젝트별 quest 1개를 체인(앞→뒤 의존)으로 엮은 데모 계획.
fn build_demo_plan(projects: &[String]) -> String {
    let mut quests = Vec::new();
    for (i, p) in projects.iter().enumerate() {
        let mut q = json!({
            "key": format!("q{i}"),
            "project": p,
            "brief": format!("[데모] {p} 작업 수행"),
        });
        if i > 0 {
            q["depends_on"] = json!([format!("q{}", i - 1)]);
        }
        quests.push(q);
    }
    // 프로젝트가 없으면 빈 계획(검증에서 거부됨) 방지용으로 그대로 둠
    json!({ "title": "데모 원정", "quests": quests }).to_string()
}

/// 앞 두 프로젝트 사이의 관계 제안(데모).
fn build_demo_reflection(projects: &[String]) -> String {
    let rels = if projects.len() >= 2 {
        json!([{
            "name": "demo-link",
            "from_project": projects[0],
            "trigger_kind": "quest_completed",
            "to_project": projects[1],
            "action": "propose",
            "confidence": 0.5,
        }])
    } else {
        json!([])
    };
    json!({ "patterns": ["[데모] 반복 패턴 감지"], "relationships": rels }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_parses_projects_from_prompt() {
        let prompt = "...\n등록된 모험지: agora, admin\n사용자 요청:\n...";
        let evs = canned_events("campaign.plan", prompt);
        let summary = match &evs[0] {
            AgentEvent::Result { summary, .. } => summary.clone().unwrap(),
            _ => panic!(),
        };
        assert!(summary.contains("agora"));
        assert!(summary.contains("admin"));
        assert!(summary.contains("depends_on")); // 두 번째 quest가 첫째에 의존
    }

    #[test]
    fn quest_execute_succeeds() {
        let rt = FakeRuntime::new("quest.execute");
        let mut n = 0;
        let out = rt
            .run("m", &AgentInvocation::default(), &mut |_| n += 1)
            .unwrap();
        assert!(out.success);
        assert!(n >= 2);
    }

    #[test]
    fn triage_auto_resolves() {
        let evs = canned_events("escalation.triage", "");
        let s = match &evs[0] {
            AgentEvent::Result { summary, .. } => summary.clone().unwrap(),
            _ => panic!(),
        };
        assert!(s.contains("ask_user"));
        assert!(s.contains("false"));
    }

    #[test]
    fn reflect_proposes_relationship() {
        let evs = canned_events("learning.reflect", "등록된 모험지: agora, admin");
        let s = match &evs[0] {
            AgentEvent::Result { summary, .. } => summary.clone().unwrap(),
            _ => panic!(),
        };
        assert!(s.contains("demo-link"));
        assert!(s.contains("agora"));
    }

    #[test]
    fn unknown_action_ok() {
        let evs = canned_events("whatever", "");
        assert!(matches!(evs[0], AgentEvent::Result { success: true, .. }));
    }
}
