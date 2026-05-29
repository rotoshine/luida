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
use luida_core::{
    is_fake, migrate, now_ms, open_db, open_ready, Campaign, Connection, EventRepo, InmailRepo,
    Project, Quest, CampaignRepo, ProjectRepo, QuestRepo,
};
use luida_planner::{plan_campaign, run_campaign};
use luida_runtimes::make_factory;
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
}

/// 사용자가 TUI에서 내릴 수 있는 에이전트 명령.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Plan(String),
    Run(i64),
    Resume { quest_id: i64, answer: String },
    Triage(i64),
}

/// 텍스트 입력이 필요한 명령의 입력 모드.
#[derive(Clone, Debug, PartialEq)]
enum InputKind {
    PlanPrompt,
    ResumeAnswer { quest_id: i64 },
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
            "needs_input" | "needs_approval" => p.needs_input += 1,
            "failed" | "aborted" => p.failed += 1,
            _ => {}
        }
    }
    Ok(p)
}

/// 명령을 동기 실행하고 결과 요약을 반환. 워커 스레드와 테스트에서 호출한다.
/// factory/worktree/conn 은 이 함수 안에서 생성 → 스레드로 넘길 값은 db_path·파라미터뿐.
pub fn dispatch(db_path: &Path, cmd: Command) -> Result<String> {
    let (mut conn, cfg) = open_ready(db_path)?;
    match cmd {
        Command::Plan(prompt) => {
            let cid = plan_campaign(&mut conn, &cfg, &prompt, make_factory())?;
            Ok(format!("원정 #{cid} 계획 완료"))
        }
        Command::Run(id) => {
            let report = run_campaign(&mut conn, &cfg, id, make_worktree().as_ref(), make_factory())?;
            let trig = if report.triggered > 0 {
                format!(" / 트리거 {}", report.triggered)
            } else {
                String::new()
            };
            Ok(format!(
                "원정 #{id} 실행 — 완료 {} / 대기 {} / 실패 {}{trig}",
                report.completed.len(),
                report.needs_input.len(),
                report.failed.len()
            ))
        }
        Command::Resume { quest_id, answer } => {
            let out = resume_quest(&mut conn, &cfg, quest_id, &answer, make_factory())?;
            Ok(format!("q{quest_id} 재개 → {out:?}"))
        }
        Command::Triage(id) => {
            let d = triage_escalation(&mut conn, &cfg, id, make_factory())?;
            Ok(format!(
                "q{id} triage — 사용자필요 {} · {}",
                if d.ask_user { "예" } else { "아니오" },
                d.reason
            ))
        }
    }
}

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
    /// 도움말 오버레이 표시 여부 (? 토글).
    help: bool,
    /// status 토스트가 설정된 시각(ms). 일정 시간 후 자동 소멸.
    status_at: Option<i64>,
    /// 렌더 틱 카운터 (스피너 애니메이션용).
    tick: u64,
    /// 실행 중인 원정 id (Run 명령일 때만). 진행 바 대상.
    running_campaign: Option<i64>,
    /// 실행 중 원정 진행도 (폴링으로 갱신, Running 때만 Some).
    progress: Option<Progress>,
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
            help: false,
            status_at: None,
            tick: 0,
            running_campaign: None,
            progress: None,
        };
        app.reset_selection();
        app
    }

    /// status 토스트를 설정하고 시각을 기록 (자동 소멸 타이머용).
    fn set_status(&mut self, msg: String) {
        self.status = Some(msg);
        self.status_at = Some(now_ms());
    }

    /// 다음 needs_input(판단대기) 모험으로 점프 (Quests 탭 + 선택). 없으면 무시.
    fn jump_to_needs_input(&mut self) {
        if let Some(i) = self.dash.quests.iter().position(|q| q.status == "needs_input") {
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

    /// 명령 후 별도 read connection으로 대시보드 재조회 + 선택 인덱스 보정.
    fn reload(&mut self) -> Result<()> {
        let conn = open_db(&self.db_path)?;
        self.dash = Dashboard::load(&conn)?;
        let len = self.current_len();
        if len == 0 {
            self.state.select(None);
        } else if self.state.selected().is_none_or(|i| i >= len) {
            self.state.select(Some(len - 1));
        }
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

    /// 상세 뷰 토글: 닫혀있으면 현재 선택으로 열고, 열려있으면 닫는다.
    fn toggle_detail(&mut self) {
        if self.detail.is_some() {
            self.detail = None;
            return;
        }
        if let Some(target) = self.target_for_selection() {
            self.detail = Some(Detail { target, title: String::new(), lines: Vec::new() });
            let _ = self.refresh_detail(); // 조회 실패해도 빈 상세로 열림
        }
    }

    /// 상세가 열려있을 때 j/k 이동 시 현재 선택 항목으로 대상을 갱신.
    fn sync_detail_to_selection(&mut self) {
        if self.detail.is_none() {
            return;
        }
        match self.target_for_selection() {
            Some(target) => {
                self.detail = Some(Detail { target, title: String::new(), lines: Vec::new() });
                let _ = self.refresh_detail();
            }
            None => self.detail = None,
        }
    }

    /// 상세 대상의 events 타임라인을 별도 read conn 으로 재조회 (실시간 폴링).
    fn refresh_detail(&mut self) -> Result<()> {
        let target = match &self.detail {
            Some(d) => d.target.clone(),
            None => return Ok(()),
        };
        let conn = open_db(&self.db_path)?;
        let repo = EventRepo::new(&conn);
        let (title, events) = match target {
            DetailTarget::Campaign(id) => (format!("원정 #{id} 진행"), repo.for_campaign(id, 200)?),
            DetailTarget::Quest(id) => (format!("모험 q{id} 진행"), repo.for_quest(id, 200)?),
        };
        let lines = if events.is_empty() {
            vec!["(아직 기록된 진행이 없습니다)".to_string()]
        } else {
            events.iter().map(format_event).collect()
        };
        self.detail = Some(Detail { target, title, lines });
        Ok(())
    }
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
    std::thread::spawn(move || {
        let msg = match dispatch(&db, cmd) {
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
        // 실행 중엔 입력 무시 (완료까지 대기). 채널 수신 시 자동 해제.
        Mode::Running => Ok(false),
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
                // n(ㅜ): 다음 판단대기 모험으로 점프
                KeyCode::Char('n' | 'ㅜ') => app.jump_to_needs_input(),
                // ?: 도움말 오버레이
                KeyCode::Char('?') => app.help = true,
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
                    app.set_status(text);
                    app.mode = Mode::Normal;
                    app.running_label = None;
                    app.rx = None;
                    app.reload()?;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    app.set_status("⚠ 워커가 비정상 종료했습니다".to_string());
                    app.mode = Mode::Normal;
                    app.running_label = None;
                    app.rx = None;
                }
            }
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
    let list_area = if let Some(detail) = &app.detail {
        let body = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(chunks[body_idx]);
        let panel = Paragraph::new(detail.lines.join("\n"))
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(format!(" {} (Esc 닫기) ", detail.title))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(GREEN)),
            );
        f.render_widget(panel, body[1]);
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
                " ⏳ 실행 중: {} — 완료까지 대기 ",
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
                " Tab 탭 · j/k 이동 · Enter/d 상세 · x 실행 · p 계획 · n 판단대기 · ? 도움말 · q 종료 "
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
            "  Tab        탭 전환 (모험지 / 원정 / 모험)",
            "  j / k      위 / 아래 이동",
            "  Enter / d  선택 항목 상세(타임라인) 토글",
            "  x          원정 실행 (원정 탭)",
            "  p          새 원정 계획 (프롬프트 입력)",
            "  r          모험 재개 (답변 입력)",
            "  t          escalation triage",
            "  n          다음 판단대기 모험으로 점프",
            "  Esc        상세 / 도움말 닫기 (없으면 종료)",
            "  q          종료",
            "",
            "  입력 중: Enter 제출 · Shift/Alt+Enter 개행 · Esc 취소",
            "  한글 IME: q=ㅂ p=ㅔ r=ㄱ t=ㅅ j=ㅓ k=ㅏ d=ㅇ x=ㅌ n=ㅜ",
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
    let w = area.width * percent_x / 100;
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
            "등록된 모험지가 없습니다. `luida project add` 로 등록하세요.".to_string(),
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
        "needs_input" | "needs_approval" => Color::Rgb(0xFF, 0xA5, 0x00),
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
    fn compute_progress_counts_by_status() {
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
        QuestRepo::new(&conn).insert(mk("completed")).unwrap();
        QuestRepo::new(&conn).insert(mk("completed")).unwrap();
        QuestRepo::new(&conn).insert(mk("running")).unwrap();
        QuestRepo::new(&conn).insert(mk("needs_input")).unwrap();
        // compute_progress 는 db_path 기반이라 인메모리로는 못 부르므로 분포 로직만 간접 확인:
        let qs = QuestRepo::new(&conn).list_for_campaign(cid).unwrap();
        assert_eq!(qs.len(), 4);
        assert_eq!(qs.iter().filter(|q| q.status == "completed").count(), 2);
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
    fn running_mode_ignores_keys() {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.mode = Mode::Running;
        // Running 중엔 q도 종료 안 됨, 탭도 안 바뀜.
        assert!(!handle_key(&mut app, ev(KeyCode::Char('q'))).unwrap());
        assert!(!handle_key(&mut app, ev(KeyCode::Tab)).unwrap());
        assert_eq!(app.tab, Tab::Projects);
    }

    #[test]
    fn dispatch_plan_then_run_fake() {
        // LUIDA_FAKE 결정적 런타임으로 plan→run 실행 검증.
        std::env::set_var("LUIDA_FAKE", "1");
        let dir = std::env::temp_dir().join(format!("luida-tui-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("t.db");
        {
            let (conn, _) = open_ready(&db).unwrap();
            ProjectRepo::new(&conn).add("agora", "/a", "main", None).unwrap();
            ProjectRepo::new(&conn).add("admin", "/b", "main", None).unwrap();
        }
        let s = dispatch(&db, Command::Plan("agora와 admin 정렬".into())).unwrap();
        assert!(s.contains("계획"), "plan 결과: {s}");
        let s2 = dispatch(&db, Command::Run(1)).unwrap();
        assert!(s2.contains("실행") && s2.contains("완료"), "run 결과: {s2}");
        std::env::remove_var("LUIDA_FAKE");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
