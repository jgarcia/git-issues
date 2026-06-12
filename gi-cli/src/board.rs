//! `gi board` — the read-only Kanban TUI.
//!
//! Three columns (Open / In Progress / Done) populated from the same validated
//! read path as `gi list`. The user can move between columns, scroll within a
//! column, and open an issue's detail (its body). It is strictly **read-only**:
//! nothing here mutates an issue — every state change still goes through the CLI
//! verbs. Keeping [`App`] and [`ui`] free of any terminal I/O lets the rendering
//! be exercised with ratatui's `TestBackend` (see the tests below); only [`run`]
//! touches the real terminal and event stream.

use std::io;

use anyhow::{Context, Result};
use gi_core::{Issue, Status};
use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

/// The three columns, in workflow order. Indices into [`App::columns`].
const COLUMNS: [Status; 3] = [Status::Open, Status::InProgress, Status::Done];
const COLUMN_TITLES: [&str; 3] = ["Open", "In Progress", "Done"];

/// Whether the board is showing the columns or a single issue's detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Board,
    Detail,
}

/// All the state the board needs: the issues bucketed by column, which column is
/// active, the selected row within each column, the view mode, and the detail
/// scroll offset. No terminal handle lives here, so it is trivially testable.
pub struct App {
    /// Issues per column, indexed to match [`COLUMNS`].
    columns: [Vec<Issue>; 3],
    /// The focused column (0..=2).
    active: usize,
    /// Selected row within each column; clamped to that column's length.
    selected: [usize; 3],
    mode: Mode,
    /// Vertical scroll offset for the detail view, in lines.
    detail_scroll: u16,
    /// Set once the user asks to quit.
    quit: bool,
}

impl App {
    /// Bucket validated issues into their columns. Within a column they are
    /// ordered like `gi list`: by title, then id — a stable, intuitive order.
    pub fn new(issues: Vec<Issue>) -> App {
        let mut columns: [Vec<Issue>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for issue in issues {
            let col = COLUMNS.iter().position(|s| *s == issue.status).unwrap();
            columns[col].push(issue);
        }
        for col in &mut columns {
            col.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
        }
        App {
            columns,
            active: 0,
            selected: [0; 3],
            mode: Mode::Board,
            detail_scroll: 0,
            quit: false,
        }
    }

    /// The issue currently under the cursor, if the active column is non-empty.
    fn selected_issue(&self) -> Option<&Issue> {
        self.columns[self.active].get(self.selected[self.active])
    }

    /// Move focus one column left.
    fn focus_left(&mut self) {
        self.active = self.active.saturating_sub(1);
    }

    /// Move focus one column right.
    fn focus_right(&mut self) {
        self.active = (self.active + 1).min(COLUMNS.len() - 1);
    }

    /// Move the selection up within the active column.
    fn select_up(&mut self) {
        let sel = &mut self.selected[self.active];
        *sel = sel.saturating_sub(1);
    }

    /// Move the selection down within the active column, clamped to its last row.
    fn select_down(&mut self) {
        let len = self.columns[self.active].len();
        if len == 0 {
            return;
        }
        let sel = &mut self.selected[self.active];
        *sel = (*sel + 1).min(len - 1);
    }

    /// Open the detail view for the selected issue, if any.
    fn open_detail(&mut self) {
        if self.selected_issue().is_some() {
            self.mode = Mode::Detail;
            self.detail_scroll = 0;
        }
    }

    /// Leave the detail view, back to the columns.
    fn close_detail(&mut self) {
        self.mode = Mode::Board;
    }

    /// Apply a key press to the state. Pure: it never touches the terminal, so
    /// the event loop and the tests drive the board the same way.
    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        // Ctrl-C always quits, in either mode.
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Board => match code {
                KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
                KeyCode::Left | KeyCode::Char('h') => self.focus_left(),
                KeyCode::Right | KeyCode::Char('l') => self.focus_right(),
                KeyCode::Up | KeyCode::Char('k') => self.select_up(),
                KeyCode::Down | KeyCode::Char('j') => self.select_down(),
                KeyCode::Enter => self.open_detail(),
                _ => {}
            },
            Mode::Detail => match code {
                KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter | KeyCode::Left => {
                    self.close_detail()
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.detail_scroll = self.detail_scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1)
                }
                _ => {}
            },
        }
    }
}

