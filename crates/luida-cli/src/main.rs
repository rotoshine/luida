//! luida — 단일 진입점 CLI (v2 Rust).

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use luida_core::agents::{default_agents_path, AgentRuntime, ResolvedAgent};
use luida_core::{
    machine_id, resolve, resume_bundle, runtime_available, suspend_campaign, AgentsConfig,
    CampaignRepo, Connection, HandoffBundle, ProjectRepo, QuestRepo, RelationshipRepo,
    default_db_path, migrate, open_db,
};
use luida_brain::{ingest_project, reflect, report_campaign, MemoryVault};
use luida_planner::{plan_campaign, run_campaign};
use luida_runtimes::runtime_for_kind;
use luida_runtimes::fake_runtime_for;
use luida_sidecar::{resume_quest, triage_escalation, Worktree, WorktreeProvider, WorktrunkProvider};

/// 데모 모드 여부 — LUIDA_FAKE=1이면 외부 LLM/repo 없이 결정적 fake 런타임 사용.
fn is_fake() -> bool {
    std::env::var("LUIDA_FAKE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// 런타임 factory — fake면 결정적 데모 런타임, 아니면 로컬 CLI(claude/codex).
fn make_factory() -> impl Fn(&ResolvedAgent) -> Result<Box<dyn AgentRuntime>> {
    let fake = is_fake();
    move |r: &ResolvedAgent| {
        if fake {
            Ok(fake_runtime_for(&r.action))
        } else {
            runtime_for_kind(&r.kind, r.command.as_deref())
        }
    }
}

/// 데모용 worktree provider — wt/git 없이 temp 디렉터리 생성.
struct TempWorktree;
impl WorktreeProvider for TempWorktree {
    fn create(&self, _repo: &std::path::Path, codename: &str) -> Result<Worktree> {
        let safe: String = codename.chars().map(|c| if c == '/' { '-' } else { c }).collect();
        let dir = std::env::temp_dir().join("luida-fake-wt").join(safe);
        std::fs::create_dir_all(&dir)?;
        Ok(Worktree {
            branch: codename.to_string(),
            path: dir,
        })
    }
}

/// 현재 모드에 맞는 worktree provider.
fn make_worktree() -> Box<dyn WorktreeProvider> {
    if is_fake() {
        Box::new(TempWorktree)
    } else {
        Box::new(WorktrunkProvider::default())
    }
}

/// db 열고 마이그레이션 + agents.json 로드.
fn open_ready(db_path: &std::path::Path) -> Result<(Connection, AgentsConfig)> {
    let mut conn = open_db(db_path)?;
    migrate(&mut conn)?;
    let cfg = AgentsConfig::load_or_default(&default_agents_path())?;
    Ok((conn, cfg))
}

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
    /// 원정(campaign) — 계획·실행·보고
    Campaign {
        #[command(subcommand)]
        action: CampaignAction,
    },
    /// 모험(quest) — 재개·triage
    Quest {
        #[command(subcommand)]
        action: QuestAction,
    },
    /// 모험 중단·재개 (기기 간 핸드오프)
    Adventure {
        #[command(subcommand)]
        action: AdventureAction,
    },
    /// 학습 — 최근 이벤트 분석 → 프로젝트 관계 제안
    Reflect {
        /// 최근 N시간의 이벤트 (기본 24)
        #[arg(long, default_value_t = 24)]
        since_hours: i64,
    },
    /// 프로젝트 간 자동화 관계 관리
    Relationship {
        #[command(subcommand)]
        action: RelationshipAction,
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
enum CampaignAction {
    /// 사용자 프롬프트 → 원정 DAG 계획 (campaign.plan)
    Plan { prompt: String },
    /// 계획된 원정을 의존성 순으로 실행
    Run { id: i64 },
    /// 완료 원정 보고서 작성 → 모험의 서
    Report { id: i64 },
    /// 진행 중 원정 목록
    List,
}

#[derive(Subcommand)]
enum QuestAction {
    /// needs_input 모험을 답변으로 재개
    Resume { id: i64, answer: String },
    /// escalation을 분류 (자동 해소 가능 여부)
    Triage { id: i64 },
}

#[derive(Subcommand)]
enum RelationshipAction {
    /// 전체 관계 목록 (활성·비활성)
    List,
    /// 관계 활성화 (학습 제안 승인)
    Enable { name: String },
    /// 관계 비활성화
    Disable { name: String },
}

#[derive(Subcommand)]
enum AdventureAction {
    /// 원정을 중단하고 핸드오프 번들(JSON) 저장
    Suspend {
        id: i64,
        #[arg(long, default_value = ".luida-handoff.json")]
        out: String,
        #[arg(long)]
        force: bool,
    },
    /// 핸드오프 번들에서 원정을 이어받기(재개)
    Resume {
        #[arg(long, default_value = ".luida-handoff.json")]
        from: String,
    },
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
    /// 모험지 맥락 요약 (README/구조 → 모험의 서)
    Ingest { name: String },
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
            let (mut conn, cfg) = open_ready(&db_path)?;
            match action {
                ProjectAction::Add {
                    name,
                    path,
                    base,
                    desc,
                } => {
                    ProjectRepo::new(&conn).add(&name, &path, &base, desc.as_deref())?;
                    println!("🗺  모험지 등록: {name} ({base}) → {path}");
                }
                ProjectAction::List => {
                    let projects = ProjectRepo::new(&conn).list()?;
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
                    if ProjectRepo::new(&conn).remove(&name)? {
                        println!("🗑  모험지 제거: {name}");
                    } else {
                        println!("그런 모험지가 없습니다: {name}");
                    }
                }
                ProjectAction::Ingest { name } => {
                    let vault = MemoryVault::default_vault();
                    let path = ingest_project(&mut conn, &cfg, &name, &vault, make_factory())?;
                    println!("📖 모험지 맥락 요약: {name} → {}", path.display());
                }
            }
        }
        Cmd::Campaign { action } => {
            let (mut conn, cfg) = open_ready(&db_path)?;
            match action {
                CampaignAction::Plan { prompt } => {
                    let cid = plan_campaign(&mut conn, &cfg, &prompt, make_factory())?;
                    let quests = QuestRepo::new(&conn).list_for_campaign(cid)?;
                    println!("🗺  원정 #{cid} 계획 완료 — quest {}건:", quests.len());
                    for q in quests {
                        println!("   q{} [{}] {}: {}", q.id, q.status, q.project, q.brief);
                    }
                    println!("   실행: `luida campaign run {cid}`");
                }
                CampaignAction::Run { id } => {
                    let report =
                        run_campaign(&mut conn, &cfg, id, make_worktree().as_ref(), make_factory())?;
                    println!(
                        "⚔  원정 #{id} 실행 — 완료 {} / 대기 {} / 실패 {}",
                        report.completed.len(),
                        report.needs_input.len(),
                        report.failed.len()
                    );
                    if report.all_completed {
                        println!("   모든 모험 완료 🍺 — `luida campaign report {id}`로 기록하세요.");
                    } else if !report.needs_input.is_empty() {
                        println!("   판단 대기: {:?} — `luida quest resume <id> \"<답변>\"`", report.needs_input);
                    }
                }
                CampaignAction::Report { id } => {
                    let vault = MemoryVault::default_vault();
                    let path = report_campaign(&mut conn, &cfg, id, &vault, make_factory())?;
                    println!("📜 모험의 서에 기록: {}", path.display());
                }
                CampaignAction::List => {
                    let active = CampaignRepo::new(&conn).list_active()?;
                    if active.is_empty() {
                        println!("진행 중인 원정이 없습니다.");
                    } else {
                        println!("🗺  진행 중 원정 {}건:", active.len());
                        for c in active {
                            println!("   #{} [{}] {}", c.id, c.status, c.title);
                        }
                    }
                }
            }
        }
        Cmd::Quest { action } => {
            let (mut conn, cfg) = open_ready(&db_path)?;
            match action {
                QuestAction::Resume { id, answer } => {
                    let out = resume_quest(&mut conn, &cfg, id, &answer, make_factory())?;
                    println!("⚔  모험 q{id} 재개 → {out:?}");
                }
                QuestAction::Triage { id } => {
                    let d = triage_escalation(&mut conn, &cfg, id, make_factory())?;
                    println!(
                        "🔎 q{id} triage — 사용자 필요: {} / 이유: {}",
                        if d.ask_user { "예" } else { "아니오(자동 해소 가능)" },
                        d.reason
                    );
                    if let Some(a) = d.auto_answer {
                        println!("   자동 답변 후보: {a}");
                    }
                }
            }
        }
        Cmd::Adventure { action } => {
            let (mut conn, _cfg) = open_ready(&db_path)?;
            let machine = machine_id();
            match action {
                AdventureAction::Suspend { id, out, force } => {
                    let bundle = suspend_campaign(&conn, id, &machine, force)?;
                    std::fs::write(&out, bundle.to_json()?)?;
                    println!("🏕  원정 #{id} 중단 (기기 {machine}) → {out}");
                    println!("   다른 기기에서: `luida adventure resume --from {out}`");
                }
                AdventureAction::Resume { from } => {
                    let json = std::fs::read_to_string(&from)?;
                    let bundle = HandoffBundle::from_json(&json)?;
                    let origin = bundle.origin_machine.clone();
                    let cid = resume_bundle(&mut conn, &bundle, &machine)?;
                    println!("⚔  원정 이어받음 (기기 {machine}) → 새 #{cid} (원본: {origin})");
                }
            }
        }
        Cmd::Reflect { since_hours } => {
            let (mut conn, cfg) = open_ready(&db_path)?;
            let since_ms = luida_core::now_ms() - since_hours.max(0) * 3_600_000;
            let report = reflect(&mut conn, &cfg, since_ms, make_factory())?;
            println!(
                "🧠 학습 완료 — 관계 제안 {}건 저장(비활성) / {}건 스킵 / 패턴 {}건",
                report.proposals_inserted,
                report.proposals_skipped,
                report.patterns.len()
            );
            for p in &report.patterns {
                println!("   · {p}");
            }
            if report.proposals_inserted > 0 {
                println!("   제안은 비활성 상태입니다. `luida relationship enable <name>`로 활성화하세요.");
            }
        }
        Cmd::Relationship { action } => {
            let (conn, _cfg) = open_ready(&db_path)?;
            let repo = RelationshipRepo::new(&conn);
            match action {
                RelationshipAction::List => {
                    let rels = repo.list_all()?;
                    if rels.is_empty() {
                        println!("등록된 관계가 없습니다. `luida reflect`로 제안을 받거나 직접 추가하세요.");
                    } else {
                        println!("🔗 관계 {}건:", rels.len());
                        for r in rels {
                            println!(
                                "   {:<18} {} --{}/{}-> {}  [{}{}]",
                                r.name.as_deref().unwrap_or("(이름없음)"),
                                r.from_project,
                                r.trigger_kind,
                                r.action,
                                r.to_project,
                                if r.is_enabled() { "활성" } else { "비활성" },
                                if r.source == "learned-promoted" { ", 학습" } else { "" },
                            );
                        }
                    }
                }
                RelationshipAction::Enable { name } => {
                    let r = repo.find_by_name(&name)?.context("그런 관계가 없습니다")?;
                    repo.set_enabled(r.id, true)?;
                    println!("✅ 관계 활성화: {name}");
                }
                RelationshipAction::Disable { name } => {
                    let r = repo.find_by_name(&name)?.context("그런 관계가 없습니다")?;
                    repo.set_enabled(r.id, false)?;
                    println!("⏸  관계 비활성화: {name}");
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
