//! learning.reflect (spec §3.2) — 이벤트 분석 → 프로젝트 관계 제안.
//!
//! 최근 events를 LLM에 주고 자동화 가능한 프로젝트 간 관계(relationship)를 제안받아,
//! 검증 후 **비활성(enabled=false) 제안**으로 저장(사람 검토 후 활성화). source=learned-promoted.

use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;

use luida_core::agents::{AgentInvocation, AgentRuntime, ResolvedAgent};
use luida_core::models::{RELATIONSHIP_ACTIONS, RELATIONSHIP_TRIGGERS};
use luida_core::{
    resolve, AgentsConfig, Connection, EventRepo, NewEvent, NewRelationship, ProjectRepo,
    RelationshipRepo,
};

/// reflect 결과 요약.
#[derive(Debug, Default, PartialEq)]
pub struct ReflectReport {
    pub proposals_inserted: usize,
    pub proposals_skipped: usize,
    pub patterns: Vec<String>,
}

#[derive(Deserialize)]
struct ReflectOutput {
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    relationships: Vec<RelProposal>,
}

#[derive(Deserialize)]
struct RelProposal {
    name: Option<String>,
    from_project: String,
    trigger_kind: String,
    to_project: String,
    action: String,
    #[serde(default)]
    trigger_config: Option<String>,
    #[serde(default)]
    brief_template: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
}

/// 최근 `since_ms` 이후 이벤트를 분석해 관계 제안을 저장한다.
pub fn reflect<F>(
    conn: &mut Connection,
    cfg: &AgentsConfig,
    since_ms: i64,
    runtime_factory: F,
) -> Result<ReflectReport>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    let known: HashSet<String> = ProjectRepo::new(conn)
        .list()?
        .into_iter()
        .map(|p| p.name)
        .collect();
    let events = EventRepo::new(conn).recent_since(since_ms, 200)?;

    let resolved = resolve(cfg, "learning.reflect", None)?;
    let inv = AgentInvocation {
        prompt: build_reflect_prompt(&events, &known),
        ..Default::default()
    };
    let runtime = runtime_factory(&resolved).context("reflect 런타임 생성 실패")?;
    let outcome = runtime.run(&resolved.model, &inv, &mut |_| {})?;
    let raw = outcome.summary.context("reflect가 결과(JSON)를 반환하지 않음")?;
    let parsed = parse_reflect(&raw)?;

    let mut report = ReflectReport {
        patterns: parsed.patterns,
        ..Default::default()
    };

    let tx = conn.transaction()?;
    {
        let rrepo = RelationshipRepo::new(&tx);
        for p in &parsed.relationships {
            // 검증: 등록 프로젝트 + trigger/action enum. 미달이면 skip(저장 안 함).
            let valid = known.contains(&p.from_project)
                && known.contains(&p.to_project)
                && RELATIONSHIP_TRIGGERS.contains(&p.trigger_kind.as_str())
                && RELATIONSHIP_ACTIONS.contains(&p.action.as_str());
            if !valid {
                report.proposals_skipped += 1;
                continue;
            }
            // 학습 제안은 기본 비활성 — 사람이 검토 후 활성화.
            rrepo.upsert_by_name(NewRelationship {
                name: p.name.as_deref(),
                from_project: &p.from_project,
                trigger_kind: &p.trigger_kind,
                trigger_config: p.trigger_config.as_deref().unwrap_or("{}"),
                to_project: &p.to_project,
                action: &p.action,
                brief_template: p.brief_template.as_deref(),
                enabled: false,
                source: "learned-promoted",
                confidence: p.confidence,
            })?;
            report.proposals_inserted += 1;
        }
        EventRepo::new(&tx).record(NewEvent {
            campaign_id: None,
            quest_id: None,
            actor: "brain",
            kind: "reflected",
            payload: &json!({
                "inserted": report.proposals_inserted,
                "skipped": report.proposals_skipped,
                "patterns": report.patterns.len(),
            })
            .to_string(),
        })?;
    }
    tx.commit()?;

    Ok(report)
}

fn parse_reflect(raw: &str) -> Result<ReflectOutput> {
    let start = raw.find('{').context("reflect JSON 없음")?;
    let end = raw.rfind('}').context("reflect JSON 없음")?;
    if end <= start {
        bail!("reflect JSON 형식 오류");
    }
    Ok(serde_json::from_str(&raw[start..=end])?)
}