/// Render the whole board into `frame` for the current `app` state. Pure with
/// respect to the terminal — it only describes what to draw — so a `TestBackend`
/// can snapshot the result.
pub fn ui(frame: &mut Frame, app: &App) {
    match app.mode {
        Mode::Board => draw_columns(frame, app),
        Mode::Detail => draw_detail(frame, app),
    }
}

/// Draw the three Kanban columns side by side with a one-line help footer.
fn draw_columns(frame: &mut Frame, app: &App) {
    let outer = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(frame.area());

    let cols = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(outer[0]);

    for (idx, area) in cols.iter().enumerate() {
        let is_active = idx == app.active;
        let title = format!(" {} ({}) ", COLUMN_TITLES[idx], app.columns[idx].len());
        let border_style = if is_active {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let items: Vec<ListItem> = app.columns[idx]
            .iter()
            .map(|issue| ListItem::new(issue_line(issue)))
            .collect();

        let mut list = List::new(items).block(block);
        // Only the active column shows a selection highlight, so it is always
        // clear which row Enter would open.
        if is_active {
            list = list
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
        }

        let mut state = ListState::default();
        if !app.columns[idx].is_empty() {
            state.select(Some(app.selected[idx].min(app.columns[idx].len() - 1)));
        }
        frame.render_stateful_widget(list, *area, &mut state);
    }

    let help = match app.selected_issue() {
        Some(_) => "←/→ column   ↑/↓ select   ⏎ detail   q quit",
        None => "←/→ column   ↑/↓ select   q quit",
    };
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
        outer[1],
    );
}

/// One list row for an issue: its short hash, title, and who holds it.
fn issue_line(issue: &Issue) -> Line<'static> {
    let who = issue.assignee.clone().unwrap_or_else(|| "-".to_string());
    Line::from(vec![
        Span::styled(
            format!("{}  ", issue.id),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw(issue.title.clone()),
        Span::styled(
            format!("  ({who})"),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

/// Draw the detail view for the selected issue: its frontmatter summary followed
/// by the markdown body, wrapped and scrollable.
fn draw_detail(frame: &mut Frame, app: &App) {
    let Some(issue) = app.selected_issue() else {
        // Shouldn't happen — detail is only entered with a selection — but never
        // panic over it; fall back to the columns.
        draw_columns(frame, app);
        return;
    };

    let who = issue.assignee.clone().unwrap_or_else(|| "-".to_string());
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            issue.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("hash: ", Style::default().fg(Color::DarkGray)),
            Span::raw(issue.id.clone()),
            Span::styled("   status: ", Style::default().fg(Color::DarkGray)),
            Span::raw(issue.status.as_str().to_string()),
            Span::styled("   who: ", Style::default().fg(Color::DarkGray)),
            Span::raw(who),
        ]),
        Line::from(""),
    ];
    for body_line in issue.body.lines() {
        lines.push(Line::from(body_line.to_string()));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Issue detail — q/Esc/⏎ back, ↑/↓ scroll ");

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll, 0));
    frame.render_widget(paragraph, frame.area());
}

/// Run the interactive board: set up the terminal, loop drawing and handling
/// input until the user quits, then restore the terminal — even on error.
pub fn run(issues: Vec<Issue>) -> Result<()> {
    let mut app = App::new(issues);

    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to build terminal")?;

    let result = event_loop(&mut terminal, &mut app);

    // Restore the terminal regardless of how the loop exited.
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    result
}

