//! 모험의 서 — Obsidian 호환 memory vault (로컬 우선 KB, spec §6.1).
//!
//! `~/.luida/memory/`를 vault로 본다. frontmatter + `[[wikilink]]`로 청크·원정·프로젝트를
//! 상호 연결. base 경로는 주입 가능(테스트는 temp dir).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// memory vault — base 디렉터리 아래에 chronicle/campaigns/projects를 둔다.
pub struct MemoryVault {
    base: PathBuf,
}

impl MemoryVault {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// `~/.luida/memory` (LUIDA_MEMORY_DIR env로 override).
    pub fn default_vault() -> Self {
        if let Ok(p) = std::env::var("LUIDA_MEMORY_DIR") {
            return Self::new(p);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self::new(Path::new(&home).join(".luida").join("memory"))
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    /// 원정 보고서를 `campaigns/<id>-<slug>.md`로 기록. frontmatter + 본문.
    pub fn write_campaign_report(
        &self,
        campaign_id: i64,
        slug: &str,
        frontmatter: &str,
        body: &str,
    ) -> Result<PathBuf> {
        let dir = self.base.join("campaigns");
        std::fs::create_dir_all(&dir).with_context(|| format!("vault 디렉터리 생성 실패: {dir:?}"))?;
        let path = dir.join(format!("{campaign_id:04}-{slug}.md"));
        let content = format!("{}\n\n{}\n", frontmatter.trim_end(), body.trim());
        std::fs::write(&path, content).with_context(|| format!("보고서 쓰기 실패: {path:?}"))?;
        Ok(path)
    }

    /// 모험의 서(chronicle.md)에 한 줄 append. (월별 분리는 후속 — 현재 단일 롤링 파일)
    pub fn append_chronicle(&self, line: &str) -> Result<PathBuf> {
        use std::io::Write;
        std::fs::create_dir_all(&self.base)
            .with_context(|| format!("vault 생성 실패: {:?}", self.base))?;
        let path = self.base.join("chronicle.md");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("chronicle 열기 실패: {path:?}"))?;
        writeln!(f, "{}", line.trim_end())?;
        Ok(path)
    }

    /// 프로젝트 맥락을 `projects/<name>.md`로 기록 (project.ingest용).
    pub fn write_project_context(&self, name: &str, frontmatter: &str, body: &str) -> Result<PathBuf> {
        let dir = self.base.join("projects");
        std::fs::create_dir_all(&dir).with_context(|| format!("vault 생성 실패: {dir:?}"))?;
        let path = dir.join(format!("{}.md", sanitize_filename(name)));
        let content = format!("{}\n\n{}\n", frontmatter.trim_end(), body.trim());
        std::fs::write(&path, content).with_context(|| format!("맥락 쓰기 실패: {path:?}"))?;
        Ok(path)
    }
}

/// 파일명 안전화 — 경로 구분자·제어문자만 '-'로. CJK 등은 보존.
pub fn sanitize_filename(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            let unsafe_char = c.is_control()
                || c.is_whitespace()
                || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
            if unsafe_char {
                '-'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_vault() -> MemoryVault {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "luida-vault-test-{}-{}",
            std::process::id(),
            seq
        ));
        MemoryVault::new(dir)
    }

    #[test]
    fn write_and_read_campaign_report() {
        let v = temp_vault();
        let p = v
            .write_campaign_report(42, "스키마-동기화", "---\ntype: campaign-report\n---", "본문")
            .unwrap();
        assert!(p.ends_with("0042-스키마-동기화.md"));
        let content = std::fs::read_to_string(&p).unwrap();
        assert!(content.contains("campaign-report"));
        assert!(content.contains("본문"));
    }

    #[test]
    fn append_chronicle_accumulates() {
        let v = temp_vault();
        v.append_chronicle("- 첫 원정").unwrap();
        let p = v.append_chronicle("- 둘째 원정").unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert!(content.contains("첫 원정"));
        assert!(content.contains("둘째 원정"));
        assert_eq!(content.lines().count(), 2);
    }

    #[test]
    fn write_project_context_sanitizes() {
        let v = temp_vault();
        let p = v.write_project_context("agora/web", "---\n---", "맥락").unwrap();
        assert!(p.ends_with("agora-web.md"));
    }

    #[test]
    fn sanitize_handles_edge() {
        assert_eq!(sanitize_filename("a/b:c"), "a-b-c");
        assert_eq!(sanitize_filename("커뮤니티 웹"), "커뮤니티-웹");
        assert_eq!(sanitize_filename("///"), "untitled");
    }
}
