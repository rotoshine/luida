//! Memory Tree (spec §6.1) — 청크화 + 계층 요약 트리.
//!
//! 긴 문서를 토큰 예산 청크로 쪼개 leaf로 저장하고, fanout 단위로 묶어 요약 부모를
//! 만들어 단일 루트까지 쌓는다. reflect/plan은 상위 노드만 읽어 토큰 효율적으로 맥락 주입.
//!
//! 요약 함수는 주입식(LLM 또는 단순 결합) — 구조와 요약 정책을 분리해 테스트 가능.

use anyhow::{bail, Context, Result};

use luida_core::{Connection, MemoryChunkRepo, NewMemoryChunk};

/// 대략적 토큰 추정 (코드포인트/4, 최소 1). 정밀 토크나이저는 후속.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4).max(1)
}

/// 마크다운을 토큰 예산 이내 청크로 분할 (문단 경계 보존).
/// 한 문단이 예산을 넘으면 그 문단은 단독 청크가 된다(추가 분할 안 함).
pub fn chunk_markdown(text: &str, budget_tokens: usize) -> Vec<String> {
    let budget = budget_tokens.max(1);
    let blocks: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .collect();

    let mut chunks = Vec::new();
    let mut cur = String::new();
    for b in blocks {
        let prospective = if cur.is_empty() {
            estimate_tokens(b)
        } else {
            estimate_tokens(&cur) + estimate_tokens(b)
        };
        if !cur.is_empty() && prospective > budget {
            chunks.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(b);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

/// 문서를 청크화해 leaf(level 0)로 저장. 저장된 chunk id들 반환.
pub fn ingest_chunks(
    conn: &mut Connection,
    text: &str,
    path: Option<&str>,
    budget_tokens: usize,
) -> Result<Vec<i64>> {
    let chunks = chunk_markdown(text, budget_tokens);
    if chunks.is_empty() {
        return Ok(vec![]);
    }
    let tx = conn.transaction()?;
    let mut ids = Vec::with_capacity(chunks.len());
    {
        let repo = MemoryChunkRepo::new(&tx);
        for c in &chunks {
            let id = repo.insert(NewMemoryChunk {
                parent_id: None,
                level: 0,
                score: None,
                token_estimate: estimate_tokens(c) as i64,
                path,
                summary: c,
            })?;
            ids.push(id);
        }
    }
    tx.commit()?;
    Ok(ids)
}

/// leaf id들을 fanout 단위로 묶어 요약 부모를 만들고 단일 루트까지 쌓는다.
/// `summarize`는 자식 요약들을 받아 부모 요약을 만든다(LLM 또는 단순 결합). 루트 id 반환.
pub fn build_summary_tree<F>(
    conn: &mut Connection,
    leaf_ids: Vec<i64>,
    fanout: usize,
    summarize: F,
) -> Result<i64>
where
    F: Fn(&[String]) -> Result<String>,
{
    if leaf_ids.is_empty() {
        bail!("leaf가 없습니다");
    }
    if fanout < 2 {
        bail!("fanout은 2 이상이어야 합니다");
    }

    let tx = conn.transaction()?;
    let root = {
        let repo = MemoryChunkRepo::new(&tx);
        let mut current = leaf_ids;
        let mut level = 1i64;
        while current.len() > 1 {
            let mut next = Vec::new();
            for group in current.chunks(fanout) {
                let mut summaries = Vec::with_capacity(group.len());
                for &id in group {
                    let chunk = repo
                        .get(id)?
                        .with_context(|| format!("chunk {id} 없음"))?;
                    summaries.push(chunk.summary);
                }
                let parent_summary = summarize(&summaries)?;
                let pid = repo.insert(NewMemoryChunk {
                    parent_id: None,
                    level,
                    score: None,
                    token_estimate: estimate_tokens(&parent_summary) as i64,
                    path: None,
                    summary: &parent_summary,
                })?;
                for &cid in group {
                    repo.set_parent(cid, pid)?;
                }
                next.push(pid);
            }
            current = next;
            level += 1;
        }
        current[0]
    };
    tx.commit()?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use luida_core::{migrate, open_memory, MemoryChunkRepo};

    fn db() -> Connection {
        let mut c = open_memory().unwrap();
        migrate(&mut c).unwrap();
        c
    }

    #[test]
    fn estimate_tokens_monotonic() {
        assert!(estimate_tokens("a") >= 1);
        assert!(estimate_tokens("aaaaaaaa") > estimate_tokens("aa"));
    }

    #[test]
    fn chunk_short_text_single() {
        let chunks = chunk_markdown("짧은 문단 하나", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "짧은 문단 하나");
    }

    #[test]
    fn chunk_splits_on_budget() {
        // 각 문단 ~5토큰, 예산 6 → 문단마다 분리
        let text = "문단 하나입니다\n\n문단 둘입니다\n\n문단 셋입니다";
        let chunks = chunk_markdown(text, 6);
        assert!(chunks.len() >= 2);
        // 내용 보존
        let joined = chunks.join("\n\n");
        assert!(joined.contains("문단 하나"));
        assert!(joined.contains("문단 셋"));
    }

    #[test]
    fn chunk_empty_text() {
        assert!(chunk_markdown("", 10).is_empty());
        assert!(chunk_markdown("\n\n  \n\n", 10).is_empty());
    }

    #[test]
    fn ingest_chunks_stores_leaves() {
        let mut conn = db();
        let text = "에이\n\n비\n\n시\n\n디";
        let ids = ingest_chunks(&mut conn, text, Some("/v/doc.md"), 2).unwrap();
        assert!(ids.len() >= 2);
        let repo = MemoryChunkRepo::new(&conn);
        for id in &ids {
            let c = repo.get(*id).unwrap().unwrap();
            assert_eq!(c.level, 0);
            assert_eq!(c.path.as_deref(), Some("/v/doc.md"));
        }
    }

    #[test]
    fn build_tree_single_root_and_links() {
        let mut conn = db();
        // leaf 5개
        let ids = {
            let repo = MemoryChunkRepo::new(&conn);
            (0..5)
                .map(|i| {
                    repo.insert(NewMemoryChunk {
                        parent_id: None,
                        level: 0,
                        score: None,
                        token_estimate: 1,
                        path: None,
                        summary: &format!("leaf {i}"),
                    })
                    .unwrap()
                })
                .collect::<Vec<_>>()
        };
        // 단순 결합 요약
        let summarize = |kids: &[String]| -> Result<String> { Ok(format!("[{}]", kids.join("|"))) };
        let root = build_summary_tree(&mut conn, ids.clone(), 2, summarize).unwrap();

        let repo = MemoryChunkRepo::new(&conn);
        // 루트는 부모 없음
        assert!(repo.get(root).unwrap().unwrap().parent_id.is_none());
        // 루트가 유일
        let roots = repo.roots().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, root);
        // 모든 leaf는 부모를 가짐
        for id in &ids {
            assert!(repo.get(*id).unwrap().unwrap().parent_id.is_some());
        }
        // 루트 요약은 하위 요약을 결합한 형태
        assert!(repo.get(root).unwrap().unwrap().summary.contains("leaf 0"));
    }

    #[test]
    fn build_tree_rejects_empty_and_bad_fanout() {
        let mut conn = db();
        assert!(build_summary_tree(&mut conn, vec![], 2, |_| Ok("x".into())).is_err());
        let id = MemoryChunkRepo::new(&conn)
            .insert(NewMemoryChunk {
                parent_id: None,
                level: 0,
                score: None,
                token_estimate: 1,
                path: None,
                summary: "x",
            })
            .unwrap();
        assert!(build_summary_tree(&mut conn, vec![id], 1, |_| Ok("x".into())).is_err());
    }

    #[test]
    fn build_tree_single_leaf_is_root() {
        let mut conn = db();
        let id = MemoryChunkRepo::new(&conn)
            .insert(NewMemoryChunk {
                parent_id: None,
                level: 0,
                score: None,
                token_estimate: 1,
                path: None,
                summary: "only",
            })
            .unwrap();
        // leaf 1개 → 그 자체가 루트 (요약 노드 생성 안 함)
        let root = build_summary_tree(&mut conn, vec![id], 2, |_| Ok("x".into())).unwrap();
        assert_eq!(root, id);
    }
}
