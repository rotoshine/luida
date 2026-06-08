//! luida-tui — ratatui 기반 TUI 대시보드 + 에이전트 명령.
//!
//! 탭: 모험지(Projects) / 원정(Campaigns) / 모험(Quests). escalation 대기 카운트.
//! 명령(campaign plan/run · quest resume/triage)은 **백그라운드 워커 스레드**에서 실행하고
//! (mpsc 채널로 결과 수신), 완료 시 대시보드를 자동 갱신한다. 메인 루프는 `event::poll`로
//! 논블로킹 — 장시간 작업 중에도 UI가 멈추지 않는다.

use std::collections::HashMap;
use std::io::{stdout, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, ExecutableCommand};
use luida_brain::{report_campaign, MemoryVault};
use luida_core::{
    is_fake, migrate, now_ms, open_db, open_ready, reconcile_interrupted_quests, Campaign,
    CancelToken, Connection, EventRepo, InmailRepo, Project, Quest, CampaignRepo, ProjectRepo,
    QuestRepo,
};
use luida_planner::{plan_campaign, run_campaign};
use luida_runtimes::make_cancellable_factory;
use luida_sidecar::{make_worktree, resume_quest, triage_escalation};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap};

const GOLD: Color = Color::Rgb(0xFC, 0xD3, 0x4D);
const DIM: Color = Color::Rgb(0x8A, 0xA0, 0xC0);
const GREEN: Color = Color::Rgb(0x4A, 0xDE, 0x80);
const RED: Color = Color::Rgb(0xF8, 0x71, 0x71);

/// DB 스냅샷 — 렌더 입력. load는 순수 데이터 조회라 단위테스트 가능.
pub struct Dashboard {
    pub projects: Vec<Project>,
    pub campaigns: Vec<Campaign>,
    pub quests: Vec<Quest>,
    /// 사용자(@user) 앞 미배달 escalation 등 inmail 수.
    pub pending_user_mail: usize,
    /// campaign_id → (완료 quest 수, 전체 quest 수). 목록 진행도 표시용.
    pub campaign_progress: HashMap<i64, (usize, usize)>,
}

impl Dashboard {
    pub fn load(conn: &Connection) -> Result<Self> {
        let campaigns = CampaignRepo::new(conn).list_active()?;
        let qrepo = QuestRepo::new(conn);
        let mut campaign_progress = HashMap::new();
        for c in &campaigns {
            let qs = qrepo.list_for_campaign(c.id)?;
            let done = qs.iter().filter(|q| q.status == "completed").count();
            campaign_progress.insert(c.id, (done, qs.len()));
        }
        Ok(Self {
            projects: ProjectRepo::new(conn).list()?,
            campaigns,
            quests: qrepo.list_active()?,
            pending_user_mail: InmailRepo::new(conn).pending_for("@user")?.len(),
            campaign_progress,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tab {
    Projects,
    Campaigns,
    Quests,
}

impl Tab {
    fn all() -> [Tab; 3] {
        [Tab::Projects, Tab::Campaigns, Tab::Quests]
    }
    fn title(self) -> &'static str {
        match self {
            Tab::Projects => "모험지",
            Tab::Campaigns => "원정",
            Tab::Quests => "모험",
        }
    }
    fn next(self) -> Tab {
        match self {
            Tab::Projects => Tab::Campaigns,
            Tab::Campaigns => Tab::Quests,
            Tab::Quests => Tab::Projects,
        }
    }
    fn prev(self) -> Tab {
        match self {
            Tab::Projects => Tab::Quests,
            Tab::Campaigns => Tab::Projects,
            Tab::Quests => Tab::Campaigns,
        }
    }
}

/// 사용자가 TUI에서 내릴 수 있는 에이전트 명령.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Plan(String),
    Run(i64),
    Resume { quest_id: i64, answer: String },
    Triage(i64),
    /// 완료 원정 → 모험의 서 보고서 기록 (campaign.report).
    Report(i64),
}

/// 텍스트 입력이 필요한 명령의 입력 모드.
#[derive(Clone, Debug, PartialEq)]
enum InputKind {
    PlanPrompt,
    ResumeAnswer { quest_id: i64 },
    /// 모험지 등록 — `이름 경로 [브랜치] [설명...]` 한 줄 폼.
    AddProject,
}

/// 현재 상호작용 모드.
enum Mode {
    Normal,
    Input(InputKind),
    Running,
}

/// 워커 스레드 → 메인 루프 결과 메시지.
enum WorkerMsg {
    Done(String),
    Failed(String),
}

/// 상세 뷰 대상 (원정 또는 모험).
#[derive(Clone)]
enum DetailTarget {
    Campaign(i64),
    Quest(i64),
}

/// 원정/모험 상세 뷰 — events 타임라인 (별도 read conn 으로 폴링 갱신).
struct Detail {
    target: DetailTarget,
    title: String,
    lines: Vec<String>,
}

/// 실행 중 원정의 quest 상태 분포 — 진행 바용.
struct Progress {
    campaign_id: i64,
    total: usize,
    completed: usize,
    running: usize,
    needs_input: usize,
    failed: usize,
}

/// quest 가 사용자 판단/승인을 기다리는 상태인가 (판단대기 클래스).
/// 진행 집계·상태 색상·`n` 점프가 모두 같은 분류를 쓰도록 한 곳에 둔다.
fn is_waiting(status: &str) -> bool {
    matches!(status, "needs_input" | "needs_approval")
}

/// 원정의 quest 상태를 별도 read conn 으로 집계 (진행 바 폴링).
fn compute_progress(db_path: &Path, cid: i64) -> Result<Progress> {
    let conn = open_db(db_path)?;
    let quests = QuestRepo::new(&conn).list_for_campaign(cid)?;
    let mut p = Progress {
        campaign_id: cid,
        total: quests.len(),
        completed: 0,
        running: 0,
        needs_input: 0,
        failed: 0,
    };
    for q in &quests {
        match q.status.as_str() {
            "completed" => p.completed += 1,
            "running" | "reviewing" => p.running += 1,
            s if is_waiting(s) => p.needs_input += 1,
            "failed" | "aborted" => p.failed += 1,
            _ => {}
        }
    }
    Ok(p)
}

/// 명령을 동기 실행하고 결과 요약을 반환. 워커 스레드와 테스트에서 호출한다.
/// factory/worktree/conn 은 이 함수 안에서 생성 → 스레드로 넘길 값은 db_path·파라미터·cancel.
/// `cancel` 토큰은 런타임 factory 에 주입되어, TUI 종료 시 실행 중인 외부 CLI 자식을 즉시 정리한다.
pub fn dispatch(db_path: &Path, cmd: Command, cancel: CancelToken) -> Result<String> {
    let (mut conn, cfg) = open_ready(db_path)?;
    let factory = make_cancellable_factory(cancel);
    match cmd {
        Command::Plan(prompt) => {
            let cid = plan_campaign(&mut conn, &cfg, &prompt, factory)?;
            Ok(format!("원정 #{cid} 계획 완료"))
        }
        Command::Run(id) => {
            let report = run_campaign(&mut conn, &cfg, id, make_worktree().as_ref(), factory)?;
            let trig = if report.triggered > 0 {
                format!(" / 트리거 {}", report.triggered)
            } else {
                String::new()
            };
            if !report.interrupted.is_empty() {
                Ok(format!(
                    "원정 #{id} 중단 — 완료 {} / 남은 모험 재개 가능 (x)",
                    report.completed.len()
                ))
            } else {
                Ok(format!(
                    "원정 #{id} 실행 — 완료 {} / 대기 {} / 실패 {}{trig}",
                    report.completed.len(),
                    report.needs_input.len(),
                    report.failed.len()
                ))
            }
        }
        Command::Resume { quest_id, answer } => {
            let out = resume_quest(&mut conn, &cfg, quest_id, &answer, factory)?;
            Ok(format!("q{quest_id} 재개 → {out:?}"))
        }
        Command::Triage(id) => {
            let d = triage_escalation(&mut conn, &cfg, id, factory)?;
            Ok(format!(
                "q{id} triage — 사용자필요 {} · {}",
                if d.ask_user { "예" } else { "아니오" },
                d.reason
            ))
        }
        Command::Report(id) => {
            let vault = MemoryVault::default_vault();
            let path = report_campaign(&mut conn, &cfg, id, &vault, factory)?;
            Ok(format!("원정 #{id} 보고 기록 → {}", path.display()))
        }
    }
}

/// 모험지 등록 폼(`이름 경로 [브랜치] [설명...]`) 파싱. 이름·경로 필수.
/// 공백 분리라 경로에 공백이 있으면 지원하지 않는다(그 경우 CLI 사용).
fn parse_project_form(input: &str) -> Option<(String, String, String, Option<String>)> {
    let mut it = input.split_whitespace();
    let name = it.next()?.to_string();
    let path = it.next()?.to_string();
    let base = it.next().unwrap_or("main").to_string();
    let rest: Vec<&str> = it.collect();
    let desc = if rest.is_empty() {
        None
    } else {
        Some(rest.join(" "))
    };
    Some((name, path, base, desc))
}

/// 긴 문자열을 n글자로 자르고 말줄임(…). 상세 헤더용.
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}

/// 목록 재조회(reload) 시 선택을 안정적으로 보존하기 위한 키.
/// 목록은 updated_at DESC라 진행에 따라 재정렬되므로 인덱스가 아닌 정체성으로 추적한다.
#[derive(Clone, PartialEq, Debug)]
enum SelKey {
    Name(String),
    Id(i64),
}

/// 대시보드 자동 갱신 주기(ms). 백그라운드 server/daemon/다른 프로세스의 변경을 반영.
const RELOAD_INTERVAL_MS: i64 = 1200;

struct App {
    dash: Dashboard,
    tab: Tab,
    state: ListState,
    db_path: PathBuf,
    mode: Mode,
    input: String,
    /// 마지막 명령 결과 토스트 ("✅ …" / "⚠ …").
    status: Option<String>,
    /// 실행 중 명령 라벨 ("campaign run #1" 등).
    running_label: Option<String>,
    /// 워커 결과 채널 (Running 동안만 Some).
    rx: Option<Receiver<WorkerMsg>>,
    /// 상세 뷰 (열려있을 때만 Some). events 타임라인을 폴링해 표시.
    detail: Option<Detail>,
    /// 상세 타임라인 수직 스크롤 오프셋(줄). detail_follow=false 일 때만 사용.
    detail_scroll: u16,
    /// 상세 타임라인 꼬리추적(tail) 여부. true면 항상 최신 줄을 보여준다(기본).
    detail_follow: bool,
    /// 마지막 렌더에서 상세 패널의 보이는 줄 수(페이지 단위 스크롤용).
    detail_rows: u16,
    /// 도움말 오버레이 표시 여부 (? 토글).
    help: bool,
    /// status 토스트가 설정된 시각(ms). 일정 시간 후 자동 소멸.
    status_at: Option<i64>,
    /// 마지막 대시보드 자동 갱신 시각(ms).
    last_reload_ms: i64,
    /// 렌더 틱 카운터 (스피너 애니메이션용).
    tick: u64,
    /// 실행 중인 원정 id (Run 명령일 때만). 진행 바 대상.
    running_campaign: Option<i64>,
    /// 실행 중 원정 진행도 (폴링으로 갱신, Running 때만 Some).
    progress: Option<Progress>,
    /// 현재 워커의 취소 토큰 (Running 동안만 Some). 종료 시 cancel()로 자식 정리.
    cancel: Option<CancelToken>,
    /// 종료 요청됨 — 워커 취소 후 정리가 끝나면 루프를 빠져나간다.
    quitting: bool,
    /// 종료 요청 시각(ms) — 워커가 안 끝나도 유예 후 강제 종료(자식은 이미 kill됨).
    quit_at: Option<i64>,
}

impl App {
    fn new(dash: Dashboard, db_path: PathBuf) -> Self {
        let mut app = Self {
            dash,
            tab: Tab::Projects,
            state: ListState::default(),
            db_path,
            mode: Mode::Normal,
            input: String::new(),
            status: None,
            running_label: None,
            rx: None,
            detail: None,
            detail_scroll: 0,
            detail_follow: true,
            detail_rows: 0,
            help: false,
            status_at: None,
            last_reload_ms: now_ms(),
            tick: 0,
            running_campaign: None,
            progress: None,
            cancel: None,
            quitting: false,
            quit_at: None,
        };
        app.reset_selection();
        app
    }

