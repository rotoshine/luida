//! luida CLI 통합 테스트 — 빌드된 `luida` 바이너리를 실제 실행한다.
//!
//! 모든 외부 의존성(LLM/repo/vault)은 `LUIDA_FAKE=1` 결정적 런타임으로 대체하고,
//! DB/agents/memory 경로는 env override 로 임시 디렉터리에 격리한다(홈 오염·전역 상태 없음).
//! 각 테스트는 독립 임시 디렉터리 + 독립 프로세스라 병렬 실행에도 안전하다.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

/// 테스트별 고유 임시 작업 디렉터리.
fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("luida-cli-it-{}-{tag}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 격리된 env 로 `luida <args>` 실행.
fn luida(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_luida"))
        .args(args)
        .env("LUIDA_FAKE", "1")
        .env("LUIDA_DB_PATH", dir.join("tavern.db"))
        .env("LUIDA_AGENTS_PATH", dir.join("agents.json"))
        .env("LUIDA_MEMORY_DIR", dir.join("memory"))
        .env("HOME", dir) // 방어적: 혹시 빠진 경로가 있어도 홈을 오염시키지 않음
        .output()
        .expect("luida 바이너리 실행 실패")
}

/// 성공(exit 0)을 단언하고 stdout 을 문자열로 반환.
fn ok_stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "비정상 종료: status={:?}\nstdout={}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn version_and_help_work() {
    let dir = temp_dir("help");
    let v = ok_stdout(&luida(&dir, &["--version"]));
    assert!(v.contains("luida"), "version: {v}");
    let h = ok_stdout(&luida(&dir, &["--help"]));
    // 주요 서브커맨드가 도움말에 노출되는지.
    assert!(h.contains("campaign"));
    assert!(h.contains("project"));
    assert!(h.contains("ui"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn db_init_is_idempotent() {
    let dir = temp_dir("dbinit");
    let first = ok_stdout(&luida(&dir, &["db", "init"]));
    assert!(first.contains("주점") || first.contains("DB"), "init: {first}");
    assert!(dir.join("tavern.db").exists());
    let second = ok_stdout(&luida(&dir, &["db", "init"]));
    assert!(second.contains("최신"), "두 번째 init: {second}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_add_list_remove() {
    let dir = temp_dir("project");
    ok_stdout(&luida(&dir, &["db", "init"]));
    let add = ok_stdout(&luida(
        &dir,
        &["project", "add", "agora", "--path", "/repos/agora", "--base", "main", "--desc", "커뮤니티"],
    ));
    assert!(add.contains("등록"), "add: {add}");
    let list = ok_stdout(&luida(&dir, &["project", "list"]));
    assert!(list.contains("agora"), "list: {list}");
    assert!(list.contains("커뮤니티"));
    let remove = ok_stdout(&luida(&dir, &["project", "remove", "agora"]));
    assert!(remove.contains("제거"), "remove: {remove}");
    let empty = ok_stdout(&luida(&dir, &["project", "list"]));
    assert!(empty.contains("없습니다"), "empty: {empty}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn agents_init_resolve_show() {
    let dir = temp_dir("agents");
    let init = ok_stdout(&luida(&dir, &["agents", "init"]));
    assert!(init.contains("agents.json"), "init: {init}");
    assert!(dir.join("agents.json").exists());
    let resolve = ok_stdout(&luida(&dir, &["agents", "resolve", "quest.execute"]));
    assert!(resolve.contains("runtime"), "resolve: {resolve}");
    assert!(resolve.contains("model"));
    let show = ok_stdout(&luida(&dir, &["agents", "show"]));
    assert!(show.contains("런타임"), "show: {show}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn campaign_full_lifecycle_plan_run_report() {
    let dir = temp_dir("lifecycle");
    ok_stdout(&luida(&dir, &["db", "init"]));
    ok_stdout(&luida(&dir, &["project", "add", "agora", "--path", "/r/agora"]));
    ok_stdout(&luida(&dir, &["project", "add", "admin", "--path", "/r/admin"]));

    // plan → 원정 #1 (fake DAG: agora → admin)
    let plan = ok_stdout(&luida(&dir, &["campaign", "plan", "agora와 admin 정렬"]));
    assert!(plan.contains("계획 완료"), "plan: {plan}");
    assert!(plan.contains("#1"));

    // list → 진행 중 원정 노출
    let list = ok_stdout(&luida(&dir, &["campaign", "list"]));
    assert!(list.contains("진행 중") && list.contains("#1"), "list: {list}");

    // run → 모든 quest 완료
    let run = ok_stdout(&luida(&dir, &["campaign", "run", "1"]));
    assert!(run.contains("실행") && run.contains("완료"), "run: {run}");

    // report → 모험의 서 기록 + 원정 마감
    let report = ok_stdout(&luida(&dir, &["campaign", "report", "1"]));
    assert!(report.contains("기록"), "report: {report}");

    // 완료된 원정은 더 이상 진행 중 목록에 없음
    let after = ok_stdout(&luida(&dir, &["campaign", "list"]));
    assert!(after.contains("없습니다"), "after: {after}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reflect_and_relationship_list() {
    let dir = temp_dir("reflect");
    ok_stdout(&luida(&dir, &["db", "init"]));
    let reflect = ok_stdout(&luida(&dir, &["reflect", "--since-hours", "24"]));
    assert!(reflect.contains("학습"), "reflect: {reflect}");
    // 빈 상태의 관계 목록
    let rel = ok_stdout(&luida(&dir, &["relationship", "list"]));
    assert!(rel.contains("관계"), "rel: {rel}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_missing_campaign_fails_cleanly() {
    let dir = temp_dir("badrun");
    ok_stdout(&luida(&dir, &["db", "init"]));
    // 존재하지 않는 원정 실행 → 비정상 종료(패닉 아님), stderr 에 진단.
    let out = luida(&dir, &["campaign", "run", "999"]);
    assert!(!out.status.success(), "없는 원정인데 성공 종료함");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quest_resume_wrong_state_fails() {
    let dir = temp_dir("resume");
    ok_stdout(&luida(&dir, &["db", "init"]));
    // 존재하지 않는 quest 재개 → 비정상 종료.
    let out = luida(&dir, &["quest", "resume", "1", "답변"]);
    assert!(!out.status.success(), "없는 quest 인데 성공 종료함");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_subcommand_is_error() {
    let dir = temp_dir("unknown");
    let out = luida(&dir, &["nonexistent-command"]);
    assert!(!out.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}
