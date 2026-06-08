//! 재시작 재조정 — 죽은 runner 가 남긴 running quest 를 '중단'으로 되돌려 이어받기 가능하게.
//!
//! Model A(깔끔히 중단 후 재개): 정상 종료 시엔 TUI 가 자식을 kill 하고 중단 처리하지만,
//! 강제 종료(SIGKILL)된 경우엔 quest 가 'running' 으로 남는다. 다음 시작 때 이 함수가
//! 이 머신의 죽은(또는 미기록) runner 의 running quest 를 'pending'(=중단·이어받기 가능)으로
//! 되돌리고 `quest_interrupted` 이벤트를 남긴다.

use anyhow::Result;
use serde_json::json;

use crate::agents::runner_alive;
use crate::handoff::machine_id;
use crate::repo::{EventRepo, NewEvent, QuestRepo};
use crate::Connection;

/// running/reviewing quest 중 **이 머신이 명시적으로 소유**한 죽은 runner 가 남긴 것을 'pending'
/// 으로 되돌리고 quest_interrupted 이벤트 기록. 되돌린 quest id 목록 반환.
///
/// 안전 원칙(절대 살아있는 작업을 건드리지 않는다):
/// - runner_machine 이 이 머신이 아니거나 미기록(NULL)이면 건드리지 않는다 → 다른 머신/외부 프로세스/
///   구버전이 소유한 quest 를 함부로 되돌려 split-brain(이중 디스패치) 나는 걸 막는다.
/// - PID 가 살아있고 **시작시각까지 일치**하면 건드리지 않는다 → PID 재사용으로 죽은 runner 를
///   살아있다고 오판해 quest 가 영영 'running' 에 묶이는 것도, 살아있는 걸 죽이는 것도 막는다.
pub fn reconcile_interrupted_quests(conn: &Connection) -> Result<Vec<i64>> {
    let me = machine_id();
    let running = QuestRepo::new(conn).list_running_runners()?;
    let mut recovered = Vec::new();
    for (id, pid, machine, started_at) in running {
        // 이 머신이 명시적으로 소유한 것만 재조정 대상 (NULL/다른 머신 = 불명 → 보존).
        if machine.as_deref() != Some(me.as_str()) {
            continue;
        }
        // runner 가 살아있으면(PID 생존 + 시작시각 일치) 건드리지 않음.
        if matches!(pid, Some(p) if p > 0 && runner_alive(p as u32, started_at)) {
            continue;
        }
        // 이 머신의 죽은 runner → 중단(이어받기 가능) 처리.
        let qrepo = QuestRepo::new(conn);
        let cid = qrepo.get(id)?.and_then(|q| q.campaign_id);
        qrepo.set_status(id, "pending")?;
        EventRepo::new(conn).record(NewEvent {
            campaign_id: cid,
            quest_id: Some(id),
            actor: "luida",
            kind: "quest_interrupted",
            payload: &json!({ "reason": "runner_gone", "machine": me }).to_string(),
        })?;
        recovered.push(id);
    }
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate, open_memory};
    use crate::repo::{CampaignRepo, NewCampaign, NewQuest, ProjectRepo};

    fn seed() -> (Connection, i64) {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
            .unwrap();
        (conn, cid)
    }

    fn mk_quest(conn: &Connection, cid: i64, status: &str) -> i64 {
        QuestRepo::new(conn)
            .insert(NewQuest {
                campaign_id: Some(cid),
                project: "agora",
                brief: "b",
                branch: None,
                status,
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap()
    }

    #[test]
    fn leaves_running_with_no_runner_unknown_owner() {
        // runner 미기록(NULL machine) = 소유 불명 → 함부로 되돌리지 않는다(구버전/외부 보호).
        let (conn, cid) = seed();
        let q = mk_quest(&conn, cid, "running");
        let recovered = reconcile_interrupted_quests(&conn).unwrap();
        assert!(recovered.is_empty());
        assert_eq!(QuestRepo::new(&conn).get(q).unwrap().unwrap().status, "running");
    }

    #[test]
    fn recovers_running_with_dead_pid_on_this_machine() {
        let (conn, cid) = seed();
        let q = mk_quest(&conn, cid, "running");
        QuestRepo::new(&conn)
            .set_runner(q, 4_000_000_000, &machine_id(), Some(123))
            .unwrap();
        let recovered = reconcile_interrupted_quests(&conn).unwrap();
        assert_eq!(recovered, vec![q]);
        assert_eq!(QuestRepo::new(&conn).get(q).unwrap().unwrap().status, "pending");
    }

    #[test]
    fn leaves_running_with_live_pid() {
        let (conn, cid) = seed();
        let q = mk_quest(&conn, cid, "running");
        // 자기 자신 PID + 자기 시작시각 = 살아있음 → 건드리지 않음.
        let st = crate::agents::process_start_time(std::process::id());
        QuestRepo::new(&conn)
            .set_runner(q, std::process::id() as i64, &machine_id(), st)
            .unwrap();
        let recovered = reconcile_interrupted_quests(&conn).unwrap();
        assert!(recovered.is_empty());
        assert_eq!(QuestRepo::new(&conn).get(q).unwrap().unwrap().status, "running");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn recovers_running_with_reused_pid_wrong_starttime() {
        // 살아있는 PID 라도 시작시각이 어긋나면(=PID 재사용으로 원 runner 는 죽음) 중단 처리.
        let (conn, cid) = seed();
        let q = mk_quest(&conn, cid, "running");
        let st = crate::agents::process_start_time(std::process::id()).unwrap();
        QuestRepo::new(&conn)
            .set_runner(q, std::process::id() as i64, &machine_id(), Some(st + 999_999))
            .unwrap();
        let recovered = reconcile_interrupted_quests(&conn).unwrap();
        assert_eq!(recovered, vec![q], "PID 재사용된 죽은 runner 는 복구돼야");
    }

    #[test]
    fn leaves_other_machine_owned() {
        let (conn, cid) = seed();
        let q = mk_quest(&conn, cid, "running");
        QuestRepo::new(&conn)
            .set_runner(q, 4_000_000_000, "other-machine", Some(1))
            .unwrap();
        let recovered = reconcile_interrupted_quests(&conn).unwrap();
        assert!(recovered.is_empty());
        assert_eq!(QuestRepo::new(&conn).get(q).unwrap().unwrap().status, "running");
    }

    #[test]
    fn ignores_terminal_and_pending() {
        let (conn, cid) = seed();
        mk_quest(&conn, cid, "completed");
        mk_quest(&conn, cid, "pending");
        mk_quest(&conn, cid, "needs_input");
        let recovered = reconcile_interrupted_quests(&conn).unwrap();
        assert!(recovered.is_empty());
    }
}