fn build_reflect_prompt(events: &[luida_core::Event], projects: &HashSet<String>) -> String {
    let mut names: Vec<&str> = projects.iter().map(|s| s.as_str()).collect();
    names.sort();

    let mut ev_lines = String::new();
    for e in events.iter().take(80) {
        ev_lines.push_str(&format!("- {} / {} / {}\n", e.actor, e.kind, e.payload));
    }
    if ev_lines.is_empty() {
        ev_lines.push_str("(최근 이벤트 없음)\n");
    }

    format!(
        "당신은 Luida의 학습기(brain)입니다. 아래 이벤트를 분석해 반복되는 패턴과 자동화 가능한 \
프로젝트 간 관계를 찾으세요.\n\n등록된 모험지: {}\n\n최근 이벤트:\n{}\n\
아래 JSON으로만 답하세요(설명 없이):\n\
{{\"patterns\": [string...], \"relationships\": [{{\"name\": string, \"from_project\": string, \
\"trigger_kind\": \"path_changed|quest_completed|tag_pushed\", \"to_project\": string, \
\"action\": \"auto_dispatch|propose\", \"trigger_config\": string(JSON), \"brief_template\": string, \
\"confidence\": number}}]}}\n\
- 등록된 모험지 간 관계만. 확실치 않으면 confidence를 낮게.",
        names.join(", "),
        ev_lines
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use luida_core::agents::{AgentEvent, ScriptedRuntime};
    use luida_core::{migrate, open_memory, NewEvent};

    fn cfg() -> AgentsConfig {
        let json = r#"{
          "version": 1,
          "defaults": { "runtime": "claude", "tier": "simple" },
          "runtimes": {
            "claude": { "kind": "claude-cli", "command": "claude",
              "models": { "complex": "opus", "simple": "sonnet" } }
          },
          "actions": { "learning.reflect": { "runtime": "claude", "tier": "complex" } }
        }"#;
        AgentsConfig::from_json(json).unwrap()
    }

    fn result_factory(s: &str) -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
        let s = s.to_string();
        move |_| {
            Ok(Box::new(ScriptedRuntime::new(vec![AgentEvent::Result {
                success: true,
                summary: Some(s.clone()),
            }])) as Box<dyn AgentRuntime>)
        }
    }

    fn setup() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        ProjectRepo::new(&conn).add("admin", "/b", "main", None).unwrap();
        // 이벤트 몇 개
        EventRepo::new(&conn)
            .record(NewEvent {
                campaign_id: None,
                quest_id: None,
                actor: "agora",
                kind: "quest_completed",
                payload: "{}",
            })
            .unwrap();
        conn
    }

    #[test]
    fn reflect_inserts_valid_proposal_as_disabled() {
        let mut conn = setup();
        let out = r#"{"patterns":["agora 변경이 admin에 반영됨"],
          "relationships":[{"name":"agora-to-admin","from_project":"agora",
            "trigger_kind":"quest_completed","to_project":"admin","action":"propose",
            "confidence":0.7}]}"#;
        let report = reflect(&mut conn, &cfg(), 0, result_factory(out)).unwrap();
        assert_eq!(report.proposals_inserted, 1);
        assert_eq!(report.patterns.len(), 1);

        let r = RelationshipRepo::new(&conn).find_by_name("agora-to-admin").unwrap().unwrap();
        assert_eq!(r.from_project, "agora");
        assert_eq!(r.to_project, "admin");
        assert!(!r.is_enabled()); // 제안은 비활성
        assert_eq!(r.source, "learned-promoted");
        // 비활성이라 list_enabled엔 안 나옴
        assert_eq!(RelationshipRepo::new(&conn).list_enabled().unwrap().len(), 0);
    }

    #[test]
    fn reflect_skips_invalid_proposals() {
        let mut conn = setup();
        let out = r#"{"relationships":[
            {"name":"a","from_project":"ghost","trigger_kind":"quest_completed","to_project":"admin","action":"propose"},
            {"name":"b","from_project":"agora","trigger_kind":"bogus","to_project":"admin","action":"propose"},
            {"name":"c","from_project":"agora","trigger_kind":"quest_completed","to_project":"admin","action":"send_troops"}
          ]}"#;
        let report = reflect(&mut conn, &cfg(), 0, result_factory(out)).unwrap();
        assert_eq!(report.proposals_inserted, 0);
        assert_eq!(report.proposals_skipped, 3);
    }

    #[test]
    fn reflect_handles_no_relationships() {
        let mut conn = setup();
        let report = reflect(&mut conn, &cfg(), 0, result_factory(r#"{"patterns":[]}"#)).unwrap();
        assert_eq!(report.proposals_inserted, 0);
    }

    #[test]
    fn reflect_rejects_garbage_output() {
        let mut conn = setup();
        assert!(reflect(&mut conn, &cfg(), 0, result_factory("no json")).is_err());
    }
}
