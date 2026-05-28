//! project.ingest — 등록된 모험지의 README/구조를 읽어 맥락 요약 → vault (spec §7.2).

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use luida_core::agents::{compress_context, AgentInvocation, AgentRuntime, ResolvedAgent};
use luida_core::{resolve, AgentsConfig, Connection, EventRepo, NewEvent, ProjectRepo};

use crate::memory::MemoryVault;

/// README/구조 컨텍스트에서 읽을 최대 바이트 (TokenJuice 도입 전 단순 상한).
const MAX_README_BYTES: usize = 8_000;

/// 모험지 맥락을 요약해 `projects/<name>.md`에 기록하고 project.context_path를 채운다.
pub fn ingest_project<F>(
    conn: &mut Connection,
    cfg: &AgentsConfig,
    project_name: &str,
    vault: &MemoryVault,
    runtime_factory: F,
) -> Result<std::path::PathBuf>
where
    F: Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>>,
{
    let project = ProjectRepo::new(conn)
        .get(project_name)?
        .with_context(|| format!("프로젝트 '{project_name}' 미등록"))?;

    // 수집 컨텍스트를 TokenJuice로 예산 내 압축 (큰 README·구조 대비).
    let blob = compress_context(&gather_repo_context(Path::new(&project.repo_path)), 6_000);

    let resolved = resolve(cfg, "project.ingest", Some(project_name))?;
    let inv = AgentInvocation {
        prompt: build_ingest_prompt(project_name, &project.repo_path, &blob),
        ..Default::default()
    };
    let runtime = runtime_factory(&resolved).context("ingest 런타임 생성 실패")?;
    let outcome = runtime.run(&resolved.model, &inv, &mut |_| {})?;
    let body = outcome
        .summary
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("## 자동 수집 컨텍스트\n\n{blob}"));

    let frontmatter = format!(
        "---\ntype: project-context\nproject: {}\nrepo: {}\n---",
        project_name, project.repo_path
    );
    let path = vault.write_project_context(project_name, &frontmatter, &body)?;

    let path_str = path.to_string_lossy().to_string();
    let tx = conn.transaction()?;
    ProjectRepo::new(&tx).mark_ingested(project_name, &path_str)?;
    EventRepo::new(&tx).record(NewEvent {
        campaign_id: None,
        quest_id: None,
        actor: project_name,
        kind: "project_ingested",
        payload: &json!({ "context_path": path_str }).to_string(),
    })?;
    tx.commit()?;

    Ok(path)
}

/// repo에서 README + 최상위 구조를 모은다. 경로/파일이 없어도 견고하게(빈 노트) 동작.
fn gather_repo_context(repo: &Path) -> String {
    let mut out = String::new();
    if !repo.is_dir() {
        return format!("(repo 경로 없음: {repo:?})");
    }

    // README 후보
    for cand in ["README.md", "README", "readme.md", "Readme.md"] {
        let p = repo.join(cand);
        if let Ok(mut s) = std::fs::read_to_string(&p) {
            if s.len() > MAX_README_BYTES {
                let mut end = MAX_README_BYTES;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
                s.push_str("\n…(생략)");
            }
            out.push_str(&format!("### {cand}\n{s}\n\n"));
            break;
        }
    }

    // 최상위 구조 (디렉터리/파일 이름만, 정렬)
    if let Ok(rd) = std::fs::read_dir(repo) {
        let mut names: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        if !names.is_empty() {
            out.push_str("### 최상위 구조\n");
            for n in names.iter().take(50) {
                out.push_str(&format!("- {n}\n"));
            }
        }
    }

    if out.is_empty() {
        out.push_str("(README·구조 정보 없음)");
    }
    out
}

fn build_ingest_prompt(name: &str, repo_path: &str, blob: &str) -> String {
    format!(
        "당신은 Luida의 모험지 분석가입니다. 아래 저장소를 읽고 향후 작업에 도움 될 맥락 요약을 작성하세요.\n\n\
모험지: {name}\n경로: {repo_path}\n\n수집된 정보:\n{blob}\n\n\
무엇을 하는 프로젝트인지, 주요 디렉터리·스택·진입점·주의사항을 간결한 Markdown으로 요약하세요. 본문만 출력."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryVault;
    use luida_core::agents::{AgentEvent, ScriptedRuntime};
    use luida_core::{migrate, open_memory};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn unique_dir(prefix: &str) -> std::path::PathBuf {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("{prefix}-{}-{}", std::process::id(), seq))
    }

    fn cfg() -> AgentsConfig {
        let json = r#"{
          "version": 1,
          "defaults": { "runtime": "claude", "tier": "simple" },
          "runtimes": {
            "claude": { "kind": "claude-cli", "command": "claude",
              "models": { "complex": "opus", "simple": "sonnet" } }
          },
          "actions": { "project.ingest": { "runtime": "claude", "tier": "simple" } }
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

    #[test]
    fn ingest_reads_readme_and_writes_context() {
        // 임시 repo + README
        let repo = unique_dir("luida-repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("README.md"), "# Agora\n커뮤니티 웹").unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();

        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn)
            .add("agora", repo.to_str().unwrap(), "main", None)
            .unwrap();

        let vault = MemoryVault::new(unique_dir("luida-vault"));
        let path = ingest_project(&mut conn, &cfg(), "agora", &vault, result_factory("요약: 커뮤니티 웹앱")).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("project-context"));
        assert!(content.contains("커뮤니티 웹앱"));

        let p = ProjectRepo::new(&conn).get("agora").unwrap().unwrap();
        assert!(p.context_path.is_some());
        assert!(p.last_ingested_at.is_some());
    }

    #[test]
    fn ingest_tolerates_missing_repo_with_fallback() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn)
            .add("ghost", "/nonexistent/repo/xyz", "main", None)
            .unwrap();
        let vault = MemoryVault::new(unique_dir("luida-vault"));
        // LLM이 summary 없이 끝나도 수집 blob(경로 없음 노트)로 fallback
        let empty = |_: &ResolvedAgent| {
            Ok(Box::new(ScriptedRuntime::new(vec![AgentEvent::Result {
                success: true,
                summary: None,
            }])) as Box<dyn AgentRuntime>)
        };
        let path = ingest_project(&mut conn, &cfg(), "ghost", &vault, empty).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("repo 경로 없음"));
    }

    #[test]
    fn ingest_rejects_unknown_project() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        let vault = MemoryVault::new(unique_dir("luida-vault"));
        assert!(ingest_project(&mut conn, &cfg(), "nope", &vault, result_factory("x")).is_err());
    }
}
