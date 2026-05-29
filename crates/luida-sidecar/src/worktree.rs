//! worktree provisioning 추상화.
//!
//! 메모리 규약: worktree는 **worktrunk(`wt`) 전용** — raw `git worktree` 직접 호출 금지.
//! quest dispatch가 worker를 띄우기 전 격리된 작업 공간을 만든다.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};

/// directive 파일명 충돌 방지용 프로세스 내 시퀀스.
static WT_SEQ: AtomicU64 = AtomicU64::new(0);

/// 모든 반환 경로에서 temp 파일을 정리하는 RAII 가드.
struct TmpFileGuard(PathBuf);
impl Drop for TmpFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 생성된 worktree (브랜치 + 절대경로).
#[derive(Debug, Clone, PartialEq)]
pub struct Worktree {
    pub branch: String,
    pub path: PathBuf,
}

/// 프로젝트 repo에 worker용 worktree를 만들어주는 추상화.
/// 실제 구현은 worktrunk 기반(`WorktrunkProvider`). 테스트는 fake를 주입한다.
pub trait WorktreeProvider {
    /// `repo_path`의 repo에 `codename` 브랜치로 worktree 생성.
    fn create(&self, repo_path: &Path, codename: &str) -> Result<Worktree>;
}

/// worktrunk(`wt switch --create`) 기반 provider.
///
/// `wt c`는 `wt switch --create --base origin/main --execute=claude`의 alias라
/// worktree 생성과 동시에 claude REPL을 띄운다(interactive 경로). headless 디스패치는
/// worker를 Luida가 직접 spawn하므로 **worktree 생성만** 필요 → `wt switch --create`를 쓴다.
/// (둘 다 worktrunk이므로 raw git worktree 금지 규약을 지킨다.)
///
/// worktree 경로는 `wt` 셸 함수가 쓰는 `WORKTRUNK_DIRECTIVE_CD_FILE` 메커니즘으로 회수한다.
///
/// 주의: 실제 `wt` 바이너리 + git repo가 필요하므로 **실환경에서 검증**한다(단위테스트 제외).
pub struct WorktrunkProvider {
    bin: String,
    base: String,
}

impl Default for WorktrunkProvider {
    fn default() -> Self {
        Self {
            bin: "wt".to_string(),
            base: "origin/main".to_string(),
        }
    }
}

impl WorktrunkProvider {
    pub fn new(bin: impl Into<String>, base: impl Into<String>) -> Self {
        Self {
            bin: bin.into(),
            base: base.into(),
        }
    }
}

impl WorktreeProvider for WorktrunkProvider {
    fn create(&self, repo_path: &Path, codename: &str) -> Result<Worktree> {
        if !repo_path.is_dir() {
            bail!("repo 경로가 디렉터리가 아님: {repo_path:?}");
        }
        // wt 셸 함수가 cd 타겟을 적어주는 directive 파일.
        // pid + ms + 시퀀스로 동시 dispatch 간 경합 차단, RAII로 모든 경로에서 정리.
        let seq = WT_SEQ.fetch_add(1, Ordering::Relaxed);
        let cd_file = std::env::temp_dir().join(format!(
            "luida-wt-{}-{}-{}.cd",
            std::process::id(),
            luida_core::now_ms(),
            seq
        ));
        let _guard = TmpFileGuard(cd_file.clone());
        let status = std::process::Command::new(&self.bin)
            .arg("switch")
            .arg("--create")
            .arg(codename)
            .arg("--base")
            .arg(&self.base)
            .arg("-C")
            .arg(repo_path)
            .arg("-y")
            .env("WORKTRUNK_DIRECTIVE_CD_FILE", &cd_file)
            .status()
            .context("wt 실행 실패 (worktrunk 설치/PATH 확인)")?;
        if !status.success() {
            bail!("wt switch --create 실패 (codename={codename})");
        }
        let path = std::fs::read_to_string(&cd_file)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .context("wt가 worktree 경로를 반환하지 않음 (directive 파일 비어있음)")?;
        Ok(Worktree {
            branch: codename.to_string(),
            path: PathBuf::from(path),
        })
    }
}

/// 데모용 worktree provider — `wt`/git 없이 temp 디렉터리만 생성.
pub struct TempWorktree;

impl WorktreeProvider for TempWorktree {
    fn create(&self, _repo: &Path, codename: &str) -> Result<Worktree> {
        let safe: String = codename
            .chars()
            .map(|c| if c == '/' { '-' } else { c })
            .collect();
        let dir = std::env::temp_dir().join("luida-fake-wt").join(safe);
        std::fs::create_dir_all(&dir)?;
        Ok(Worktree {
            branch: codename.to_string(),
            path: dir,
        })
    }
}

/// 현재 모드에 맞는 worktree provider — `LUIDA_FAKE`면 temp, 아니면 worktrunk.
pub fn make_worktree() -> Box<dyn WorktreeProvider> {
    if luida_core::is_fake() {
        Box::new(TempWorktree)
    } else {
        Box::new(WorktrunkProvider::default())
    }
}
