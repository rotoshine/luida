//! luida-tui — ratatui 기반 TUI 대시보드 + 에이전트 명령.
//!
//! 탭: 모험지(Projects) / 원정(Campaigns) / 모험(Quests). escalation 대기 카운트.
//! 명령(campaign plan/run · quest resume/triage)은 **백그라운드 워커 스레드**에서 실행하고
//! (mpsc 채널로 결과 수신), 완료 시 대시보드를 자동 갱신한다. 메인 루프는 `event::poll`로
//! 논블로킹 — 장시간 작업 중에도 UI가 멈추지 않는다.

use std::io::{stdout, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, ExecutableCommand};
use luida_core::{
    migrate, open_db, open_ready, Campaign, Connection, InmailRepo, Project, Quest, CampaignRepo,
    ProjectRepo, QuestRepo,
};
use luida_planner::{plan_campaign, run_campaign};
use luida_runtimes::make_factory;
use luida_sidecar::{make_worktree, resume_quest, triage_escalation};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

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
}

impl Dashboard {
    pub fn load(conn: &Connection) -> Result<Self> {
        Ok(Self {
            projects: ProjectRepo::new(conn).list()?,
            campaigns: CampaignRepo::new(conn).list_active()?,
            quests: QuestRepo::new(conn).list_active()?,
            pending_user_mail: InmailRepo::new(conn).pending_for("@user")?.len(),
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
        };
        app.reset_selection();
        app
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
}

/// 명령을 백그라운드 워커로 띄운다 (Running 중이면 무시 — 동시 1개).
fn spawn_worker(app: &mut App, cmd: Command, label: String) {
    if matches!(app.mode, Mode::Running) {
        return;
    }
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
fn handle_key(app: &mut App, code: KeyCode) -> Result<bool> {
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
        Mode::Normal => {
            match code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
                KeyCode::Tab => app.switch_tab(),
                KeyCode::Down | KeyCode::Char('j') => app.next(),
                KeyCode::Up | KeyCode::Char('k') => app.prev(),
                KeyCode::Char('p') => {
                    app.mode = Mode::Input(InputKind::PlanPrompt);
                    app.input.clear();
                }
                KeyCode::Enter => {
                    let id = app.selected_campaign().map(|c| c.id);
                    if let Some(id) = id {
                        spawn_worker(app, Command::Run(id), format!("campaign run #{id}"));
                    }
                }
                KeyCode::Char('r') => {
                    let qid = app.selected_quest().map(|q| q.id);
                    if let Some(qid) = qid {
                        app.mode = Mode::Input(InputKind::ResumeAnswer { quest_id: qid });
                        app.input.clear();
                    }
                }
                KeyCode::Char('t') => {
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
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
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
    let _guard = TerminalGuard;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let result = run_loop(&mut terminal, &mut app);
    let _ = terminal.show_cursor();
    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        // 워커 완료 확인 (논블로킹).
        if let Some(rx) = &app.rx {
            match rx.try_recv() {
                Ok(msg) => {
                    app.status = Some(match msg {
                        WorkerMsg::Done(s) => format!("✅ {s}"),
                        WorkerMsg::Failed(e) => format!("⚠ 실패: {e}"),
                    });
                    app.mode = Mode::Normal;
                    app.running_label = None;
                    app.rx = None;
                    app.reload()?;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    app.status = Some("⚠ 워커가 비정상 종료했습니다".to_string());
                    app.mode = Mode::Normal;
                    app.running_label = None;
                    app.rx = None;
                }
            }
        }

        // 키 입력 폴링 (150ms 타임아웃 → 워커 진행 중에도 UI 갱신).
        if event::poll(Duration::from_millis(150))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if handle_key(app, key.code)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    // 헤더 — 탭 바 + 실행 상태
    let mut spans = vec![Span::styled("🍺 루이다  ", Style::default().fg(GOLD).bold())];
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
        spans.push(Span::styled(
            format!(" ⏳ {label} "),
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

    // 본문 — 탭별 목록
    let (items, empty_hint) = tab_items(app);
    if items.is_empty() {
        let empty = Paragraph::new(empty_hint)
            .style(Style::default().fg(DIM))
            .block(tab_block(app, 0));
        f.render_widget(empty, chunks[1]);
    } else {
        let count = items.len();
        let list = List::new(items)
            .block(tab_block(app, count))
            .highlight_style(Style::default().bg(Color::Rgb(0x1e, 0x2d, 0x44)).fg(GOLD))
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, chunks[1], &mut app.state);
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
        Mode::Input(_) => (" Enter 제출 · Esc 취소 ".to_string(), GOLD),
        Mode::Normal => match &app.status {
            Some(s) => (
                format!(" {s}  ·  q 종료 "),
                if s.starts_with('⚠') { RED } else { GREEN },
            ),
            None => (
                " Tab 탭 · j/k 이동 · p 계획 · Enter 실행 · r 재개 · t triage · q 종료 ".to_string(),
                DIM,
            ),
        },
    };
    let footer = Paragraph::new(Span::styled(footer_text, Style::default().fg(footer_color)));
    f.render_widget(footer, chunks[2]);

    // 입력 모달 (오버레이)
    if let Mode::Input(kind) = &app.mode {
        let label = match kind {
            InputKind::PlanPrompt => "원정 계획 — 프롬프트 입력",
            InputKind::ResumeAnswer { .. } => "모험 재개 — 답변 입력",
        };
        let area = centered_rect(70, 3, f.area());
        f.render_widget(Clear, area);
        let modal = Paragraph::new(Line::from(vec![
            Span::styled(app.input.clone(), Style::default().fg(GREEN)),
            Span::styled("█", Style::default().fg(GOLD)),
        ]))
        .block(
            Block::default()
                .title(format!(" {label} "))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(GOLD)),
        );
        f.render_widget(modal, area);
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
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("#{:<4}", c.id), Style::default().fg(DIM)),
                        Span::styled(format!("{:<12}", c.status), status_color(&c.status)),
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

    #[test]
    fn dashboard_load_counts() {
        let conn = seeded();
        let d = Dashboard::load(&conn).unwrap();
        assert_eq!(d.projects.len(), 1);
        assert_eq!(d.campaigns.len(), 1);
        assert_eq!(d.quests.len(), 1);
        assert_eq!(d.pending_user_mail, 1);
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
        assert!(!handle_key(&mut app, KeyCode::Char('p')).unwrap());
        assert!(matches!(app.mode, Mode::Input(InputKind::PlanPrompt)));
        handle_key(&mut app, KeyCode::Char('a')).unwrap();
        handle_key(&mut app, KeyCode::Char('b')).unwrap();
        assert_eq!(app.input, "ab");
        handle_key(&mut app, KeyCode::Backspace).unwrap();
        assert_eq!(app.input, "a");
        handle_key(&mut app, KeyCode::Esc).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.input, "");
    }

    #[test]
    fn key_r_on_quests_enters_resume_input() {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.switch_tab(); // Campaigns
        app.switch_tab(); // Quests
        handle_key(&mut app, KeyCode::Char('r')).unwrap();
        match &app.mode {
            Mode::Input(InputKind::ResumeAnswer { quest_id }) => assert!(*quest_id > 0),
            _ => panic!("resume 입력 모드가 아님"),
        }
    }

    #[test]
    fn key_q_quits() {
        let conn = seeded();
        let mut app = test_app(&conn);
        assert!(handle_key(&mut app, KeyCode::Char('q')).unwrap());
    }

    #[test]
    fn running_mode_ignores_keys() {
        let conn = seeded();
        let mut app = test_app(&conn);
        app.mode = Mode::Running;
        // Running 중엔 q도 종료 안 됨, 탭도 안 바뀜.
        assert!(!handle_key(&mut app, KeyCode::Char('q')).unwrap());
        assert!(!handle_key(&mut app, KeyCode::Tab).unwrap());
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
