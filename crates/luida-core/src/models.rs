use serde::{Deserialize, Serialize};

/// 모험지(Project) — 등록된 repo. v2의 persistent 엔티티.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub repo_path: String,
    pub base_branch: String,
    pub description: Option<String>,
    pub context_path: Option<String>,
    pub registered_at: i64,
    pub last_ingested_at: Option<i64>,
}
