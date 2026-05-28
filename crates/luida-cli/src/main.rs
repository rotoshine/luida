//! luida — 단일 진입점 CLI (v2 Rust).

use anyhow::Result;
use clap::{Parser, Subcommand};
use luida_core::agents::default_agents_path;
use luida_core::{
    resolve, runtime_available, AgentsConfig, default_db_path, migrate, open_db, ProjectRepo,
};

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
    /// 에이전트 런타임/모델 설정
    Agents {
        #[command(subcommand)]
        action: AgentsAction,
    },
    /// HTTP/SSE 서버 (GUI·클라이언트 브리지)
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },
    /// TUI 대시보드 (모험지 등록부)
    Ui,
}

#[derive(Subcommand)]
enum ServerAction {
    /// 로컬 HTTP/SSE 서버 시작
    Start {
        #[arg(long, default_value_t = 4321)]
        port: u16,
    },
}

#[derive(Subcommand)]
enum AgentsAction {
    /// 기본 agents.json 생성 (이미 있으면 유지)
    Init,
    /// 행위(action)를 런타임/모델/모드로 해소해서 보여줌
    Resolve {
        action: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// 현재 설정 요약
    Show,
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
        Cmd::Agents { action } => {
            let path = default_agents_path();
            match action {
                AgentsAction::Init => {
                    if path.exists() {
                        println!("이미 존재: {}", path.display());
                    } else {
                        AgentsConfig::default_config().save(&path)?;
                        println!("🤖 기본 agents.json 생성: {}", path.display());
                    }
                }
                AgentsAction::Resolve { action, project } => {
                    let cfg = AgentsConfig::load_or_default(&path)?;
                    let r = resolve(&cfg, &action, project.as_deref())?;
                    println!("🎯 {} → ", r.action);
                    println!("   runtime : {} ({})", r.runtime, r.kind);
                    println!("   model   : {}", r.model);
                    println!("   tier    : {}", r.tier);
                    println!("   mode    : {}", r.mode);
                    if let Some(p) = &project {
                        println!("   project : {p}");
                    }
                    let avail = runtime_available(&cfg, &r.runtime);
                    println!("   사용가능 : {}", if avail { "예" } else { "아니오 (CLI 미설치?)" });
                }
                AgentsAction::Show => {
                    let cfg = AgentsConfig::load_or_default(&path)?;
                    println!("🤖 agents.json ({})", path.display());
                    println!("   기본: {} / {}", cfg.defaults.runtime, cfg.defaults.tier);
                    println!("   런타임:");
                    let mut names: Vec<_> = cfg.runtimes.keys().collect();
                    names.sort();
                    for n in names {
                        let rt = &cfg.runtimes[n];
                        println!(
                            "     {:<10} {:<18} complex={} simple={} {}",
                            n,
                            rt.kind,
                            rt.models.complex,
                            rt.models.simple,
                            if rt.enabled { "" } else { "[비활성]" }
                        );
                    }
                    println!("   행위 매핑: {}건", cfg.actions.len());
                }
            }
        }
        Cmd::Server { action } => match action {
            ServerAction::Start { port } => {
                let mut conn = open_db(&db_path)?;
                migrate(&mut conn)?;
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async move { luida_server::serve(port, conn).await })?;
            }
        },
        Cmd::Ui => {
            luida_tui::run(&db_path)?;
        }
    }

    Ok(())
}