    /// status 토스트를 설정하고 시각을 기록 (자동 소멸 타이머용).
    fn set_status(&mut self, msg: String) {
        self.status = Some(msg);
        self.status_at = Some(now_ms());
    }

    /// 다음 판단대기(needs_input/needs_approval) 모험으로 점프 (Quests 탭 + 선택). 없으면 무시.
    fn jump_to_needs_input(&mut self) {
        if let Some(i) = self.dash.quests.iter().position(|q| is_waiting(&q.status)) {
            self.tab = Tab::Quests;
            self.state.select(Some(i));
            self.sync_detail_to_selection();
        }
    }

    fn current_len(&self) -> usize {
        match self.tab {
            Tab::Projects => self.dash.projects.len(),
            Tab::Campaigns => self.dash.campaigns.len(),
            Tab::Quests => self.dash.quests.len(),
        }
    }

    fn reset_selection(&mut self) {
        self.state
            .select(if self.current_len() > 0 { Some(0) } else { None });
    }

    fn switch_tab(&mut self) {
        self.tab = self.tab.next();
        self.reset_selection();
    }

    fn switch_tab_prev(&mut self) {
        self.tab = self.tab.prev();
        self.reset_selection();
    }

    /// 현재 선택 항목의 안정 키 (탭별 name/id). 재조회 후 선택 복원에 쓴다.
    fn selected_key(&self) -> Option<SelKey> {
        let i = self.state.selected()?;
        Some(match self.tab {
            Tab::Projects => SelKey::Name(self.dash.projects.get(i)?.name.clone()),
            Tab::Campaigns => SelKey::Id(self.dash.campaigns.get(i)?.id),
            Tab::Quests => SelKey::Id(self.dash.quests.get(i)?.id),
        })
    }

    /// 키로 선택을 복원. 키가 사라졌으면 직전 인덱스를 길이에 맞춰 보정.
    fn select_key(&mut self, key: Option<SelKey>) {
        let len = self.current_len();
        if len == 0 {
            self.state.select(None);
            return;
        }
        let found = key.and_then(|k| match (self.tab, k) {
            (Tab::Projects, SelKey::Name(n)) => {
                self.dash.projects.iter().position(|p| p.name == n)
            }
            (Tab::Campaigns, SelKey::Id(id)) => {
                self.dash.campaigns.iter().position(|c| c.id == id)
            }
            (Tab::Quests, SelKey::Id(id)) => self.dash.quests.iter().position(|q| q.id == id),
            _ => None,
        });
        let idx = found.unwrap_or_else(|| self.state.selected().map_or(0, |i| i.min(len - 1)));
        self.state.select(Some(idx));
    }

    fn next(&mut self) {
        let len = self.current_len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().map_or(0, |i| (i + 1).min(len - 1));
        self.state.select(Some(i));
    }

    fn prev(&mut self) {
        if self.current_len() == 0 {
            return;
        }
        let i = self.state.selected().map_or(0, |i| i.saturating_sub(1));
        self.state.select(Some(i));
    }

    fn selected_campaign(&self) -> Option<&Campaign> {
        if self.tab == Tab::Campaigns {
            self.state.selected().and_then(|i| self.dash.campaigns.get(i))
        } else {
            None
        }
    }

    fn selected_quest(&self) -> Option<&Quest> {
        if self.tab == Tab::Quests {
            self.state.selected().and_then(|i| self.dash.quests.get(i))
        } else {
            None
        }
    }

    /// 별도 read connection으로 대시보드 재조회 + 선택을 정체성으로 보존.
    /// 목록이 재정렬돼도(updated_at DESC) 같은 항목에 하이라이트가 머문다.
    fn reload(&mut self) -> Result<()> {
        let key = self.selected_key();
        let conn = open_db(&self.db_path)?;
        self.dash = Dashboard::load(&conn)?;
        self.select_key(key);
        self.last_reload_ms = now_ms();
        Ok(())
    }

    /// 현재 선택 항목 기준 DetailTarget (Projects 탭은 타임라인 없음 → None).
    fn target_for_selection(&self) -> Option<DetailTarget> {
        match self.tab {
            Tab::Campaigns => self.selected_campaign().map(|c| DetailTarget::Campaign(c.id)),
            Tab::Quests => self.selected_quest().map(|q| DetailTarget::Quest(q.id)),
            Tab::Projects => None,
        }
    }

    /// 상세 스크롤을 처음 상태(꼬리추적)로 되돌린다 — 새 대상을 열 때.
    fn reset_detail_scroll(&mut self) {
        self.detail_scroll = 0;
        self.detail_follow = true;
    }

    /// 상세 뷰 토글: 닫혀있으면 현재 선택으로 열고, 열려있으면 닫는다.
    fn toggle_detail(&mut self) {
        if self.detail.is_some() {
            self.detail = None;
            return;
        }
        if let Some(target) = self.target_for_selection() {
            self.detail = Some(Detail { target, title: String::new(), lines: Vec::new() });
            self.reset_detail_scroll();
            let _ = self.refresh_detail(); // 조회 실패해도 빈 상세로 열림
        }
    }

    /// 상세가 열려있을 때 j/k 이동 시 현재 선택 항목으로 대상을 갱신.
    fn sync_detail_to_selection(&mut self) {
        if self.detail.is_none() {
            return;
        }
        // 대상이 바뀔 때만 스크롤 리셋 (같은 항목 재조회 시 위치 유지).
        let new_target = self.target_for_selection();
        let changed = match (&self.detail, &new_target) {
            (Some(d), Some(t)) => !same_target(&d.target, t),
            _ => true,
        };
        match new_target {
            Some(target) => {
                self.detail = Some(Detail { target, title: String::new(), lines: Vec::new() });
                if changed {
                    self.reset_detail_scroll();
                }
                let _ = self.refresh_detail();
            }
            None => self.detail = None,
        }
    }

