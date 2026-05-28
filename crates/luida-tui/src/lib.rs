//! luida-tui — ratatui 기반 TUI (v2-P0 골격).
//! 현재: 등록된 모험지 목록 + 빈 상태. 등록 폼·원정·모니터는 후속 Phase.

use std::io::{stdout, Stdout};
use std::path::Path;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, ExecutableCommand};
use luida_core::{migrate, open_db, Project, ProjectRepo};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

const GOLD: Color = Color::Rgb(0xFC, 0xD3, 0x4D);
const DIM: Color = Color::Rgb(0x8A, 0xA0, 0xC0);
const GREEN: Color = Color::Rgb(0x4A, 0xDE, 0x80);

struct App {
    projects: Vec<Project>,
    state: ListState,
}

impl App {
    fn new(projects: Vec<Project>) -> Self {
        let mut state = ListState::default();
        if !projects.is_empty() {
            state.select(Some(0));
        }
        Self { projects, state }
    }

    fn next(&mut self) {
        if self.projects.is_empty() {
            return;
        }
        let i = self.state.selected().map_or(0, |i| {
            if i + 1 >= self.projects.len() {
                i
            } else {
                i + 1
            }
        });
        self.state.select(Some(i));
    }

    fn prev(&mut self) {
        if self.projects.is_empty() {
            return;
        }
        let i = self.state.selected().map_or(0, |i| i.saturating_sub(1));
        self.state.select(Some(i));
    }
}

/// 터미널 상태 복원을 RAII로 보장 (C1: 패닉·에러 경로에서도 unwind 시 Drop으로 복원).
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
    migrate(&mut conn)?; // db init 없이 ui 실행해도 동작
    let projects = ProjectRepo::new(&conn).list()?;
    let mut app = App::new(projects);

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    // guard가 drop될 때(정상 종료·에러 반환·패닉 unwind 모두) 터미널 복원
    let _guard = TerminalGuard;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let result = run_loop(&mut terminal, &mut app);
    let _ = terminal.show_cursor();
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
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
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    // 헤더
    let header = Paragraph::new(Line::from(vec![
        Span::styled("🍺 루이다의 주점", Style::default().fg(GOLD).bold()),
        Span::styled("  — 모험지 등록부", Style::default().fg(DIM)),
    ]))
    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(DIM)));
    f.render_widget(header, chunks[0]);

    // 모험지 목록 or 빈 상태
    if app.projects.is_empty() {
        let empty = Paragraph::new(
            "아직 등록된 모험지가 없습니다.\n\n`luida project add <name> --path <repo>` 로 등록하세요.",
        )
        .style(Style::default().fg(DIM))
        .block(
            Block::default()
                .title(" 모험지 ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM)),
        );
        f.render_widget(empty, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .projects
            .iter()
            .map(|p| {
                let desc = p.description.clone().unwrap_or_default();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<16}", p.name), Style::default().fg(GREEN).bold()),
                    Span::styled(format!("{:<10}", p.base_branch), Style::default().fg(GOLD)),
                    Span::styled(p.repo_path.clone(), Style::default().fg(DIM)),
                    Span::styled(
                        if desc.is_empty() { String::new() } else { format!("  — {desc}") },
                        Style::default().fg(DIM),
                    ),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .title(format!(" 모험지 ({}) ", app.projects.len()))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(GOLD)),
            )
            .highlight_style(Style::default().bg(Color::Rgb(0x1e, 0x2d, 0x44)).fg(GOLD))
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, chunks[1], &mut app.state);
    }

    // 푸터
    let footer = Paragraph::new(Span::styled(
        " j/k 이동 · q 종료 ",
        Style::default().fg(DIM),
    ));
    f.render_widget(footer, chunks[2]);
}
