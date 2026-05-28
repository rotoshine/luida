//! luida-brain — 학습·기억·보고 (모험의 서).
//!
//! - `memory`: Obsidian 호환 vault (chronicle/campaigns/projects)
//! - `report`: 원정 완료 보고서 생성 → vault 기록 + campaign 마감
//!
//! (reflect/memory-tree 등 지속 학습 데몬은 후속 Phase)

mod ingest;
mod memory;
mod memtree;
mod report;

pub use ingest::ingest_project;
pub use memory::{sanitize_filename, MemoryVault};
pub use memtree::{build_summary_tree, chunk_markdown, estimate_tokens, ingest_chunks};
pub use report::report_campaign;