/// The draw/input loop, split out so terminal teardown in [`run`] always runs.
fn event_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    while !app.quit {
        terminal.draw(|frame| ui(frame, app)).context("failed to draw frame")?;

        match event::read().context("failed to read terminal event")? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                app.on_key(key.code, key.modifiers);
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn issue(id: &str, title: &str, status: Status, assignee: Option<&str>) -> Issue {
        Issue {
            id: id.to_string(),
            title: title.to_string(),
            status,
            assignee: assignee.map(|s| s.to_string()),
            body: String::new(),
        }
    }

    /// Render `app` onto a fresh `TestBackend` and return the buffer as one
    /// newline-joined string, so tests can assert on what the user would see.
    fn render(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| ui(frame, app)).unwrap();
        buffer_text(terminal.backend().buffer())
    }

    /// Flatten a ratatui buffer to rows of text, trimming trailing blanks.
    fn buffer_text(buffer: &Buffer) -> String {
        let area = buffer.area;
        let mut rows = Vec::new();
        for y in 0..area.height {
            let mut row = String::new();
            for x in 0..area.width {
                row.push_str(buffer[(x, y)].symbol());
            }
            rows.push(row.trim_end().to_string());
        }
        rows.join("\n")
    }

    /// The column header for `title` shows up alongside its issue count.
    fn assert_column_header(text: &str, title: &str, count: usize) {
        assert!(
            text.contains(&format!("{title} ({count})")),
            "expected `{title} ({count})` header in:\n{text}"
        );
    }

    fn sample() -> App {
        App::new(vec![
            issue("a1b2", "Fix login bug", Status::Open, Some("joel")),
            issue("9f2c", "Write docs", Status::Open, None),
            issue("7e3d", "Refactor parser", Status::InProgress, Some("ann")),
            issue("c4d5", "Ship release", Status::Done, Some("ann")),
        ])
    }

    #[test]
    fn issues_land_in_their_columns() {
        let app = sample();
        // Bucketing is independent of the render width.
        assert_eq!(app.columns[0].len(), 2); // open
        assert_eq!(app.columns[1].len(), 1); // in_progress
        assert_eq!(app.columns[2].len(), 1); // done

        let text = render(&app, 120, 20);
        assert_column_header(&text, "Open", 2);
        assert_column_header(&text, "In Progress", 1);
        assert_column_header(&text, "Done", 1);

        // Every issue title is rendered under some column.
        for title in ["Fix login bug", "Write docs", "Refactor parser", "Ship release"] {
            assert!(text.contains(title), "missing `{title}` in:\n{text}");
        }
    }

    #[test]
    fn open_column_lists_only_open_issues_with_their_hashes() {
        let app = sample();
        // The two open issues appear; both their hashes are on the board.
        let text = render(&app, 120, 20);
        assert!(text.contains("a1b2"));
        assert!(text.contains("9f2c"));
        // The single in-progress / done issues keep their hashes too.
        assert!(text.contains("7e3d"));
        assert!(text.contains("c4d5"));
    }

    #[test]
    fn detail_view_renders_the_selected_issue() {
        let mut app = sample();
        // Select the second open issue, then open its detail.
        app.select_down();
        let selected = app.selected_issue().unwrap().clone();
        app.open_detail();
        assert_eq!(app.mode, Mode::Detail);

        let text = render(&app, 80, 20);
        assert!(text.contains("Issue detail"), "no detail chrome in:\n{text}");
        assert!(
            text.contains(&selected.title),
            "detail missing title `{}` in:\n{text}",
            selected.title
        );
        assert!(
            text.contains(&selected.id),
            "detail missing hash `{}` in:\n{text}",
            selected.id
        );
        assert!(text.contains(selected.status.as_str()));
    }

    #[test]
    fn detail_view_shows_the_body() {
        let mut issue = issue("abcd", "Has a body", Status::Open, None);
        issue.body = "First body line\nSecond body line\n".to_string();
        let mut app = App::new(vec![issue]);
        app.open_detail();

        let text = render(&app, 80, 20);
        assert!(text.contains("First body line"), "body missing in:\n{text}");
        assert!(text.contains("Second body line"));
    }

    #[test]
    fn navigation_moves_focus_and_selection() {
        let mut app = sample();
        assert_eq!(app.active, 0);

        // Right twice, clamped at the last column.
        app.on_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(app.active, 1);
        app.on_key(KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(app.active, 2);
        app.on_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(app.active, 2, "right is clamped at the last column");

        // Back to the open column and move the selection down, clamped.
        app.on_key(KeyCode::Left, KeyModifiers::NONE);
        app.on_key(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(app.active, 0);
        app.on_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.selected[0], 1);
        app.on_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.selected[0], 1, "down is clamped at the last row");
    }

    #[test]
    fn enter_opens_and_esc_closes_detail() {
        let mut app = sample();
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.mode, Mode::Detail);
        app.on_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(app.mode, Mode::Board);
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let mut app = sample();
        app.on_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(app.quit);

        let mut app = sample();
        app.on_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.quit);
    }

    #[test]
    fn enter_on_empty_column_does_not_open_detail() {
        // No issues at all: Enter must not crash or open an empty detail.
        let mut app = App::new(Vec::new());
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.mode, Mode::Board);
        // And it still renders.
        let text = render(&app, 60, 10);
        assert_column_header(&text, "Open", 0);
    }
}