    /// 상세 대상의 메타데이터 헤더 + events 타임라인을 별도 read conn 으로 재조회 (실시간 폴링).
    fn refresh_detail(&mut self) -> Result<()> {
        let target = match &self.detail {
            Some(d) => d.target.clone(),
            None => return Ok(()),
        };
        let conn = open_db(&self.db_path)?;
        let erepo = EventRepo::new(&conn);
        let (title, mut lines, events) = match target {
            DetailTarget::Campaign(id) => {
                let mut header = Vec::new();
                if let Some(c) = CampaignRepo::new(&conn).get(id)? {
                    let qs = QuestRepo::new(&conn).list_for_campaign(id)?;
                    let done = qs.iter().filter(|q| q.status == "completed").count();
                    header.push(format!("상태 {}  ·  진행 {}/{}", c.status, done, qs.len()));
                    header.push(format!("프롬프트: {}", truncate(c.prompt.trim(), 120)));
                    if let Some(rp) = &c.report_path {
                        header.push(format!("보고서: {rp}"));
                    }
                    header.push("─".repeat(40));
                }
                (format!("원정 #{id} 진행"), header, erepo.for_campaign(id, 200)?)
            }
            DetailTarget::Quest(id) => {
                let mut header = Vec::new();
                if let Some(q) = QuestRepo::new(&conn).get(id)? {
                    header.push(format!("상태 {}  ·  프로젝트 {}", q.status, q.project));
                    header.push(format!(
                        "브랜치 {}  ·  PR {}",
                        q.branch.as_deref().unwrap_or("-"),
                        q.pr_url.as_deref().unwrap_or("-")
                    ));
                    if let Some(wt) = &q.worktree_path {
                        header.push(format!("worktree {wt}"));
                    }
                    if let Some(pg) = &q.progress {
                        header.push(format!("진행: {}", truncate(pg.trim(), 120)));
                    }
                    header.push(format!("작업: {}", truncate(q.brief.trim(), 120)));
                    header.push("─".repeat(40));
                }
                (format!("모험 q{id} 진행"), header, erepo.for_quest(id, 200)?)
            }
        };
        if events.is_empty() {
            lines.push("(아직 기록된 진행이 없습니다)".to_string());
        } else {
            lines.extend(events.iter().map(format_event));
        }
        self.detail = Some(Detail { target, title, lines });
        Ok(())
    }
}

/// 두 상세 대상이 같은 항목을 가리키는지.
fn same_target(a: &DetailTarget, b: &DetailTarget) -> bool {
    matches!(
        (a, b),
        (DetailTarget::Campaign(x), DetailTarget::Campaign(y))
            | (DetailTarget::Quest(x), DetailTarget::Quest(y))
        if x == y
    )
}

/// 상세 타임라인 페이지 스크롤. down=true면 한 페이지 아래, false면 위.
/// 위로 스크롤하면 꼬리추적이 해제되고, 바닥에 닿으면 다시 꼬리추적으로 붙는다.
fn page_detail(app: &mut App, down: bool) {
    let (total, rows) = match &app.detail {
        Some(d) => (d.lines.len() as u16, app.detail_rows.max(1)),
        None => return,
    };
    let max = total.saturating_sub(rows);
    // 스크롤할 내용이 없으면(한 페이지 이하) 위/아래 모두 no-op — 꼬리추적을 끄지 않는다.
    // (이걸 빼면 짧은 타임라인에서 PgUp 이 follow 를 꺼버려, 이후 이벤트가 쌓여도 tail 이 멈춘다.)
    if max == 0 {
        return;
    }
    if down {
        if app.detail_follow {
            return; // 이미 바닥(꼬리추적)
        }
        app.detail_scroll = (app.detail_scroll + rows).min(max);
        if app.detail_scroll >= max {
            app.detail_follow = true;
        }
    } else if app.detail_follow {
        app.detail_follow = false;
        app.detail_scroll = max.saturating_sub(rows);
    } else {
        app.detail_scroll = app.detail_scroll.saturating_sub(rows);
    }
}

/// 선택 원정의 보고 가능 여부. `None`=원정 미선택, `Some(Ok(id))`=실행 가능,
/// `Some(Err(msg))`=경고(미완료). 순수 판정이라 단위테스트 가능(워커 없음).
fn report_target(app: &App) -> Option<Result<i64, String>> {
    let id = app.selected_campaign()?.id;
    let (done, total) = app
        .dash
        .campaign_progress
        .get(&id)
        .copied()
        .unwrap_or((0, 0));
    Some(if total > 0 && done == total {
        Ok(id)
    } else {
        Err(format!("⚠ 원정 #{id}: 완료 {done}/{total} — 모두 완료 후 보고 가능"))
    })
}

/// 모험지를 즉시(워커 없이) 등록하고 대시보드를 재조회한다. DB 쓰기뿐이라 빠르다.
fn add_project_inline(
    app: &mut App,
    name: &str,
    path: &str,
    base: &str,
    desc: Option<&str>,
) -> Result<()> {
    let conn = open_db(&app.db_path)?;
    ProjectRepo::new(&conn).add(name, path, base, desc)?;
    drop(conn);
    app.reload()?;
    Ok(())
}

/// 명령을 백그라운드 워커로 띄운다 (Running 중이면 무시 — 동시 1개).
fn spawn_worker(app: &mut App, cmd: Command, label: String) {
    if matches!(app.mode, Mode::Running) {
        return;
    }
    // Run 명령이면 그 원정을 진행 바 대상으로 (다른 명령은 진행 바 없음).
    app.running_campaign = if let Command::Run(id) = &cmd { Some(*id) } else { None };
    let (tx, rx) = mpsc::channel();
    let db = app.db_path.clone();
    // 워커에 취소 토큰을 넘기고, 같은 토큰을 App 에 보관 → 종료 시 cancel()로 자식 정리.
    let token = CancelToken::new();
    let worker_token = token.clone();
    app.cancel = Some(token);
    std::thread::spawn(move || {
        let msg = match dispatch(&db, cmd, worker_token) {
            Ok(s) => WorkerMsg::Done(s),
            Err(e) => WorkerMsg::Failed(e.to_string()),
        };
        let _ = tx.send(msg);
    });
    app.rx = Some(rx);
    app.running_label = Some(label);
    app.status = None;
    app.mode = Mode::Running;
}

/// 키 처리. quit이면 Ok(true).
fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    let code = key.code;
    // 도움말 오버레이가 열려있으면 아무 키나 눌러 닫는다.
    if app.help {
        app.help = false;
        return Ok(false);
    }
    // Shift/Alt+Enter → 개행 (터미널 keyboard enhancement 지원 시). 평범한 Enter → 제출.
    let newline = key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
    match &app.mode {
        // 실행 중: q/Esc/Ctrl-C → 워커 취소(자식 정리) 후 종료. 나머지 키는 무시.
        Mode::Running => {
            let ctrl_c =
                code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
            let wants_quit = ctrl_c || matches!(code, KeyCode::Char('q' | 'ㅂ') | KeyCode::Esc);
            if wants_quit && !app.quitting {
                if let Some(c) = &app.cancel {
                    c.cancel(); // 실행 중인 외부 CLI 자식을 즉시 정리(고아 방지).
                }
                app.quitting = true;
                app.quit_at = Some(now_ms());
                app.set_status("⏳ 중단하고 종료 중…".to_string());
            }
            Ok(false)
        }
        Mode::Input(kind) => {
            let kind = kind.clone();
            match code {
                KeyCode::Esc => {
                    app.mode = Mode::Normal;
                    app.input.clear();
                }
                KeyCode::Enter if newline => {
                    app.input.push('\n');
                }
                KeyCode::Enter => {
                    let text = app.input.trim().to_string();
                    if !text.is_empty() {
                        app.input.clear();
                        match kind {
                            InputKind::PlanPrompt => {
                                spawn_worker(app, Command::Plan(text), "campaign plan".to_string());
                            }
                            InputKind::ResumeAnswer { quest_id } => {
                                spawn_worker(
                                    app,
                                    Command::Resume { quest_id, answer: text },
                                    format!("quest resume q{quest_id}"),
                                );
                            }
                            InputKind::AddProject => {
                                // 워커 없이 즉시 DB에 등록 (빠른 작업).
                                match parse_project_form(&text) {
                                    Some((name, path, base, desc)) => {
                                        match add_project_inline(
                                            app,
                                            &name,
                                            &path,
                                            &base,
                                            desc.as_deref(),
                                        ) {
                                            Ok(()) => {
                                                // 결과가 보이도록 모험지 탭으로 이동 + 새 항목 선택.
                                                app.tab = Tab::Projects;
                                                app.select_key(Some(SelKey::Name(name.clone())));
                                                // 파싱 결과(브랜치·경로)를 토스트에 노출 → 공백 경로
                                                // 오파싱을 사용자가 즉시 알아챌 수 있다.
                                                app.set_status(format!(
                                                    "✅ 모험지 등록: {name} ({base}) → {path}"
                                                ));
                                            }
                                            Err(e) => {
                                                app.set_status(format!("⚠ 등록 실패: {e}"))
                                            }
                                        }
                                    }
                                    None => app.set_status(
                                        "⚠ 형식: 이름 경로 [브랜치] [설명…]".to_string(),
                                    ),
                                }
                                app.mode = Mode::Normal;
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    app.input.pop();
                }
                KeyCode::Char(c) => {
                    app.input.push(c);
                }
                _ => {}
            }
            Ok(false)
        }
        // 한글 IME 켠 상태에서도 동작하도록 두벌식 자모(같은 물리 키)를 함께 매핑한다.
        // q→ㅂ · p→ㅔ · r→ㄱ · t→ㅅ · j→ㅓ · k→ㅏ · d→ㅇ · x→ㅌ (한영 무관).
        Mode::Normal => {
            // raw mode 라 SIGINT 가 안 오므로 Ctrl-C 를 종료로 처리. 그 외 Ctrl/Alt+글자 조합은
            // 한 글자 명령(c=보고, x=실행 등)으로 오인되지 않도록 무시한다.
            if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                if key.modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                    return Ok(true);
                }
                return Ok(false);
            }
            match code {
                KeyCode::Char('q' | 'ㅂ') => return Ok(true),
                KeyCode::Esc => {
                    // 상세 뷰가 열려있으면 닫기, 아니면 종료.
                    if app.detail.is_some() {
                        app.detail = None;
                    } else {
                        return Ok(true);
                    }
                }
                KeyCode::Tab => {
                    app.detail = None;
                    app.switch_tab();
                }
                // Shift+Tab: 역방향 탭 전환 (docs 명시).
                KeyCode::BackTab => {
                    app.detail = None;
                    app.switch_tab_prev();
                }
                // 상세가 열려있으면 PageUp/Down·Home/End 로 타임라인 스크롤(꼬리추적 토글).
                KeyCode::PageDown if app.detail.is_some() => page_detail(app, true),
                KeyCode::PageUp if app.detail.is_some() => page_detail(app, false),
                KeyCode::End if app.detail.is_some() => app.reset_detail_scroll(),
                KeyCode::Home if app.detail.is_some() => {
                    app.detail_follow = false;
                    app.detail_scroll = 0;
                }
                KeyCode::Down | KeyCode::Char('j' | 'ㅓ') => {
                    app.next();
                    app.sync_detail_to_selection();
                }
                KeyCode::Up | KeyCode::Char('k' | 'ㅏ') => {
                    app.prev();
                    app.sync_detail_to_selection();
                }
                // Enter / d(ㅇ): 선택 항목 상세 뷰(events 타임라인) 토글
                KeyCode::Enter | KeyCode::Char('d' | 'ㅇ') => app.toggle_detail(),
                // x(ㅌ): 선택 원정 실행 (Campaigns 탭)
                KeyCode::Char('x' | 'ㅌ') => {
                    let id = app.selected_campaign().map(|c| c.id);
                    if let Some(id) = id {
                        spawn_worker(app, Command::Run(id), format!("campaign run #{id}"));
                    }
                }
                // c(ㅊ): 모든 모험이 완료된 원정을 모험의 서에 보고 (Campaigns 탭)
                KeyCode::Char('c' | 'ㅊ') => match report_target(app) {
                    Some(Ok(id)) => {
                        spawn_worker(app, Command::Report(id), format!("campaign report #{id}"))
                    }
                    Some(Err(msg)) => app.set_status(msg),
                    None => {}
                },
                // n(ㅜ): 다음 판단대기 모험으로 점프
                KeyCode::Char('n' | 'ㅜ') => app.jump_to_needs_input(),
                // ?: 도움말 오버레이
                KeyCode::Char('?') => app.help = true,
                // a(ㅁ): 모험지 등록 폼
                KeyCode::Char('a' | 'ㅁ') => {
                    app.mode = Mode::Input(InputKind::AddProject);
                    app.input.clear();
                }
                KeyCode::Char('p' | 'ㅔ') => {
                    app.mode = Mode::Input(InputKind::PlanPrompt);
                    app.input.clear();
                }
                KeyCode::Char('r' | 'ㄱ') => {
                    let qid = app.selected_quest().map(|q| q.id);
                    if let Some(qid) = qid {
                        app.mode = Mode::Input(InputKind::ResumeAnswer { quest_id: qid });
                        app.input.clear();
                    }
                }
                KeyCode::Char('t' | 'ㅅ') => {
                    let qid = app.selected_quest().map(|q| q.id);
                    if let Some(qid) = qid {
                        spawn_worker(app, Command::Triage(qid), format!("quest triage q{qid}"));
                    }
                }
                _ => {}
            }
            Ok(false)
        }
    }
}

/// 터미널 상태 복원을 RAII로 보장 (패닉·에러 경로 unwind 시 Drop으로 복원).
struct TerminalGuard {
    enhanced: bool,
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.enhanced {
            let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

/// TUI를 띄운다. `luida ui`에서 호출.
pub fn run(db_path: &Path) -> Result<()> {
    let mut conn = open_db(db_path)?;
    migrate(&mut conn)?;
    // 재시작 재조정 — 이전 실행이 강제 종료돼 'running'으로 남은 모험을 '중단(이어받기 가능)'으로
    // 되돌린다(이 머신의 죽은 runner 한정). 정상 종료는 워커가 이미 중단 처리했다.
    let _ = reconcile_interrupted_quests(&conn);
    let dash = Dashboard::load(&conn)?;
    drop(conn);
    let mut app = App::new(dash, db_path.to_path_buf());

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    // Shift/Alt+Enter 등 수정자+Enter 구분을 위해 keyboard enhancement (지원 터미널만).
    let enhanced = matches!(supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let _guard = TerminalGuard { enhanced };

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let result = run_loop(&mut terminal, &mut app);
    let _ = terminal.show_cursor();
    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        app.tick = app.tick.wrapping_add(1);
        // 결과 토스트 자동 소멸 (약 5초).
        if let Some(at) = app.status_at {
            if now_ms() - at > 5000 {
                app.status = None;
                app.status_at = None;
            }
        }

        terminal.draw(|f| draw(f, app))?;

        // 워커 완료 확인 (논블로킹).
        if let Some(rx) = &app.rx {
            match rx.try_recv() {
                Ok(msg) => {
                    let text = match msg {
                        WorkerMsg::Done(s) => format!("✅ {s}"),
                        WorkerMsg::Failed(e) => format!("⚠ 실패: {e}"),
                    };
                    if !app.quitting {
                        app.set_status(text);
                    }
                    app.mode = Mode::Normal;
                    app.running_label = None;
                    app.rx = None;
                    app.cancel = None;
                    // reload 실패는 비치명적으로 (다음 자동 갱신 틱이 반영). 결과 토스트·상태는
                    // 이미 적용됐으므로, 일시적 SQLite 오류로 TUI 전체를 무너뜨리지 않는다.
                    let _ = app.reload();
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !app.quitting {
                        app.set_status("⚠ 워커가 비정상 종료했습니다".to_string());
                    }
                    app.mode = Mode::Normal;
                    app.running_label = None;
                    app.rx = None;
                    app.cancel = None;
                }
            }
        }

        // 종료 요청 처리 — 워커가 정리되면(또는 유예 초과 시) 루프 종료.
        // 자식은 이미 cancel()로 kill 됐으므로, 워커가 안 끝나도 고아는 남지 않는다.
        if app.quitting {
            let timed_out = app.quit_at.is_some_and(|t| now_ms() - t > 3000);
            if app.rx.is_none() || timed_out {
                break;
            }
        }

        // 대시보드 자동 갱신 — 백그라운드 server/daemon/다른 프로세스의 변경을 반영.
        // 입력(모달) 중엔 건너뛰어 조용히 유지. 선택은 정체성으로 보존되므로 하이라이트가 튀지 않는다.
        if !matches!(app.mode, Mode::Input(_)) && now_ms() - app.last_reload_ms > RELOAD_INTERVAL_MS
        {
            let _ = app.reload();
        }

        // 상세 뷰가 열려있으면 events 재조회 (실시간 진행 반영). 실패는 다음 틱 재시도.
        if app.detail.is_some() {
            let _ = app.refresh_detail();
        }

        // 실행 중이면 원정 진행도 갱신 (진행 바). Running 아니면 숨김.
        app.progress = if matches!(app.mode, Mode::Running) {
            app.running_campaign
                .and_then(|cid| compute_progress(&app.db_path, cid).ok())
        } else {
            None
        };

        // 키 입력 폴링 (150ms 타임아웃 → 워커 진행 중에도 UI 갱신).
        // 조합 중 잦은 재렌더가 IME preedit 를 깨뜨려, 입력당 즉시 redraw 보다
        // "다음 draw 에서 반영" 방식이 한글 조합과 덜 충돌한다.
        if event::poll(Duration::from_millis(150))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if handle_key(app, key)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn draw(f: &mut Frame, app: &mut App) {
    // 진행 바가 있으면 헤더 아래에 한 구간 더 (헤더 / [진행바] / 본문 / 푸터).
    let chunks = if app.progress.is_some() {
        Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area())
    } else {
        Layout::vertical([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
            .split(f.area())
    };
    let (body_idx, footer_idx) = if app.progress.is_some() { (2, 3) } else { (1, 2) };

    // 헤더 — 탭 바 + 실행 상태
    let mut spans = vec![Span::styled("🍺 루이다  ", Style::default().fg(GOLD).bold())];
    if is_fake() {
        spans.push(Span::styled(
            " 🧪 데모 ",
            Style::default().fg(Color::Black).bg(Color::Rgb(0x6C, 0x8C, 0xFF)).bold(),
        ));
        spans.push(Span::raw(" "));
    }
    for t in Tab::all() {
        let active = t == app.tab;
        spans.push(Span::styled(
            format!(" {} ", t.title()),
            if active {
                Style::default().fg(Color::Black).bg(GOLD).bold()
            } else {
                Style::default().fg(DIM)
            },
        ));
        spans.push(Span::raw(" "));
    }
    if let Some(label) = &app.running_label {
        let dots = ".".repeat((app.tick / 2 % 4) as usize);
        spans.push(Span::styled(
            format!(" ⏳ {label}{dots} "),
            Style::default().fg(Color::Black).bg(GOLD).bold(),
        ));
    }
    if app.dash.pending_user_mail > 0 {
        spans.push(Span::styled(
            format!(" 🔔 판단대기 {} ", app.dash.pending_user_mail),
            Style::default().fg(Color::Black).bg(RED).bold(),
        ));
    }
    let header = Paragraph::new(Line::from(spans)).block(
        Block::default().borders(Borders::ALL).border_style(Style::default().fg(DIM)),
    );
    f.render_widget(header, chunks[0]);

    // 진행 바 — 실행 중일 때만 (헤더 아래 구간).
    if let Some(p) = &app.progress {
        let ratio = if p.total > 0 {
            (p.completed as f64 / p.total as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let label = format!(
            "원정 #{} · 완료 {}/{} · 실행중 {} · 대기 {} · 실패 {}",
            p.campaign_id, p.completed, p.total, p.running, p.needs_input, p.failed
        );
        let gauge = Gauge::default()
            .block(
                Block::default()
                    .title(" 진행도 ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(GOLD)),
            )
            .gauge_style(Style::default().fg(GREEN).bg(Color::Rgb(0x1e, 0x2d, 0x44)))
            .ratio(ratio)
            .label(label);
        f.render_widget(gauge, chunks[1]);
    }

    // 본문 — 상세 뷰가 열려있으면 좌(목록)/우(타임라인) 분할, 아니면 전체 폭 목록.
    // detail_rows(가시 줄 수)를 갱신해야 해서 detail 데이터를 먼저 복사해 borrow 해제.
    let detail_data = app
        .detail
        .as_ref()
        .map(|d| (d.title.clone(), d.lines.clone()));
    let list_area = if let Some((title, lines)) = detail_data {
        let body = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(chunks[body_idx]);
        let pane = body[1];
        // 테두리 2줄 제외한 가시 영역.
        let visible = pane.height.saturating_sub(2);
        app.detail_rows = visible;
        let total = lines.len() as u16;
        let max_scroll = total.saturating_sub(visible);
        let scroll = if app.detail_follow {
            max_scroll
        } else {
            app.detail_scroll.min(max_scroll)
        };
        let tag = if app.detail_follow { "tail" } else { "↕" };
        // wrap을 끄면 1줄=1행이라 scroll/꼬리추적 계산이 정확하다(긴 줄은 우측 클리핑).
        let panel = Paragraph::new(lines.join("\n"))
            .style(Style::default().fg(Color::White))
            .scroll((scroll, 0))
            .block(
                Block::default()
                    .title(format!(" {title} [{tag}] (PgUp/PgDn·Home·End·Esc) "))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(GREEN)),
            );
        f.render_widget(panel, pane);
        body[0]
    } else {
        chunks[body_idx]
    };

    let (items, empty_hint) = tab_items(app);
    if items.is_empty() {
        let empty = Paragraph::new(empty_hint)
            .style(Style::default().fg(DIM))
            .block(tab_block(app, 0));
        f.render_widget(empty, list_area);
    } else {
        let count = items.len();
        let list = List::new(items)
            .block(tab_block(app, count))
            .highlight_style(Style::default().bg(Color::Rgb(0x1e, 0x2d, 0x44)).fg(GOLD))
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, list_area, &mut app.state);
    }

    // 푸터 — 모드별 동적 힌트 / 상태 토스트
    let (footer_text, footer_color) = match &app.mode {
        Mode::Running => (
            format!(
                " ⏳ 실행 중: {} — q/Esc 중단하고 종료 ",
                app.running_label.as_deref().unwrap_or("")
            ),
            GOLD,
        ),
        Mode::Input(_) => (
            " Enter 제출 · Shift/Alt+Enter 개행 · Esc 취소 ".to_string(),
            GOLD,
        ),
        Mode::Normal => match &app.status {
            Some(s) => (
                format!(" {s}  ·  q 종료 "),
                if s.starts_with('⚠') { RED } else { GREEN },
            ),
            None => (
                " Tab 탭 · j/k 이동 · Enter/d 상세 · x 실행 · p 계획 · a 등록 · c 보고 · n 판단대기 · ? 도움말 · q 종료 "
                    .to_string(),
                DIM,
            ),
        },
    };
    let footer = Paragraph::new(Span::styled(footer_text, Style::default().fg(footer_color)));
    f.render_widget(footer, chunks[footer_idx]);

    // 입력 모달 (오버레이) — 멀티라인 지원
    if let Mode::Input(kind) = &app.mode {
        let label = match kind {
            InputKind::PlanPrompt => "원정 계획 — 프롬프트 (Shift/Alt+Enter 개행)",
            InputKind::ResumeAnswer { .. } => "모험 재개 — 답변 (Shift/Alt+Enter 개행)",
            InputKind::AddProject => "모험지 등록 — 이름 경로 [브랜치] [설명…]",
        };
        let body = format!("{}█", app.input);
        let rows = body.split('\n').count() as u16;
        let height = (rows + 2).clamp(3, f.area().height.max(3));
        let area = centered_rect(70, height, f.area());
        f.render_widget(Clear, area);
        let modal = Paragraph::new(body)
            .style(Style::default().fg(GREEN))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(format!(" {label} "))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(GOLD)),
            );
        f.render_widget(modal, area);
    }

    // 도움말 오버레이 (최상위)
    if app.help {
        let lines = [
            "  키 도움말",
            "",
            "  Tab / Shift+Tab  탭 전환 (모험지 / 원정 / 모험)",
            "  j / k            위 / 아래 이동",
            "  Enter / d        선택 항목 상세(타임라인) 토글",
            "  PgUp/PgDn        상세 스크롤 (Home 맨위 · End 꼬리추적 복귀)",
            "  x                원정 실행 (원정 탭)",
            "  p                새 원정 계획 (프롬프트 입력)",
            "  a                모험지 등록 (이름 경로 [브랜치] [설명])",
            "  c                완료 원정 보고 (모험의 서)",
            "  r                모험 재개 (답변 입력)",
            "  t                escalation triage",
            "  n                다음 판단대기 모험으로 점프",
            "  Esc              상세 / 도움말 닫기 (없으면 종료)",
            "  q                종료",
            "",
            "  실행 중 q/Esc    중단하고 종료 — 자식 정리, 모험은 x 로 이어받기",
            "",
            "  입력 중: Enter 제출 · Shift/Alt+Enter 개행 · Esc 취소",
            "  자동 갱신: 1.2초마다 대시보드 새로고침 (선택 유지)",
            "  한글 IME: q=ㅂ p=ㅔ r=ㄱ t=ㅅ j=ㅓ k=ㅏ d=ㅇ x=ㅌ n=ㅜ a=ㅁ c=ㅊ",
            "",
            "  아무 키나 눌러 닫기",
        ];
        let height = (lines.len() as u16 + 2).min(f.area().height.max(3));
        let area = centered_rect(64, height, f.area());
        f.render_widget(Clear, area);
        let help = Paragraph::new(lines.join("\n"))
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .title(" 🍺 루이다 도움말 ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(GOLD)),
            );
        f.render_widget(help, area);
    }
}

/// 화면 가운데에 가로 percent_x%, 세로 height줄짜리 영역.
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    // u32 로 곱해 넓은 터미널(width≥937 등)에서 u16 오버플로 패닉을 피한다.
    let w = (area.width as u32 * percent_x as u32 / 100) as u16;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect { x, y, width: w, height }
}

fn tab_block(app: &App, count: usize) -> Block<'static> {
    Block::default()
        .title(format!(" {} ({}) ", app.tab.title(), count))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GOLD))
}

/// 현재 탭의 ListItem들 + 빈 상태 힌트.
fn tab_items(app: &App) -> (Vec<ListItem<'static>>, String) {
    match app.tab {
        Tab::Projects => (
            app.dash
                .projects
                .iter()
                .map(|p| {
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{:<16}", p.name), Style::default().fg(GREEN).bold()),
                        Span::styled(format!("{:<10}", p.base_branch), Style::default().fg(GOLD)),
                        Span::styled(p.repo_path.clone(), Style::default().fg(DIM)),
                    ]))
                })
                .collect(),
            "등록된 모험지가 없습니다. a 키 또는 `luida project add` 로 등록하세요.".to_string(),
        ),
        Tab::Campaigns => (
            app.dash
                .campaigns
                .iter()
                .map(|c| {
                    let prog = app
                        .dash
                        .campaign_progress
                        .get(&c.id)
                        .map(|(done, total)| format!("{done}/{total}"))
                        .unwrap_or_default();
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("#{:<4}", c.id), Style::default().fg(DIM)),
                        Span::styled(format!("{:<12}", c.status), status_color(&c.status)),
                        Span::styled(format!("{prog:<6}"), Style::default().fg(GOLD)),
                        Span::styled(c.title.clone(), Style::default().fg(GREEN)),
                    ]))
                })
                .collect(),
            "진행 중인 원정이 없습니다. p 로 새 원정을 계획하세요.".to_string(),
        ),
        Tab::Quests => (
            app.dash
                .quests
                .iter()
                .map(|q| {
                    let prog = q.progress.clone().unwrap_or_default();
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("q{:<4}", q.id), Style::default().fg(DIM)),
                        Span::styled(format!("{:<12}", q.status), status_color(&q.status)),
                        Span::styled(format!("{:<14}", q.project), Style::default().fg(GOLD)),
                        Span::styled(
                            if prog.is_empty() { q.brief.clone() } else { prog },
                            Style::default().fg(DIM),
                        ),
                    ]))
                })
                .collect(),
            "진행 중인 모험이 없습니다.".to_string(),
        ),
    }
}

