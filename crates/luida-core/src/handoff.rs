//! 모험 중단·재개 (Suspend/Resume, spec §14).
//!
//! campaign+quests를 `HandoffBundle`(JSON)로 봉인해 다른 기기로 운반 → 재개.
//! single owner 잠금으로 한 시점에 한 기기만 active.
//!
//! **데이터 번들만 운반**(spec §14.7) — tavern.db 직접 동기화 금지(WAL 손상 위험).
//! 미커밋 코드 파일의 git patch transport는 별도(실환경) 계층; 여기서는 진행상태 번들.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::db::now_ms;
use crate::repo::{CampaignRepo, NewCampaign, NewQuest, QuestRepo};
use rusqlite::Connection;

/// 봉인된 원정 상태 (운반 단위).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffBundle {
    pub schema: u32,
    pub origin_machine: String,
    pub exported_at: i64,
    pub campaign: CampaignSnapshot,
    pub quests: Vec<QuestSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CampaignSnapshot {
    pub title: String,
    pub prompt: String,
    pub plan_json: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestSnapshot {
    pub project: String,
    pub brief: String,
    pub branch: Option<String>,
    pub status: String,
    pub progress: Option<String>,
    /// 의존성을 bundle.quests 내 인덱스로 표현 (id 비의존).
    pub depends_on: Vec<usize>,
}

const BUNDLE_SCHEMA: u32 = 1;

impl HandoffBundle {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(s: &str) -> Result<Self> {
        let b: HandoffBundle = serde_json::from_str(s)?;
        if b.schema != BUNDLE_SCHEMA {
            bail!("지원하지 않는 번들 schema: {}", b.schema);
        }
        Ok(b)
    }
}

/// 이 기기의 식별자. `LUIDA_MACHINE_ID` env > `HOSTNAME` env > "local".
pub fn machine_id() -> String {
    for key in ["LUIDA_MACHINE_ID", "HOSTNAME"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                return v;
            }
        }
    }
    "local".to_string()
}

/// 원정을 중단(suspend)하고 봉인 번들 생성. campaign을 suspended+owner로 표시.
///
/// 다른 기기가 이미 suspended로 소유 중이면 거부(force로 강제).
pub fn suspend_campaign(
    conn: &Connection,
    campaign_id: i64,
    machine: &str,
    force: bool,
) -> Result<HandoffBundle> {
    let campaign = CampaignRepo::new(conn)
        .get(campaign_id)?
        .with_context(|| format!("campaign {campaign_id} 없음"))?;

    if !force
        && campaign.handoff_state == "suspended"
        && campaign.owner_machine.as_deref() != Some(machine)
    {
        bail!(
            "원정 #{campaign_id}은 다른 기기({:?})에서 이미 중단됨. --force로 강제.",
            campaign.owner_machine
        );
    }

    let quests = QuestRepo::new(conn).list_for_campaign(campaign_id)?;
    let id_to_pos: HashMap<i64, usize> =
        quests.iter().enumerate().map(|(i, q)| (q.id, i)).collect();

    let qrepo = QuestRepo::new(conn);
    let mut snaps = Vec::with_capacity(quests.len());
    for q in &quests {
        let depends_on = qrepo
            .dependencies(q.id)?
            .iter()
            .filter_map(|d| id_to_pos.get(d).copied())
            .collect();
        snaps.push(QuestSnapshot {
            project: q.project.clone(),
            brief: q.brief.clone(),
            branch: q.branch.clone(),
            status: q.status.clone(),
            progress: q.progress.clone(),
            depends_on,
        });
    }

    CampaignRepo::new(conn).set_handoff(campaign_id, "suspended", Some(machine))?;

    Ok(HandoffBundle {
        schema: BUNDLE_SCHEMA,
        origin_machine: machine.to_string(),
        exported_at: now_ms(),
        campaign: CampaignSnapshot {
            title: campaign.title,
            prompt: campaign.prompt,
            plan_json: campaign.plan_json,
            status: campaign.status,
        },
        quests: snaps,
    })
}

/// 번들을 새 원정으로 재개(import). id 재매핑, in-flight(running/needs_input)는 pending 리셋.
/// 새 campaign id 반환. handoff_state=resumed, owner=machine.
pub fn resume_bundle(conn: &mut Connection, bundle: &HandoffBundle, machine: &str) -> Result<i64> {
    let tx = conn.transaction()?;
    let cid = CampaignRepo::new(&tx).insert(NewCampaign {
        title: &bundle.campaign.title,
        prompt: &bundle.campaign.prompt,
        plan_json: &bundle.campaign.plan_json,
        status: "running",
    })?;

    let qrepo = QuestRepo::new(&tx);
    let mut pos_to_id = vec![0i64; bundle.quests.len()];
    for (i, q) in bundle.quests.iter().enumerate() {
        let status = resume_status(&q.status);
        // worktree는 재개 기기에서 새로 만들므로 branch/progress는 안 옮김(pending이면 무의미).
        let branch = if status == "pending" { None } else { q.branch.as_deref() };
        let id = qrepo.insert(NewQuest {
            campaign_id: Some(cid),
            project: &q.project,
            brief: &q.brief,
            branch,
            status,
            depends_on_quest_id: None,
            source_inmail_id: None,
        })?;
        pos_to_id[i] = id;
    }
    // 의존성 재구성 (모든 quest insert 후 — 인덱스 안전).
    for (i, q) in bundle.quests.iter().enumerate() {
        for &dep_pos in &q.depends_on {
            let dep_id = *pos_to_id
                .get(dep_pos)
                .with_context(|| format!("번들 의존 인덱스 범위 초과: {dep_pos}"))?;
            qrepo.add_dependency(pos_to_id[i], dep_id)?;
        }
    }

    CampaignRepo::new(&tx).set_handoff(cid, "resumed", Some(machine))?;
    tx.commit()?;
    Ok(cid)
}

