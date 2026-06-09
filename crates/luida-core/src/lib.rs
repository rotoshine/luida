//! luida-core — tavern.db 스키마·연결·repository (v2 Rust).
//!
//! project-centric. campaigns/quests/inmail/events/relationships + agents resolver.

pub mod agents;
pub mod db;
pub mod handoff;
pub mod models;
pub mod recover;
pub mod repo;

pub use db::{default_db_path, migrate, now_ms, open_db, open_memory};
pub use rusqlite::Connection;
pub use models::{
    Campaign, Event, Inmail, MemoryChunk, Project, Quest, Relationship, EpochMs,
};
pub use repo::{
    CampaignRepo, EnqueueResult, EventRepo, InmailRepo, MemoryChunkRepo, NewCampaign, NewEvent,
    NewInmail, NewMemoryChunk, NewQuest, NewRelationship, ProjectRepo, QuestInsert, QuestRepo,
    RelationshipRepo,
};
pub use agents::{
    default_agents_path, kill_process_group, pid_alive, process_start_time, resolve, runner_alive,
    runtime_available, AgentsConfig, CancelToken, ResolvedAgent,
};
pub use handoff::{
    machine_id, resume_bundle, suspend_campaign, HandoffBundle,
};
pub use recover::reconcile_interrupted_quests;

/// 데모 모드 여부 — `LUIDA_FAKE=1`(또는 true)이면 외부 LLM/repo 없이 결정적 fake 런타임 사용.
pub fn is_fake() -> bool {
    std::env::var("LUIDA_FAKE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// db 열고 마이그레이션 + 재시작 재조정 + agents.json 로드 (CLI·TUI 공용 부팅).
///
/// 부팅마다 reconcile 를 돌려, 이전 실행(CLI·server·TUI 무관)이 강제 종료돼 'running'으로 남은
/// **이 머신의 죽은 runner** 모험을 '중단(이어받기 가능)'으로 되돌린다. 살아있는 runner(자기 자신
/// 포함)는 PID+시작시각으로 식별돼 건드리지 않으므로, 동시 실행 중에도 안전하다.
pub fn open_ready(db_path: &std::path::Path) -> anyhow::Result<(Connection, AgentsConfig)> {
    let mut conn = open_db(db_path)?;
    migrate(&mut conn)?;
    // reconcile 실패(일시적 DB 락/IO 등)로 부팅을 막지는 않되, 조용히 삼키지 않고 알린다.
    // reconcile 은 멱등이라 다음 부팅에서 다시 시도된다.
    if let Err(e) = reconcile_interrupted_quests(&conn) {
        eprintln!("⚠️  부팅 재조정(reconcile) 실패 — 무시하고 계속: {e}");
    }
    let cfg = AgentsConfig::load_or_default(&default_agents_path())?;
    Ok((conn, cfg))
}

#[cfg(test)]
mod boot_tests {
    use super::*;
    use crate::repo::{CampaignRepo, NewCampaign, NewQuest, ProjectRepo, QuestRepo};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// open_ready 가 부팅 시 이 머신의 죽은 runner 모험을 'running'→'pending'으로 복구하는지.
    /// (server/CLI 가 디스패치 후 강제 종료된 케이스의 자가 치유 경로)
    #[test]
    fn open_ready_reconciles_dead_runner_quest() {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("luida-core-boot-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("t.db");
        let qid;
        {
            let (conn, _) = open_ready(&db).unwrap();
            ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
            let cid = CampaignRepo::new(&conn)
                .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
                .unwrap();
            qid = QuestRepo::new(&conn)
                .insert(NewQuest {
                    campaign_id: Some(cid),
                    project: "agora",
                    brief: "b",
                    branch: None,
                    status: "running",
                    depends_on_quest_id: None,
                    source_inmail_id: None,
                })
                .unwrap();
            // 이 머신의 죽은 runner 모사 (큰 PID = 거의 확실히 죽음).
            QuestRepo::new(&conn)
                .set_runner(qid, 4_000_000_000, &machine_id(), Some(123))
                .unwrap();
            assert_eq!(QuestRepo::new(&conn).get(qid).unwrap().unwrap().status, "running");
        }
        // 재부팅(open_ready) → 죽은 runner 모험이 pending(이어받기 가능)으로 복구.
        let (conn, _) = open_ready(&db).unwrap();
        assert_eq!(QuestRepo::new(&conn).get(qid).unwrap().unwrap().status, "pending");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
