//! luida — 단일 진입점 CLI (v2 Rust).

use anyhow::Result;
use clap::{Parser, Subcommand};
use luida_core::{default_db_path, migrate, open_db, ProjectRepo};

#[derive(Parser)]
#[command(
    name = "luida",
    version,
    about = "🍺 루이다 — 멀티 에이전트 오케스트레이터 (v2)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// tavern.db 관리
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// 모험지(프로젝트) 관리
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// TUI 대시보드 (모험지 등록부)
    Ui,
}

#[derive(Subcommand)]
enum DbAction {
    /// tavern.db 초기화·마이그레이션
    Init,
}

#[derive(Subcommand)]
enum ProjectAction {
    /// 모험지 등록 (또는 갱신)
    Add {
        name: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value = "main")]
        base: String,
        #[arg(long)]
        desc: Option<String>,
    },
    /// 모험지 목록
    List,
    /// 모험지 제거
    Remove { name: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db_path = default_db_path();

    match cli.cmd {
        Cmd::Db { action } => match action {
            DbAction::Init => {
                let mut conn = open_db(&db_path)?;
                let applied = migrate(&mut conn)?;
                println!("🏮 루이다의 주점을 준비했어요.");
                println!("   DB: {}", db_path.display());
                if applied.is_empty() {
                    println!("   상태: 이미 최신");
                } else {
                    println!("   새로 적용: {}", applied.join(", "));
                }
            }
        },
        Cmd::Project { action } => {
            let mut conn = open_db(&db_path)?;
            migrate(&mut conn)?;
            let repo = ProjectRepo::new(&conn);
            match action {
                ProjectAction::Add {
                    name,
                    path,
                    base,
                    desc,
                } => {
                    repo.add(&name, &path, &base, desc.as_deref())?;
                    println!("🗺  모험지 등록: {name} ({base}) → {path}");
                }
                ProjectAction::List => {
                    let projects = repo.list()?;
                    if projects.is_empty() {
                        println!("등록된 모험지가 없습니다. `luida project add` 로 등록하세요.");
                    } else {
                        println!("🗺  모험지 {}곳:", projects.len());
                        for p in projects {
                            let desc = p.description.unwrap_or_default();
                            println!(
                                "   {:<16} {:<10} {}{}",
                                p.name,
                                p.base_branch,
                                p.repo_path,
                                if desc.is_empty() {
                                    String::new()
                                } else {
                                    format!("  — {desc}")
                                }
                            );
                        }
                    }
                }
                ProjectAction::Remove { name } => {
                    if repo.remove(&name)? {
                        println!("🗑  모험지 제거: {name}");
                    } else {
                        println!("그런 모험지가 없습니다: {name}");
                    }
                }
            }
        }
        Cmd::Ui => {
            luida_tui::run(&db_path)?;
        }
    }

    Ok(())
}