/// 재개 시 quest 상태: 종료 상태는 보존, in-flight(running/needs_input 등)는 pending으로 리셋.
fn resume_status(s: &str) -> &'static str {
    match s {
        "completed" => "completed",
        "failed" => "failed",
        "aborted" => "aborted",
        _ => "pending",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate, open_memory};
    use crate::repo::{ProjectRepo, QuestRepo};

    fn setup() -> (Connection, i64) {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        ProjectRepo::new(&conn).add("admin", "/b", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign {
                title: "동기화",
                prompt: "agora→admin",
                plan_json: "{\"x\":1}",
                status: "running",
            })
            .unwrap();
        let qr = QuestRepo::new(&conn);
        let a = qr
            .insert(NewQuest {
                campaign_id: Some(cid),
                project: "agora",
                brief: "스키마",
                branch: Some("feat/a"),
                status: "completed",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        let b = qr
            .insert(NewQuest {
                campaign_id: Some(cid),
                project: "admin",
                brief: "반영",
                branch: Some("feat/b"),
                status: "running",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        qr.add_dependency(b, a).unwrap();
        (conn, cid)
    }

    #[test]
    fn suspend_produces_bundle_and_marks_state() {
        let (conn, cid) = setup();
        let bundle = suspend_campaign(&conn, cid, "home-mac", false).unwrap();
        assert_eq!(bundle.schema, 1);
        assert_eq!(bundle.origin_machine, "home-mac");
        assert_eq!(bundle.campaign.title, "동기화");
        assert_eq!(bundle.quests.len(), 2);
        // b는 a(인덱스 0)에 의존
        let b = bundle.quests.iter().find(|q| q.project == "admin").unwrap();
        assert_eq!(b.depends_on, vec![0]);
        // 상태 마킹
        let c = CampaignRepo::new(&conn).get(cid).unwrap().unwrap();
        assert_eq!(c.handoff_state, "suspended");
        assert_eq!(c.owner_machine.as_deref(), Some("home-mac"));
    }

    #[test]
    fn suspend_rejects_other_machine_without_force() {
        let (conn, cid) = setup();
        suspend_campaign(&conn, cid, "home-mac", false).unwrap();
        // 다른 기기가 force 없이 재중단 → 거부
        assert!(suspend_campaign(&conn, cid, "work-mac", false).is_err());
        // force면 허용
        assert!(suspend_campaign(&conn, cid, "work-mac", true).is_ok());
    }

    #[test]
    fn bundle_json_roundtrip() {
        let (conn, cid) = setup();
        let bundle = suspend_campaign(&conn, cid, "m", false).unwrap();
        let json = bundle.to_json().unwrap();
        let back = HandoffBundle::from_json(&json).unwrap();
        assert_eq!(bundle, back);
    }

    #[test]
    fn from_json_rejects_bad_schema() {
        let bad = r#"{"schema":99,"origin_machine":"m","exported_at":0,
          "campaign":{"title":"t","prompt":"p","plan_json":"{}","status":"running"},"quests":[]}"#;
        assert!(HandoffBundle::from_json(bad).is_err());
    }

    #[test]
    fn resume_imports_with_remapped_deps_and_reset() {
        let (mut conn, cid) = setup();
        let bundle = suspend_campaign(&conn, cid, "home-mac", false).unwrap();
        let new_cid = resume_bundle(&mut conn, &bundle, "work-mac").unwrap();
        assert_ne!(new_cid, cid);

        let c = CampaignRepo::new(&conn).get(new_cid).unwrap().unwrap();
        assert_eq!(c.handoff_state, "resumed");
        assert_eq!(c.owner_machine.as_deref(), Some("work-mac"));

        let quests = QuestRepo::new(&conn).list_for_campaign(new_cid).unwrap();
        assert_eq!(quests.len(), 2);
        let a = quests.iter().find(|q| q.project == "agora").unwrap();
        let b = quests.iter().find(|q| q.project == "admin").unwrap();
        // a는 completed 보존, b는 running→pending 리셋
        assert_eq!(a.status, "completed");
        assert_eq!(b.status, "pending");
        assert!(b.branch.is_none()); // pending이라 worktree branch 초기화
        // 의존성 재구성: b가 a에 의존 → a 완료라 b ready
        let ready = QuestRepo::new(&conn).ready_in_campaign(new_cid).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, b.id);
    }

    #[test]
    fn machine_id_reads_env() {
        // 명시 env 우선 (다른 테스트와 간섭 없게 set 후 unset)
        std::env::set_var("LUIDA_MACHINE_ID", "test-mac-xyz");
        assert_eq!(machine_id(), "test-mac-xyz");
        std::env::remove_var("LUIDA_MACHINE_ID");
    }
}