fn status_color(status: &str) -> Style {
    let c = match status {
        "completed" => GREEN,
        "failed" | "aborted" => RED,
        s if is_waiting(s) => Color::Rgb(0xFF, 0xA5, 0x00),
        "running" | "reviewing" => GOLD,
        _ => DIM,
    };
    Style::default().fg(c)
}

/// Event 1건 → 상세 타임라인 한 줄. payload(JSON/문자열)는 앞부분만 요약.
fn format_event(e: &luida_core::Event) -> String {
    let (icon, label) = match e.kind.as_str() {
        "campaign_planned" => ("📋", "계획"),
        "quest_dispatched" => ("⚙", "디스패치"),
        "quest_resumed" => ("▶", "재개"),
        "tool_use" => ("🔧", "도구"),
        "quest_completed" => ("✅", "완료"),
        "quest_needs_input" => ("⚠", "판단대기"),
        "quest_failed" => ("✗", "실패"),
        "escalation" => ("❓", "질문"),
        "trigger_dispatched" => ("🔗", "트리거"),
        other => ("·", other),
    };
    let p = e.payload.trim();
    let payload = if p.is_empty() || p == "{}" {
        String::new()
    } else {
        let s: String = p.chars().take(80).collect();
        format!("  {s}")
    };
    format!("{icon} {label} · {}{payload}", e.actor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use luida_core::{open_memory, NewCampaign, NewQuest};

    fn seeded() -> Connection {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign {
                title: "동기화",
                prompt: "p",
                plan_json: "{}",
                status: "running",
            })
            .unwrap();
        QuestRepo::new(&conn)
            .insert(NewQuest {
                campaign_id: Some(cid),
                project: "agora",
                brief: "작업",
                branch: None,
                status: "running",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        InmailRepo::new(&conn)
            .enqueue(luida_core::NewInmail {
                from_session: "luida",
                to_session: "@user",
                kind: "escalation",
                payload: "{}",
                reply_to: None,
                quest_id: None,
                campaign_id: Some(cid),
                dedupe_key: None,
            })
            .unwrap();
        conn
    }

    fn test_app(conn: &Connection) -> App {
        App::new(Dashboard::load(conn).unwrap(), PathBuf::from(":memory:"))
    }

    /// 테스트별 고유 임시 DB 경로 (parent 디렉터리 생성). 병렬 실행 충돌 방지.
    fn temp_db(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("luida-tui-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("t.db")
    }

    /// 파일 DB 위에 App 구성 (reload/refresh_detail/add 가 실제 경로를 쓰도록).
    fn file_app(path: &Path) -> App {
        let conn = open_db(path).unwrap();
        App::new(Dashboard::load(&conn).unwrap(), path.to_path_buf())
    }

    /// 마이그레이션된 빈 파일 DB 준비.
    fn fresh_file_db(tag: &str) -> PathBuf {
        let path = temp_db(tag);
        let mut conn = open_db(&path).unwrap();
        migrate(&mut conn).unwrap();
        path
    }

    /// 프로세스-전역 env 를 설정하고 Drop(패닉 unwind 포함) 시 제거하는 가드.
    /// 테스트가 assert 패닉해도 LUIDA_FAKE 등이 다른 테스트로 새지 않게 한다.
    struct EnvGuard(Vec<&'static str>);
    impl EnvGuard {
        fn set(vars: &[(&'static str, &str)]) -> Self {
            for (k, v) in vars {
                std::env::set_var(k, v);
            }
            EnvGuard(vars.iter().map(|(k, _)| *k).collect())
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for k in &self.0 {
                std::env::remove_var(k);
            }
        }
    }

    /// 수정자 없는 키 이벤트.
    fn ev(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// 수정자 포함 키 이벤트.
    fn ev_mod(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn dashboard_load_counts() {
        let conn = seeded();
        let d = Dashboard::load(&conn).unwrap();
        assert_eq!(d.projects.len(), 1);
        assert_eq!(d.campaigns.len(), 1);
        assert_eq!(d.quests.len(), 1);
        assert_eq!(d.pending_user_mail, 1);
        // seed: quest 1건(running) → 완료 0 / 전체 1
        let cid = d.campaigns[0].id;
        assert_eq!(d.campaign_progress.get(&cid), Some(&(0, 1)));
    }

    #[test]
    fn compute_progress_buckets_statuses_with_aliases() {
        // compute_progress 를 파일 DB 로 직접 호출 — 별칭 상태(reviewing/needs_approval/aborted)
        // 가 각각 running/needs_input/failed 로 합산되는지, catch-all(pending)은 무시되는지 검증.
        let db = fresh_file_db("progress");
        let cid;
        {
            let conn = open_db(&db).unwrap();
            ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
            cid = CampaignRepo::new(&conn)
                .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
                .unwrap();
            let mk = |status| NewQuest {
                campaign_id: Some(cid),
                project: "agora",
                brief: "b",
                branch: None,
                status,
                depends_on_quest_id: None,
                source_inmail_id: None,
            };
            let q = QuestRepo::new(&conn);
            q.insert(mk("completed")).unwrap();
            q.insert(mk("completed")).unwrap();
            q.insert(mk("running")).unwrap();
            q.insert(mk("reviewing")).unwrap(); // → running 버킷
            q.insert(mk("needs_input")).unwrap();
            q.insert(mk("needs_approval")).unwrap(); // → needs_input 버킷
            q.insert(mk("failed")).unwrap();
            q.insert(mk("aborted")).unwrap(); // → failed 버킷
            q.insert(mk("pending")).unwrap(); // catch-all: 어떤 버킷도 아님
        }
        let p = compute_progress(&db, cid).unwrap();
        assert_eq!(p.campaign_id, cid);
        assert_eq!(p.total, 9);
        assert_eq!(p.completed, 2);
        assert_eq!(p.running, 2); // running + reviewing
        assert_eq!(p.needs_input, 2); // needs_input + needs_approval
        assert_eq!(p.failed, 2); // failed + aborted
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn tab_switch_and_navigation() {
        let conn = seeded();
        let mut app = test_app(&conn);
        assert_eq!(app.tab, Tab::Projects);
        assert_eq!(app.state.selected(), Some(0));
        app.switch_tab();
        assert_eq!(app.tab, Tab::Campaigns);
        app.switch_tab();
        assert_eq!(app.tab, Tab::Quests);
        app.switch_tab();
        assert_eq!(app.tab, Tab::Projects);
        app.next();
        assert_eq!(app.state.selected(), Some(0)); // 1개라 그대로
        app.prev();
        assert_eq!(app.state.selected(), Some(0));
    }

    #[test]
    fn tab_items_render_rows() {
        let conn = seeded();
        let mut app = test_app(&conn);
        assert_eq!(tab_items(&app).0.len(), 1); // projects
        app.switch_tab();
        assert_eq!(tab_items(&app).0.len(), 1); // campaigns
        app.switch_tab();
        assert_eq!(tab_items(&app).0.len(), 1); // quests
    }

    #[test]
    fn empty_dashboard_has_no_selection() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        let app = test_app(&conn);
        assert_eq!(app.current_len(), 0);
        assert_eq!(app.state.selected(), None);
    }

    #[test]
    fn key_p_enters_plan_input_and_edits() {
        let conn = seeded();
        let mut app = test_app(&conn);
        assert!(!handle_key(&mut app, ev(KeyCode::Char('p'))).unwrap());
        assert!(matches!(app.mode, Mode::Input(InputKind::PlanPrompt)));
        handle_key(&mut app, ev(KeyCode::Char('a'))).unwrap();
        handle_key(&mut app, ev(KeyCode::Char('b'))).unwrap();
        assert_eq!(app.input, "ab");
        handle_key(&mut app, ev(KeyCode::Backspace)).unwrap();
        assert_eq!(app.input, "a");
        handle_key(&mut app, ev(KeyCode::Esc)).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.input, "");
    }

    #[test]
    fn shift_enter_inserts_newline_not_submit() {
        let conn = seeded();
        let mut app = test_app(&conn);
        handle_key(&mut app, ev(KeyCode::Char('p'))).unwrap();
        handle_key(&mut app, ev(KeyCode::Char('a'))).unwrap();
        // Shift+Enter → 개행, 제출 아님 (모드 유지)
        handle_key(&mut app, ev_mod(KeyCode::Enter, KeyModifiers::SHIFT)).unwrap();
        handle_key(&mut app, ev(KeyCode::Char('b'))).unwrap();
        assert_eq!(app.input, "a\nb");
        assert!(matches!(app.mode, Mode::Input(InputKind::PlanPrompt)));
        // Alt+Enter 도 개행
        handle_key(&mut app, ev_mod(KeyCode::Enter, KeyModifiers::ALT)).unwrap();
        assert_eq!(app.input, "a\nb\n");
    }

    #[test]
    fn key_r_on_quests_enters_resume_input() {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.switch_tab(); // Campaigns
        app.switch_tab(); // Quests
        handle_key(&mut app, ev(KeyCode::Char('r'))).unwrap();
        match &app.mode {
            Mode::Input(InputKind::ResumeAnswer { quest_id }) => assert!(*quest_id > 0),
            _ => panic!("resume 입력 모드가 아님"),
        }
    }

    #[test]
    fn key_q_quits() {
        let conn = seeded();
        let mut app = test_app(&conn);
        assert!(handle_key(&mut app, ev(KeyCode::Char('q'))).unwrap());
    }

    #[test]
    fn hangul_jamo_keys_work_in_normal_mode() {
        let conn = seeded();
        // 'ㅂ'(q 자리) → 종료
        let mut app = test_app(&conn);
        assert!(handle_key(&mut app, ev(KeyCode::Char('ㅂ'))).unwrap());
        // 'ㅔ'(p 자리) → 원정 계획 입력 모드
        let mut app2 = test_app(&conn);
        handle_key(&mut app2, ev(KeyCode::Char('ㅔ'))).unwrap();
        assert!(matches!(app2.mode, Mode::Input(InputKind::PlanPrompt)));
    }

    #[test]
    fn detail_toggle_open_close() {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.switch_tab(); // Campaigns (seed 에 원정 1건)
        assert!(app.detail.is_none());
        // Enter → 상세 열림
        handle_key(&mut app, ev(KeyCode::Enter)).unwrap();
        assert!(app.detail.is_some());
        // Esc → 닫힘 (종료 아님)
        assert!(!handle_key(&mut app, ev(KeyCode::Esc)).unwrap());
        assert!(app.detail.is_none());
        // 'd'(ㅇ 동치) 로도 토글
        handle_key(&mut app, ev(KeyCode::Char('d'))).unwrap();
        assert!(app.detail.is_some());
        handle_key(&mut app, ev(KeyCode::Char('d'))).unwrap();
        assert!(app.detail.is_none());
    }

    #[test]
    fn key_x_runs_selected_campaign() {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.switch_tab(); // Campaigns
        handle_key(&mut app, ev(KeyCode::Char('x'))).unwrap();
        assert!(matches!(app.mode, Mode::Running));
    }

    #[test]
    fn tab_switch_closes_detail() {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.switch_tab(); // Campaigns
        handle_key(&mut app, ev(KeyCode::Enter)).unwrap();
        assert!(app.detail.is_some());
        handle_key(&mut app, ev(KeyCode::Tab)).unwrap();
        assert!(app.detail.is_none());
    }

    #[test]
    fn help_overlay_toggles() {
        let conn = seeded();
        let mut app = test_app(&conn);
        handle_key(&mut app, ev(KeyCode::Char('?'))).unwrap();
        assert!(app.help);
        // 아무 키나 누르면 닫힘 (종료 아님)
        assert!(!handle_key(&mut app, ev(KeyCode::Char('j'))).unwrap());
        assert!(!app.help);
    }

    #[test]
    fn jump_to_needs_input_selects_quest() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
            .unwrap();
        let mk = |status| NewQuest {
            campaign_id: Some(cid),
            project: "agora",
            brief: "b",
            branch: None,
            status,
            depends_on_quest_id: None,
            source_inmail_id: None,
        };
        QuestRepo::new(&conn).insert(mk("running")).unwrap();
        QuestRepo::new(&conn).insert(mk("needs_input")).unwrap();

        let mut app = App::new(Dashboard::load(&conn).unwrap(), PathBuf::from(":memory:"));
        handle_key(&mut app, ev(KeyCode::Char('n'))).unwrap();
        assert_eq!(app.tab, Tab::Quests);
        let sel = app.state.selected().unwrap();
        assert_eq!(app.dash.quests[sel].status, "needs_input");
    }

    #[test]
    fn running_mode_ignores_nonquit_keys() {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.mode = Mode::Running;
        // Running 중엔 이동/탭 키는 무시 (즉시 종료도 아님).
        assert!(!handle_key(&mut app, ev(KeyCode::Tab)).unwrap());
        assert!(!handle_key(&mut app, ev(KeyCode::Char('j'))).unwrap());
        assert_eq!(app.tab, Tab::Projects);
        assert!(!app.quitting);
    }

    #[test]
    fn ctrl_c_quits_in_normal_mode() {
        // raw mode 라 SIGINT 가 안 오므로 Ctrl-C 를 종료로 처리.
        let conn = seeded();
        let mut app = test_app(&conn);
        assert!(handle_key(&mut app, ev_mod(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap());
    }

    #[test]
    fn ctrl_modified_letters_do_not_trigger_commands() {
        // Ctrl+x 가 원정 실행으로 오인되면 안 됨(글자 명령은 modifier 없을 때만).
        let conn = seeded();
        let mut app = test_app(&conn);
        app.switch_tab(); // Campaigns
        assert!(!handle_key(&mut app, ev_mod(KeyCode::Char('x'), KeyModifiers::CONTROL)).unwrap());
        assert!(matches!(app.mode, Mode::Normal), "Ctrl+x 가 실행을 트리거하면 안 됨");
        // Ctrl+p 도 계획 입력 모드로 안 들어감.
        assert!(!handle_key(&mut app, ev_mod(KeyCode::Char('p'), KeyModifiers::CONTROL)).unwrap());
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn running_q_cancels_worker_and_requests_quit() {
        // 실행 중 q → 워커 취소(자식 정리) + 종료 요청. 루프는 워커 정리 후 빠져나간다.
        let conn = seeded();
        let mut app = test_app(&conn);
        let token = CancelToken::new();
        app.cancel = Some(token.clone());
        app.mode = Mode::Running;
        assert!(!handle_key(&mut app, ev(KeyCode::Char('q'))).unwrap()); // 즉시 종료 아님
        assert!(app.quitting);
        assert!(token.is_cancelled(), "워커 취소 토큰이 켜져야 함");
        // Esc·Ctrl-C 도 동일 경로 (이미 quitting이면 재취소 안 함).
    }

    #[test]
    fn running_esc_also_requests_cancel_quit() {
        let conn = seeded();
        let mut app = test_app(&conn);
        let token = CancelToken::new();
        app.cancel = Some(token.clone());
        app.mode = Mode::Running;
        handle_key(&mut app, ev(KeyCode::Esc)).unwrap();
        assert!(app.quitting);
        assert!(token.is_cancelled());
    }

    #[test]
    fn dispatch_plan_then_run_then_report_fake() {
        // LUIDA_FAKE 결정적 런타임으로 plan→run→report 전체 라이프사이클 검증.
        // 환경변수 변경 테스트는 이 하나로 모아 병렬 실행 시 race 를 피한다.
        // 고유 temp 디렉터리(temp_db) + EnvGuard 로, assert 패닉 시에도 env 가 새지 않는다.
        let db = temp_db("fake");
        let dir = db.parent().unwrap().to_path_buf();
        let mem = dir.join("memory");
        let _env = EnvGuard::set(&[
            ("LUIDA_FAKE", "1"),
            // 보고서 vault 를 temp 로 격리 (홈 디렉터리 오염 방지).
            ("LUIDA_MEMORY_DIR", mem.to_str().unwrap()),
        ]);
        {
            let (conn, _) = open_ready(&db).unwrap();
            ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
            ProjectRepo::new(&conn).add("admin", "/b", "main", None).unwrap();
        }
        let s = dispatch(&db, Command::Plan("agora와 admin 정렬".into()), CancelToken::new()).unwrap();
        assert!(s.contains("계획"), "plan 결과: {s}");
        let s2 = dispatch(&db, Command::Run(1), CancelToken::new()).unwrap();
        assert!(s2.contains("실행") && s2.contains("완료"), "run 결과: {s2}");
        // 모든 quest 완료 → 보고서 기록 가능.
        let s3 = dispatch(&db, Command::Report(1), CancelToken::new()).unwrap();
        assert!(s3.contains("보고"), "report 결과: {s3}");
        // report 로 원정이 completed 마감됐는지 확인.
        let conn = open_db(&db).unwrap();
        let c = CampaignRepo::new(&conn).get(1).unwrap().unwrap();
        assert_eq!(c.status, "completed");
        assert!(c.report_path.is_some());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 새 기능 테스트 ───────────────────────────────────────────────────────────

    #[test]
    fn parse_project_form_variants() {
        assert_eq!(
            parse_project_form("agora /repos/agora"),
            Some(("agora".into(), "/repos/agora".into(), "main".into(), None))
        );
        assert_eq!(
            parse_project_form("agora /r develop"),
            Some(("agora".into(), "/r".into(), "develop".into(), None))
        );
        assert_eq!(
            parse_project_form("agora /r develop 커뮤니티 웹 서비스"),
            Some((
                "agora".into(),
                "/r".into(),
                "develop".into(),
                Some("커뮤니티 웹 서비스".into())
            ))
        );
        // 여분 공백 흡수
        assert_eq!(
            parse_project_form("   a    /b   "),
            Some(("a".into(), "/b".into(), "main".into(), None))
        );
        // 이름·경로 필수
        assert_eq!(parse_project_form(""), None);
        assert_eq!(parse_project_form("onlyname"), None);
    }

    #[test]
    fn truncate_handles_multibyte() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("가나다라", 2), "가나…");
        assert_eq!(truncate("가나", 2), "가나"); // 경계: 같으면 그대로
    }

    #[test]
    fn tab_prev_cycles_backwards() {
        assert_eq!(Tab::Projects.prev(), Tab::Quests);
        assert_eq!(Tab::Quests.prev(), Tab::Campaigns);
        assert_eq!(Tab::Campaigns.prev(), Tab::Projects);
    }

    #[test]
    fn backtab_switches_to_prev_tab() {
        let conn = seeded();
        let mut app = test_app(&conn);
        assert_eq!(app.tab, Tab::Projects);
        handle_key(&mut app, ev(KeyCode::BackTab)).unwrap();
        assert_eq!(app.tab, Tab::Quests);
        handle_key(&mut app, ev(KeyCode::BackTab)).unwrap();
        assert_eq!(app.tab, Tab::Campaigns);
    }

    #[test]
    fn same_target_compares_identity() {
        assert!(same_target(&DetailTarget::Campaign(1), &DetailTarget::Campaign(1)));
        assert!(!same_target(&DetailTarget::Campaign(1), &DetailTarget::Campaign(2)));
        assert!(same_target(&DetailTarget::Quest(5), &DetailTarget::Quest(5)));
        assert!(!same_target(&DetailTarget::Quest(5), &DetailTarget::Quest(6)));
        // 종류가 다르면 항상 false
        assert!(!same_target(&DetailTarget::Campaign(1), &DetailTarget::Quest(1)));
    }

    /// page_detail/스크롤용 더미 상세 (100줄).
    fn app_with_detail(lines: usize, rows: u16) -> App {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.detail = Some(Detail {
            target: DetailTarget::Quest(1),
            title: "t".into(),
            lines: (0..lines).map(|i| format!("line {i}")).collect(),
        });
        app.detail_rows = rows;
        app.detail_follow = true;
        app.detail_scroll = 0;
        app
    }

    #[test]
    fn page_detail_scroll_and_follow_transitions() {
        let mut app = app_with_detail(100, 10); // max_scroll = 90
        // 꼬리추적 중 PgDn 은 무시 (이미 바닥).
        page_detail(&mut app, true);
        assert!(app.detail_follow);
        assert_eq!(app.detail_scroll, 0);
        // PgUp → 꼬리추적 해제 + 바닥에서 한 페이지 위 (90-10=80).
        page_detail(&mut app, false);
        assert!(!app.detail_follow);
        assert_eq!(app.detail_scroll, 80);
        // 한 번 더 위로 (70).
        page_detail(&mut app, false);
        assert_eq!(app.detail_scroll, 70);
        // 아래로 (80).
        page_detail(&mut app, true);
        assert_eq!(app.detail_scroll, 80);
        assert!(!app.detail_follow);
        // 아래로 → 바닥(90)에 닿으면 다시 꼬리추적.
        page_detail(&mut app, true);
        assert_eq!(app.detail_scroll, 90);
        assert!(app.detail_follow);
    }

    #[test]
    fn detail_scroll_keys_routed_only_when_open() {
        let mut app = app_with_detail(100, 10); // max_scroll = 90
        // End → 꼬리추적 복귀.
        app.detail_follow = false;
        app.detail_scroll = 30;
        handle_key(&mut app, ev(KeyCode::End)).unwrap();
        assert!(app.detail_follow);
        assert_eq!(app.detail_scroll, 0);
        // Home → 최상단 고정.
        handle_key(&mut app, ev(KeyCode::Home)).unwrap();
        assert!(!app.detail_follow);
        assert_eq!(app.detail_scroll, 0);
        // PgUp 은 꼬리추적 상태에서 한 페이지 위로 — 구체적 오프셋(max-rows=80)까지 검증.
        app.detail_follow = true;
        app.detail_scroll = 0;
        handle_key(&mut app, ev(KeyCode::PageUp)).unwrap();
        assert!(!app.detail_follow);
        assert_eq!(app.detail_scroll, 80);
        // PgDn 으로 한 페이지 아래.
        handle_key(&mut app, ev(KeyCode::PageDown)).unwrap();
        assert_eq!(app.detail_scroll, 90); // 바닥 → 다시 꼬리추적
        assert!(app.detail_follow);
        // 상세를 닫으면 PgDn 은 무시(패닉/변화 없음).
        app.detail = None;
        let before = app.tab;
        handle_key(&mut app, ev(KeyCode::PageDown)).unwrap();
        assert_eq!(app.tab, before);
    }

    #[test]
    fn page_detail_short_timeline_keeps_follow() {
        // 타임라인이 한 페이지보다 짧으면(max==0) PgUp 도 꼬리추적을 끄면 안 된다.
        let mut app = app_with_detail(3, 10);
        assert!(app.detail_follow);
        page_detail(&mut app, false); // PgUp
        assert!(app.detail_follow, "짧은 타임라인 PgUp 이 follow 를 끔");
        assert_eq!(app.detail_scroll, 0);
        page_detail(&mut app, true); // PgDn
        assert!(app.detail_follow);
        assert_eq!(app.detail_scroll, 0);
    }

    #[test]
    fn reload_preserves_selection_when_list_reorders() {
        let db = fresh_file_db("reorder");
        let conn = open_db(&db).unwrap();
        ProjectRepo::new(&conn).add("p", "/p", "main", None).unwrap();
        let crepo = CampaignRepo::new(&conn);
        let a = crepo.insert(NewCampaign { title: "A", prompt: "p", plan_json: "{}", status: "running" }).unwrap();
        let b = crepo.insert(NewCampaign { title: "B", prompt: "p", plan_json: "{}", status: "running" }).unwrap();
        // 결정적 정렬: updated_at 직접 설정 → 처음엔 [B(2000), A(1000)].
        conn.execute("UPDATE campaigns SET updated_at = 1000 WHERE id = ?1", [a]).unwrap();
        conn.execute("UPDATE campaigns SET updated_at = 2000 WHERE id = ?1", [b]).unwrap();

        let mut app = file_app(&db);
        app.switch_tab(); // Campaigns
        // A 선택 (현재 index 1).
        let a_idx = app.dash.campaigns.iter().position(|c| c.id == a).unwrap();
        app.state.select(Some(a_idx));
        assert_eq!(app.dash.campaigns[a_idx].id, a);
        // A 를 최신으로 → 재정렬되면 [A(3000), B(2000)].
        conn.execute("UPDATE campaigns SET updated_at = 3000 WHERE id = ?1", [a]).unwrap();
        app.reload().unwrap();
        // 선택은 여전히 A (인덱스가 바뀌어도 정체성 유지).
        let sel = app.state.selected().unwrap();
        assert_eq!(app.dash.campaigns[sel].id, a);
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn reload_clamps_when_selection_removed() {
        let db = fresh_file_db("removed");
        {
            let conn = open_db(&db).unwrap();
            let p = ProjectRepo::new(&conn);
            p.add("aaa", "/a", "main", None).unwrap();
            p.add("bbb", "/b", "main", None).unwrap();
            p.add("ccc", "/c", "main", None).unwrap();
        }
        let mut app = file_app(&db); // Projects 탭, 정렬 [aaa,bbb,ccc]
        app.state.select(Some(2)); // ccc 선택
        // ccc 제거 → 키 사라짐.
        open_db(&db).unwrap().execute("DELETE FROM projects WHERE name='ccc'", []).unwrap();
        app.reload().unwrap();
        // fallback: 직전 인덱스(2)를 len-1(=1)로 보정.
        assert_eq!(app.state.selected(), Some(1));
        assert_eq!(app.dash.projects.len(), 2);
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn add_project_via_input_flow() {
        let db = fresh_file_db("addproj");
        let mut app = file_app(&db);
        assert_eq!(app.dash.projects.len(), 0);
        // 일부러 다른 탭에서 시작 — 등록 후 모험지 탭으로 이동하는지 확인.
        app.switch_tab(); // Campaigns
        // a → AddProject 입력 모드 (탭 무관 전역).
        handle_key(&mut app, ev(KeyCode::Char('a'))).unwrap();
        assert!(matches!(app.mode, Mode::Input(InputKind::AddProject)));
        // 폼을 직접 채우고 Enter 제출.
        app.input = "agora /repos/agora develop 커뮤니티".to_string();
        handle_key(&mut app, ev(KeyCode::Enter)).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.dash.projects.len(), 1);
        let p = &app.dash.projects[0];
        assert_eq!(p.name, "agora");
        assert_eq!(p.repo_path, "/repos/agora");
        assert_eq!(p.base_branch, "develop");
        assert_eq!(p.description.as_deref(), Some("커뮤니티"));
        // 결과가 보이도록 모험지 탭으로 이동 + 새 항목 선택.
        assert_eq!(app.tab, Tab::Projects);
        let sel = app.state.selected().unwrap();
        assert_eq!(app.dash.projects[sel].name, "agora");
        // 토스트에 파싱된 브랜치·경로가 노출되어 오파싱을 알아챌 수 있다.
        let toast = app.status.as_deref().unwrap();
        assert!(toast.contains("등록") && toast.contains("develop") && toast.contains("/repos/agora"));
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn jump_targets_needs_approval_too() {
        // 'n' 점프는 needs_input 뿐 아니라 needs_approval(같은 판단대기 클래스)도 잡아야 한다.
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
            .unwrap();
        let mk = |status| NewQuest {
            campaign_id: Some(cid),
            project: "agora",
            brief: "b",
            branch: None,
            status,
            depends_on_quest_id: None,
            source_inmail_id: None,
        };
        QuestRepo::new(&conn).insert(mk("running")).unwrap();
        QuestRepo::new(&conn).insert(mk("needs_approval")).unwrap();
        let mut app = App::new(Dashboard::load(&conn).unwrap(), PathBuf::from(":memory:"));
        handle_key(&mut app, ev(KeyCode::Char('n'))).unwrap();
        assert_eq!(app.tab, Tab::Quests);
        let sel = app.state.selected().unwrap();
        assert!(is_waiting(&app.dash.quests[sel].status));
    }

    #[test]
    fn add_project_invalid_form_warns() {
        let db = fresh_file_db("addbad");
        let mut app = file_app(&db);
        handle_key(&mut app, ev(KeyCode::Char('a'))).unwrap();
        app.input = "onlyname".to_string(); // 경로 누락
        handle_key(&mut app, ev(KeyCode::Enter)).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.dash.projects.len(), 0);
        assert!(app.status.as_deref().unwrap().contains("형식"));
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn detail_quest_header_shows_metadata() {
        let db = fresh_file_db("qdetail");
        {
            let conn = open_db(&db).unwrap();
            ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
            let cid = CampaignRepo::new(&conn)
                .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
                .unwrap();
            let qid = QuestRepo::new(&conn)
                .insert(NewQuest {
                    campaign_id: Some(cid),
                    project: "agora",
                    brief: "스키마 정렬 작업",
                    branch: None,
                    status: "running",
                    depends_on_quest_id: None,
                    source_inmail_id: None,
                })
                .unwrap();
            QuestRepo::new(&conn).set_worktree(qid, "luida/q1", "/wt/q1").unwrap();
        }
        let mut app = file_app(&db);
        app.switch_tab(); // Campaigns
        app.switch_tab(); // Quests
        handle_key(&mut app, ev(KeyCode::Enter)).unwrap(); // 상세 열기
        let d = app.detail.as_ref().unwrap();
        let joined = d.lines.join("\n");
        assert!(joined.contains("상태 running"), "헤더: {joined}");
        assert!(joined.contains("agora"));
        assert!(joined.contains("luida/q1")); // 브랜치
        assert!(joined.contains("worktree /wt/q1"));
        assert!(joined.contains("스키마 정렬 작업"));
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn detail_campaign_header_shows_progress() {
        let db = fresh_file_db("cdetail");
        {
            let conn = open_db(&db).unwrap();
            ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
            let cid = CampaignRepo::new(&conn)
                .insert(NewCampaign { title: "동기화", prompt: "agora 정렬", plan_json: "{}", status: "running" })
                .unwrap();
            let mk = |status| NewQuest {
                campaign_id: Some(cid),
                project: "agora",
                brief: "b",
                branch: None,
                status,
                depends_on_quest_id: None,
                source_inmail_id: None,
            };
            QuestRepo::new(&conn).insert(mk("completed")).unwrap();
            QuestRepo::new(&conn).insert(mk("running")).unwrap();
        }
        let mut app = file_app(&db);
        app.switch_tab(); // Campaigns
        handle_key(&mut app, ev(KeyCode::Enter)).unwrap();
        let joined = app.detail.as_ref().unwrap().lines.join("\n");
        assert!(joined.contains("진행 1/2"), "헤더: {joined}");
        assert!(joined.contains("agora 정렬")); // 프롬프트
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn report_target_warns_when_campaign_incomplete() {
        // seeded: quest 1건 running → 진행 0/1 → 보고 불가 경고.
        let conn = seeded();
        let mut app = test_app(&conn);
        app.switch_tab(); // Campaigns
        match report_target(&app) {
            Some(Err(msg)) => assert!(msg.contains("완료 0/1"), "msg: {msg}"),
            other => panic!("경고를 기대: {other:?}"),
        }
        // 키 경로도 동일: 워커 안 띄우고 경고 토스트만.
        handle_key(&mut app, ev(KeyCode::Char('c'))).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.status.as_deref().unwrap().contains("완료"));
    }

    #[test]
    fn report_target_ok_when_all_quests_complete() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        let cid = CampaignRepo::new(&conn)
            .insert(NewCampaign { title: "t", prompt: "p", plan_json: "{}", status: "running" })
            .unwrap();
        QuestRepo::new(&conn)
            .insert(NewQuest {
                campaign_id: Some(cid),
                project: "agora",
                brief: "b",
                branch: None,
                status: "completed",
                depends_on_quest_id: None,
                source_inmail_id: None,
            })
            .unwrap();
        let mut app = test_app(&conn);
        app.switch_tab(); // Campaigns
        // 진행 1/1 → Ok(id).
        assert_eq!(report_target(&app), Some(Ok(cid)));
    }

    #[test]
    fn report_target_none_off_campaigns_tab() {
        let conn = seeded();
        let app = test_app(&conn); // Projects 탭
        assert!(report_target(&app).is_none());
    }

    #[test]
    fn auto_reload_picks_up_external_insert() {
        // 대시보드 자동 갱신: 다른 연결(=백그라운드 프로세스 모사)이 추가한 항목이 reload 로 반영.
        let db = fresh_file_db("autoreload");
        {
            let conn = open_db(&db).unwrap();
            ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
        }
        let mut app = file_app(&db);
        assert_eq!(app.dash.projects.len(), 1);
        // 외부 연결(=백그라운드 프로세스 모사)이 새 모험지 등록.
        ProjectRepo::new(&open_db(&db).unwrap()).add("admin", "/b", "main", None).unwrap();
        app.reload().unwrap();
        assert_eq!(app.dash.projects.len(), 2);
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }
}
