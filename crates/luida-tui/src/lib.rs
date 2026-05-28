//! luida-tui — ratatui 기반 TUI 대시보드.
//! 탭: 모험지(Projects) / 원정(Campaigns) / 모험(Quests). escalation 대기 카운트.

use std::io::{stdout, Stdout};
use std::path::Path;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, ExecutableCommand};
use luida_core::{
    migrate, open_db, Campaign, Connection, InmailRepo, Project, Quest, CampaignRepo, ProjectRepo,
    QuestRepo,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

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

struct App {
    dash: Dashboard,
    tab: Tab,
    state: ListState,
}

impl App {
    fn new(dash: Dashboard) -> Self {
        let mut app = Self {
            dash,
            tab: Tab::Projects,
            state: ListState::default(),
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
    let mut app = App::new(dash);

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
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Tab => app.switch_tab(),
                KeyCode::Down | KeyCode::Char('j') => app.next(),
                KeyCode::Up | KeyCode::Char('k') => app.prev(),
                _ => {}
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

    // 헤더 — 탭 바
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

    let footer = Paragraph::new(Span::styled(
        " Tab 탭전환 · j/k 이동 · q 종료 ",
        Style::default().fg(DIM),
    ));
    f.render_widget(footer, chunks[2]);
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
            "진행 중인 원정이 없습니다. `luida campaign plan \"...\"` 로 시작하세요.".to_string(),
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
        let mut app = App::new(Dashboard::load(&conn).unwrap());
        assert_eq!(app.tab, Tab::Projects);
        assert_eq!(app.state.selected(), Some(0));
        app.switch_tab();
        assert_eq!(app.tab, Tab::Campaigns);
        app.switch_tab();
        assert_eq!(app.tab, Tab::Quests);
        app.switch_tab();
        assert_eq!(app.tab, Tab::Projects);
        // 네비게이션 경계
        app.next();
        assert_eq!(app.state.selected(), Some(0)); // 1개라 그대로
        app.prev();
        assert_eq!(app.state.selected(), Some(0));
    }

    #[test]
    fn tab_items_render_rows() {
        let conn = seeded();
        let mut app = App::new(Dashboard::load(&conn).unwrap());
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
        let app = App::new(Dashboard::load(&conn).unwrap());
        assert_eq!(app.current_len(), 0);
        assert_eq!(app.state.selected(), None);
    }
}
