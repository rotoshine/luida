//! 관계(relationship) 트리거 평가 (spec §7.3 자동화) — 학습 루프의 실행 고리.
//!
//! quest 완료 시 from_project가 일치하는 **활성** 관계를 평가:
//!  - `auto_dispatch`: to_project에 pending quest 생성(같은 campaign → run_campaign이 픽업)
//!  - `propose`: @user에게 proposal inmail (사람이 결정)

use anyhow::{Context, Result};
use serde_json::json;

use luida_core::{
    Connection, EventRepo, InmailRepo, NewEvent, NewInmail, NewQuest, QuestRepo, RelationshipRepo,
};

/// 트리거 평가 결과.
#[derive(Debug, Default, PartialEq)]
pub struct TriggerResult {
    /// auto_dispatch로 새로 만든 quest id들.
    pub dispatched: Vec<i64>,
    /// propose로 보낸 제안 수.
    pub proposals: usize,
}

/// 완료된 quest에 대해 `quest_completed` 트리거 관계를 평가·실행한다.
pub fn fire_quest_completed(conn: &mut Connection, quest_id: i64) -> Result<TriggerResult> {
    let quest = QuestRepo::new(conn)
        .get(quest_id)?
        .with_context(|| format!("quest {quest_id} 없음"))?;

    // from_project 일치 + enabled (list_by_from은 enabled만).
    let rels: Vec<_> = RelationshipRepo::new(conn)
        .list_by_from(&quest.project)?
        .into_iter()
        .filter(|r| r.trigger_kind == "quest_completed")
        .collect();

    let mut result = TriggerResult::default();
    for r in &rels {
        let brief = render_template(r.brief_template.as_deref(), &quest.project, &r.to_project, &quest.brief);
        match r.action.as_str() {
            "auto_dispatch" => {
                let new_id = QuestRepo::new(conn).insert(NewQuest {
                    campaign_id: quest.campaign_id,
                    project: &r.to_project,
                    brief: &brief,
                    branch: None,
                    status: "pending",
                    depends_on_quest_id: None,
                    source_inmail_id: None,
                })?;
                EventRepo::new(conn).record(NewEvent {
                    campaign_id: quest.campaign_id,
                    quest_id: Some(new_id),
                    actor: "luida",
                    kind: "trigger_dispatched",
                    payload: &json!({
                        "from_quest": quest_id,
                        "relationship": r.name,
                        "to_project": r.to_project,
                    })
                    .to_string(),
                })?;
                result.dispatched.push(new_id);
            }
            "propose" => {
                let dedupe = r
                    .name
                    .as_deref()
                    .map(|n| format!("propose-{n}-q{quest_id}"));
                InmailRepo::new(conn).enqueue(NewInmail {
                    from_session: "luida",
                    to_session: "@user",
                    kind: "proposal",
                    payload: &json!({
                        "from_project": quest.project,
                        "to_project": r.to_project,
                        "relationship": r.name,
                        "brief": brief,
                    })
                    .to_string(),
                    reply_to: None,
                    quest_id: Some(quest_id),
                    campaign_id: quest.campaign_id,
                    dedupe_key: dedupe.as_deref(),
                })?;
                result.proposals += 1;
            }
            _ => {}
        }
    }
    Ok(result)
}

/// brief_template 렌더 — `{from_project}`/`{to_project}`/`{brief}` 치환. 없으면 기본 문구.
fn render_template(template: Option<&str>, from: &str, to: &str, brief: &str) -> String {
    match template {
        Some(t) => t
            .replace("{from_project}", from)
            .replace("{to_project}", to)
            .replace("{brief}", brief),
        None => format!("{from}의 '{brief}' 완료 → {to}에 반영"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luida_core::{
        migrate, open_memory, NewRelationship, ProjectRepo,
    };

    fn setup() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        ProjectRepo::new(&conn).add("admin", "/b", "main", None).unwrap();
        conn
    }

    fn completed_quest(conn: &Connection, project: &str) -> i64 {
        let id = QuestRepo::new(conn)
            .insert(NewQuest {
                campaign_id: None,
                project,
                brief: "스키마 변경",
                branch: None,
                status: "pending",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        QuestRepo::new(conn).mark_completed(id, None).unwrap();
        id
    }

    fn add_rel(conn: &Connection, name: &str, from: &str, to: &str, action: &str, tmpl: Option<&str>) {
        RelationshipRepo::new(conn)
            .insert(NewRelationship {
                name: Some(name),
                from_project: from,
                trigger_kind: "quest_completed",
                trigger_config: "{}",
                to_project: to,
                action,
                brief_template: tmpl,
                enabled: true,
                source: "human",
                confidence: None,
            })
            .unwrap();
    }

    #[test]
    fn auto_dispatch_creates_followup_quest() {
        let mut conn = setup();
        add_rel(&conn, "agora-admin", "agora", "admin", "auto_dispatch", Some("{from_project}→{to_project}: {brief}"));
        let qid = completed_quest(&conn, "agora");
        let result = fire_quest_completed(&mut conn, qid).unwrap();
        assert_eq!(result.dispatched.len(), 1);
        let new_q = QuestRepo::new(&conn).get(result.dispatched[0]).unwrap().unwrap();
        assert_eq!(new_q.project, "admin");
        assert_eq!(new_q.status, "pending");
        assert_eq!(new_q.brief, "agora→admin: 스키마 변경");
    }

    #[test]
    fn propose_sends_user_inmail() {
        let mut conn = setup();
        add_rel(&conn, "agora-admin", "agora", "admin", "propose", None);
        let qid = completed_quest(&conn, "agora");
        let result = fire_quest_completed(&mut conn, qid).unwrap();
        assert_eq!(result.proposals, 1);
        assert!(result.dispatched.is_empty());
        let mail = InmailRepo::new(&conn).pending_for("@user").unwrap();
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].kind, "proposal");
    }

    #[test]
    fn disabled_relationship_does_not_fire() {
        let mut conn = setup();
        RelationshipRepo::new(&conn)
            .insert(NewRelationship {
                name: Some("off"),
                from_project: "agora",
                trigger_kind: "quest_completed",
                trigger_config: "{}",
                to_project: "admin",
                action: "auto_dispatch",
                brief_template: None,
                enabled: false,
                source: "learned-promoted",
                confidence: None,
            })
            .unwrap();
        let qid = completed_quest(&conn, "agora");
        let result = fire_quest_completed(&mut conn, qid).unwrap();
        assert!(result.dispatched.is_empty());
    }

    #[test]
    fn no_matching_relationship_is_noop() {
        let mut conn = setup();
        add_rel(&conn, "x", "admin", "agora", "auto_dispatch", None); // from=admin
        let qid = completed_quest(&conn, "agora"); // 완료는 agora
        let result = fire_quest_completed(&mut conn, qid).unwrap();
        assert_eq!(result, TriggerResult::default());
    }

    #[test]
    fn default_template_when_none() {
        assert_eq!(
            render_template(None, "agora", "admin", "스키마"),
            "agora의 '스키마' 완료 → admin에 반영"
        );
    }
}
