use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::now_ms;
use crate::models::Quest;

/// running quest 의 runner 리스 한 행: (quest_id, runner_pid, runner_machine, runner_started_at).
pub type QuestRunner = (i64, Option<i64>, Option<String>, Option<i64>);

/// 새 quest 생성 입력.
pub struct NewQuest<'a> {
    pub campaign_id: Option<i64>,
    pub project: &'a str,
    pub brief: &'a str,
    pub branch: Option<&'a str>,
    pub status: &'a str,
    pub depends_on_quest_id: Option<i64>,
    pub source_inmail_id: Option<i64>,
}

/// 멱등 insert 결과.
pub struct QuestInsert {
    pub id: i64,
    /// false면 source_inmail_id 충돌로 기존 quest 반환.
    pub created: bool,
}

pub struct QuestRepo<'a> {
    conn: &'a Connection,
}

impl<'a> QuestRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, q: NewQuest) -> Result<i64> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO quests
               (campaign_id, project, brief, branch, status,
                depends_on_quest_id, source_inmail_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                q.campaign_id,
                q.project,
                q.brief,
                q.branch,
                q.status,
                q.depends_on_quest_id,
                q.source_inmail_id,
                now
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// source_inmail_id 기반 멱등 insert. 이미 있으면 기존 반환.
    /// UNIQUE race(다른 프로세스가 그 사이 insert)도 재조회로 복구.
    pub fn insert_idempotent(&self, q: NewQuest) -> Result<QuestInsert> {
        let src = q.source_inmail_id;
        if let Some(s) = src {
            if let Some(existing) = self.find_by_source(s)? {
                return Ok(QuestInsert {
                    id: existing.id,
                    created: false,
                });
            }
        }
        match self.insert(q) {
            Ok(id) => Ok(QuestInsert { id, created: true }),
            Err(e) => {
                if let Some(s) = src {
                    if let Some(existing) = self.find_by_source(s)? {
                        return Ok(QuestInsert {
                            id: existing.id,
                            created: false,
                        });
                    }
                }
                Err(e)
            }
        }
    }

    pub fn get(&self, id: i64) -> Result<Option<Quest>> {
        Ok(self
            .conn
            .query_row(SELECT_ONE, params![id], Self::map_row)
            .optional()?)
    }

    pub fn find_by_source(&self, source_inmail_id: i64) -> Result<Option<Quest>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, campaign_id, project, brief, branch, worktree_path, status,
                        progress, pr_url, log_path, depends_on_quest_id, source_inmail_id,
                        created_at, updated_at, completed_at
                 FROM quests WHERE source_inmail_id = ?1",
                params![source_inmail_id],
                Self::map_row,
            )
            .optional()?)
    }

    pub fn list_active(&self) -> Result<Vec<Quest>> {
        self.query_many(
            "SELECT id, campaign_id, project, brief, branch, worktree_path, status,
                    progress, pr_url, log_path, depends_on_quest_id, source_inmail_id,
                    created_at, updated_at, completed_at
             FROM quests
             WHERE status NOT IN ('completed', 'failed', 'aborted')
             ORDER BY updated_at DESC",
            params![],
        )
    }

    pub fn list_for_campaign(&self, campaign_id: i64) -> Result<Vec<Quest>> {
        self.query_many(
            "SELECT id, campaign_id, project, brief, branch, worktree_path, status,
                    progress, pr_url, log_path, depends_on_quest_id, source_inmail_id,
                    created_at, updated_at, completed_at
             FROM quests WHERE campaign_id = ?1 ORDER BY id",
            params![campaign_id],
        )
    }

    pub fn set_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE quests SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now_ms(), id],
        )?;
        Ok(())
    }

    /// running 으로 돌리는 프로세스(runner) 기록 — 재시작 시 고아/중단 재조정용.
    /// `started_at` 은 runner 프로세스 시작 시각(epoch ms) — PID 재사용 구분용.
    pub fn set_runner(
        &self,
        id: i64,
        pid: i64,
        machine: &str,
        started_at: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE quests SET runner_pid = ?1, runner_machine = ?2, runner_started_at = ?3,
                              updated_at = ?4 WHERE id = ?5",
            params![pid, machine, started_at, now_ms(), id],
        )?;
        Ok(())
    }

    /// running 상태 quest 의 (id, pid, machine, started_at) 목록. 재시작 재조정 입력.
    ///
    /// 재조정 대상은 set_runner 로 runner 리스(pid·시작시각)를 확실히 찍는 'running' 으로 한정한다.
    /// 다른 상태(예: 'reviewing')를 추가하려면 그 상태 진입 시에도 set_runner 갱신을 보장해야,
    /// stale runner_pid 로 멀쩡한 작업을 pending 으로 되돌리는 오복구를 막을 수 있다.
    pub fn list_running_runners(&self) -> Result<Vec<QuestRunner>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, runner_pid, runner_machine, runner_started_at FROM quests
             WHERE status = 'running'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn set_progress(&self, id: i64, progress: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE quests SET progress = ?1, updated_at = ?2 WHERE id = ?3",
            params![progress, now_ms(), id],
        )?;
        Ok(())
    }

    pub fn set_worktree(&self, id: i64, branch: &str, path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE quests SET branch = ?1, worktree_path = ?2, updated_at = ?3 WHERE id = ?4",
            params![branch, path, now_ms(), id],
        )?;
        Ok(())
    }

    pub fn mark_completed(&self, id: i64, pr_url: Option<&str>) -> Result<()> {
        let now = now_ms();
        self.conn.execute(
            "UPDATE quests SET status = 'completed', pr_url = ?1, updated_at = ?2, completed_at = ?2 WHERE id = ?3",
            params![pr_url, now, id],
        )?;
        Ok(())
    }

    /// quest에 의존성 추가 (다중 의존 DAG). 멱등(INSERT OR IGNORE).
    pub fn add_dependency(&self, quest_id: i64, depends_on_quest_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO quest_deps (quest_id, depends_on_quest_id) VALUES (?1, ?2)",
            params![quest_id, depends_on_quest_id],
        )?;
        Ok(())
    }

    /// quest의 모든 의존성 quest id (quest_deps 기준).
    pub fn dependencies(&self, quest_id: i64) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT depends_on_quest_id FROM quest_deps WHERE quest_id = ?1")?;
        let rows = stmt.query_map(params![quest_id], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// campaign 내 모든 quest 의 의존성(quest_id → depends_on_quest_id 목록)을 한 번의 쿼리로.
    /// ready_in_campaign 의 quest 별 `dependencies()` 개별 호출(N+1)을 제거하기 위한 일괄 로드.
    fn dependencies_for_campaign(&self, campaign_id: i64) -> Result<HashMap<i64, Vec<i64>>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.quest_id, d.depends_on_quest_id
             FROM quest_deps d JOIN quests q ON q.id = d.quest_id
             WHERE q.campaign_id = ?1",
        )?;
        let rows = stmt.query_map(params![campaign_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut map: HashMap<i64, Vec<i64>> = HashMap::new();
        for r in rows {
            let (qid, dep) = r?;
            map.entry(qid).or_default().push(dep);
        }
        Ok(map)
    }

    /// 의존 quest가 모두 완료되어 실행 가능한 quest (pending + 모든 의존 완료).
    ///
    /// 의존성은 두 곳을 모두 본다: 레거시 단일 `depends_on_quest_id`(back-compat) +
    /// `quest_deps` 조인 테이블(다중 의존). 둘 다 완료여야 ready.
    pub fn ready_in_campaign(&self, campaign_id: i64) -> Result<Vec<Quest>> {
        use std::collections::HashSet;
        let all = self.list_for_campaign(campaign_id)?;
        let completed: HashSet<i64> = all
            .iter()
            .filter(|q| q.status == "completed")
            .map(|q| q.id)
            .collect();
        // campaign 의 모든 quest_deps 를 한 번에 로드 → pending quest 별 개별 쿼리(N+1) 제거.
        // run_campaign 루프가 매 반복마다 ready_in_campaign 을 부르므로 호출 빈도가 높다.
        let deps_by_quest = self.dependencies_for_campaign(campaign_id)?;

        let mut ready = Vec::new();
        for q in all.into_iter().filter(|q| q.status == "pending") {
            let legacy_ok = q
                .depends_on_quest_id
                .is_none_or(|dep| completed.contains(&dep));
            if !legacy_ok {
                continue;
            }
            // 의존 기록이 없으면(None) 의존 없음 = ready. 있으면 전부 완료여야 ready.
            let multi_ok = deps_by_quest
                .get(&q.id)
                .is_none_or(|deps| deps.iter().all(|dep| completed.contains(dep)));
            if multi_ok {
                ready.push(q);
            }
        }
        Ok(ready)
    }

    fn query_many(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Quest>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(p, Self::map_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn map_row(r: &Row) -> rusqlite::Result<Quest> {
        Ok(Quest {
            id: r.get(0)?,
            campaign_id: r.get(1)?,
            project: r.get(2)?,
            brief: r.get(3)?,
            branch: r.get(4)?,
            worktree_path: r.get(5)?,
            status: r.get(6)?,
            progress: r.get(7)?,
            pr_url: r.get(8)?,
            log_path: r.get(9)?,
            depends_on_quest_id: r.get(10)?,
            source_inmail_id: r.get(11)?,
            created_at: r.get(12)?,
            updated_at: r.get(13)?,
            completed_at: r.get(14)?,
        })
    }
}

const SELECT_ONE: &str =
    "SELECT id, campaign_id, project, brief, branch, worktree_path, status,
            progress, pr_url, log_path, depends_on_quest_id, source_inmail_id,
            created_at, updated_at, completed_at
     FROM quests WHERE id = ?1";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate, open_memory};
    use crate::repo::ProjectRepo;

    fn setup() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        let p = ProjectRepo::new(&conn);
        p.add("agora", "/a", "main", None).unwrap();
        p.add("admin", "/b", "main", None).unwrap();
        conn
    }

    fn nq<'a>(project: &'a str, brief: &'a str) -> NewQuest<'a> {
        NewQuest {
            campaign_id: None,
            project,
            brief,
            branch: None,
            status: "pending",
            depends_on_quest_id: None,
            source_inmail_id: None,
        }
    }

    #[test]
    fn insert_and_get() {
        let conn = setup();
        let repo = QuestRepo::new(&conn);
        let id = repo.insert(nq("agora", "스키마 작업")).unwrap();
        let q = repo.get(id).unwrap().unwrap();
        assert_eq!(q.project, "agora");
        assert_eq!(q.brief, "스키마 작업");
        assert_eq!(q.status, "pending");
    }

    #[test]
    fn fk_rejects_unknown_project() {
        let conn = setup();
        let repo = QuestRepo::new(&conn);
        assert!(repo.insert(nq("ghost", "x")).is_err());
    }

    #[test]
    fn invalid_status_rejected() {
        let conn = setup();
        let repo = QuestRepo::new(&conn);
        let mut q = nq("agora", "x");
        q.status = "bogus";
        assert!(repo.insert(q).is_err());
    }

    #[test]
    fn idempotent_by_source_inmail() {
        let conn = setup();
        let repo = QuestRepo::new(&conn);
        let mut q1 = nq("agora", "first");
        q1.source_inmail_id = Some(42);
        let r1 = repo.insert_idempotent(q1).unwrap();
        assert!(r1.created);

        let mut q2 = nq("agora", "dup");
        q2.source_inmail_id = Some(42);
        let r2 = repo.insert_idempotent(q2).unwrap();
        assert!(!r2.created);
        assert_eq!(r1.id, r2.id);
    }

    #[test]
    fn status_and_worktree_updates() {
        let conn = setup();
        let repo = QuestRepo::new(&conn);
        let id = repo.insert(nq("agora", "x")).unwrap();
        repo.set_status(id, "running").unwrap();
        repo.set_worktree(id, "feat/x", "/wt/x").unwrap();
        repo.set_progress(id, Some("50%")).unwrap();
        let q = repo.get(id).unwrap().unwrap();
        assert_eq!(q.status, "running");
        assert_eq!(q.branch.as_deref(), Some("feat/x"));
        assert_eq!(q.worktree_path.as_deref(), Some("/wt/x"));
        assert_eq!(q.progress.as_deref(), Some("50%"));
    }

    #[test]
    fn mark_completed() {
        let conn = setup();
        let repo = QuestRepo::new(&conn);
        let id = repo.insert(nq("agora", "x")).unwrap();
        repo.mark_completed(id, Some("https://x/pr/1")).unwrap();
        let q = repo.get(id).unwrap().unwrap();
        assert_eq!(q.status, "completed");
        assert_eq!(q.pr_url.as_deref(), Some("https://x/pr/1"));
        assert!(q.completed_at.is_some());
    }

    #[test]
    fn list_active_excludes_terminal() {
        let conn = setup();
        let repo = QuestRepo::new(&conn);
        let a = repo.insert(nq("agora", "a")).unwrap();
        let b = repo.insert(nq("admin", "b")).unwrap();
        repo.mark_completed(b, None).unwrap();
        let active = repo.list_active().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, a);
    }

    #[test]
    fn ready_in_campaign_respects_dag() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        ProjectRepo::new(&conn).add("admin", "/b", "main", None).unwrap();
        let cid = crate::repo::CampaignRepo::new(&conn)
            .insert(crate::repo::NewCampaign {
                title: "t",
                prompt: "p",
                plan_json: "{}",
                status: "running",
            })
            .unwrap();
        let repo = QuestRepo::new(&conn);
        // q1: 의존 없음 → ready
        let mut q1 = nq("agora", "q1");
        q1.campaign_id = Some(cid);
        let q1id = repo.insert(q1).unwrap();
        // q2: q1에 의존 → q1 완료 전엔 not ready
        let mut q2 = nq("admin", "q2");
        q2.campaign_id = Some(cid);
        q2.depends_on_quest_id = Some(q1id);
        repo.insert(q2).unwrap();

        let ready = repo.ready_in_campaign(cid).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, q1id);

        // q1 완료 → q2도 ready
        repo.mark_completed(q1id, None).unwrap();
        let ready2 = repo.ready_in_campaign(cid).unwrap();
        assert_eq!(ready2.len(), 1);
        assert_eq!(ready2[0].project, "admin");
    }

    #[test]
    fn ready_respects_multi_dependency_quest_deps() {
        // 다이아몬드: a → b,c → d. d는 quest_deps로 b·c 둘 다 의존.
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("p", "/p", "main", None).unwrap();
        let cid = crate::repo::CampaignRepo::new(&conn)
            .insert(crate::repo::NewCampaign {
                title: "t",
                prompt: "p",
                plan_json: "{}",
                status: "running",
            })
            .unwrap();
        let repo = QuestRepo::new(&conn);
        let mk = |brief: &str| {
            let mut q = nq("p", "x");
            q.campaign_id = Some(cid);
            q.brief = brief;
            repo.insert(q).unwrap()
        };
        let a = mk("a");
        let b = mk("b");
        let c = mk("c");
        let d = mk("d");
        repo.add_dependency(b, a).unwrap();
        repo.add_dependency(c, a).unwrap();
        repo.add_dependency(d, b).unwrap();
        repo.add_dependency(d, c).unwrap();

        // 처음엔 a만 ready
        let r = repo.ready_in_campaign(cid).unwrap();
        assert_eq!(r.iter().map(|q| q.id).collect::<Vec<_>>(), vec![a]);

        // a 완료 → b,c ready (d는 아직)
        repo.mark_completed(a, None).unwrap();
        let r = repo.ready_in_campaign(cid).unwrap();
        let ids: std::collections::HashSet<i64> = r.iter().map(|q| q.id).collect();
        assert_eq!(ids, [b, c].into_iter().collect());

        // b만 완료 → d는 아직 (c 미완)
        repo.mark_completed(b, None).unwrap();
        let r = repo.ready_in_campaign(cid).unwrap();
        assert_eq!(r.iter().map(|q| q.id).collect::<Vec<_>>(), vec![c]);

        // c도 완료 → d ready
        repo.mark_completed(c, None).unwrap();
        let r = repo.ready_in_campaign(cid).unwrap();
        assert_eq!(r.iter().map(|q| q.id).collect::<Vec<_>>(), vec![d]);
    }

    #[test]
    fn add_dependency_is_idempotent() {
        let conn = setup();
        let repo = QuestRepo::new(&conn);
        let a = repo.insert(nq("agora", "a")).unwrap();
        let b = repo.insert(nq("admin", "b")).unwrap();
        repo.add_dependency(b, a).unwrap();
        repo.add_dependency(b, a).unwrap(); // 중복 무시
        assert_eq!(repo.dependencies(b).unwrap(), vec![a]);
    }
}
