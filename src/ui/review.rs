use crate::models::{EmQuadrant, WeekSummary};
use crate::services::{monday_of_week, ReviewService};
use crate::ui::text_editor::TextEditor;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReviewViewError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
    #[error("Database error: {0}")]
    Database(String),
}

pub type Result<T> = std::result::Result<T, ReviewViewError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    QuadrantTable,
    SplitAttention,
    Notes,
}

impl Section {
    fn next(self) -> Self {
        match self {
            Section::QuadrantTable => Section::SplitAttention,
            Section::SplitAttention => Section::Notes,
            Section::Notes => Section::QuadrantTable,
        }
    }
}

#[derive(Debug, Clone)]
enum EditState {
    Target { buffer: String, row: usize },
    Notes(TextEditor),
}

pub struct ReviewView {
    service: ReviewService,
    summary: WeekSummary,
    week_start: chrono::NaiveDate,
    selected_row: usize,
    section: Section,
    editing: Option<EditState>,
    has_active_session: bool,
}

impl ReviewView {
    pub fn new(service: ReviewService, week_start: chrono::NaiveDate) -> Result<Self> {
        let summary = service
            .get_week_summary(week_start)
            .map_err(|e| ReviewViewError::Database(e.to_string()))?;

        let has_active_session = service.has_active_session(week_start);

        Ok(Self {
            service,
            summary,
            week_start,
            selected_row: 0,
            section: Section::QuadrantTable,
            editing: None,
            has_active_session,
        })
    }

    pub fn run(mut self) -> Result<()> {
        enable_raw_mode().map_err(|e| ReviewViewError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| ReviewViewError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| ReviewViewError::Terminal(e.to_string()))?;

        self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| ReviewViewError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| ReviewViewError::Terminal(e.to_string()))?;

        Ok(())
    }

    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
        loop {
            if let Err(e) = terminal.draw(|f| self.render(f)) {
                eprintln!("Render error: {}", e);
                break;
            }

            if let Ok(Event::Key(key)) = event::read() {
                if self.editing.is_some() {
                    if self.handle_edit_key(key) {
                        break;
                    }
                } else {
                    match (key.modifiers, key.code) {
                        (_, KeyCode::Char('q')) | (_, KeyCode::Esc) => break,
                        (_, KeyCode::Tab) => self.section = self.section.next(),
                        (_, KeyCode::Char('j')) | (_, KeyCode::Down) => self.move_selection(1),
                        (_, KeyCode::Char('k')) | (_, KeyCode::Up) => self.move_selection(-1),
                        (_, KeyCode::Char('h')) => self.change_week(-1),
                        (_, KeyCode::Char('l')) => self.change_week(1),
                        (_, KeyCode::Char('t')) => {
                            self.section = Section::QuadrantTable;
                            self.selected_row = 0;
                            self.start_target_edit();
                        }
                        (_, KeyCode::Char('n')) => {
                            self.section = Section::Notes;
                            self.start_notes_edit();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let max = match self.section {
            Section::QuadrantTable => 3,
            Section::SplitAttention => {
                self.summary.top_split_tasks.len().saturating_sub(1) as i32
            }
            Section::Notes => 0,
        };
        let new = (self.selected_row as i32 + delta).clamp(0, max);
        self.selected_row = new as usize;
    }

    fn change_week(&mut self, delta: i32) {
        self.week_start += chrono::Duration::weeks(delta as i64);
        self.selected_row = 0;
        self.reload();
    }

    fn reload(&mut self) {
        if let Ok(s) = self.service.get_week_summary(self.week_start) {
            self.summary = s;
        }
        self.has_active_session = self.service.has_active_session(self.week_start);
    }

    fn start_target_edit(&mut self) {
        self.editing = Some(EditState::Target {
            buffer: String::new(),
            row: self.selected_row,
        });
    }

    fn start_notes_edit(&mut self) {
        let existing = self
            .service
            .get_review(self.week_start)
            .ok()
            .flatten()
            .map(|r| r.notes)
            .unwrap_or_default();
        self.editing = Some(EditState::Notes(TextEditor::new(&existing)));
    }

    fn handle_edit_key(&mut self, key: event::KeyEvent) -> bool {
        let is_ctrl_s = key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('s');
        let is_esc = key.code == KeyCode::Esc;

        if let Some(EditState::Notes(editor)) = &mut self.editing {
            if is_ctrl_s {
                let _ = self.service.save_review(self.week_start, editor.text());
                self.editing = None;
            } else if is_esc {
                self.editing = None;
            } else {
                editor.handle_key(key);
            }
            return false;
        }

        if let Some(EditState::Target { buffer, row }) = &mut self.editing {
            match key.code {
                KeyCode::Enter => {
                    let row = *row;
                    if let Ok(minutes) = buffer.parse::<i64>() {
                        let quadrant = EmQuadrant::all()[row];
                        let _ = self.service.set_target(self.week_start, quadrant, minutes);
                        self.reload();
                    }
                    self.editing = None;
                }
                KeyCode::Esc => self.editing = None,
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    buffer.push(c);
                }
                _ => {}
            }
        }

        false
    }

    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(f.size());

        self.render_header(f, chunks[0]);
        self.render_quadrant_table(f, chunks[1]);
        self.render_split_attention(f, chunks[2]);
        self.render_notes(f, chunks[3]);
        self.render_footer(f, chunks[4]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let week_end = self.week_start + chrono::Duration::days(6);
        let active_indicator = if self.has_active_session { " *" } else { "" };
        let title = format!(
            " Weekly Review: {} to {}{} ",
            self.week_start.format("%Y-%m-%d"),
            week_end.format("%Y-%m-%d"),
            active_indicator
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

        f.render_widget(block, area);
    }

    fn render_quadrant_table(&self, f: &mut Frame, area: Rect) {
        let selected = self.section == Section::QuadrantTable;
        let title = if selected {
            " Quadrant Budget vs Actual ▶ "
        } else {
            " Quadrant Budget vs Actual "
        };

        let mut lines: Vec<Line> = Vec::new();

        let target_color = if self.summary.has_targets {
            Color::White
        } else {
            Color::DarkGray
        };

        lines.push(Line::from(Span::styled(
            format!("{:<14} {:>7} {:>7} {:>7}", "Quadrant", "Target", "Actual", "Delta"),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

        for (i, q) in EmQuadrant::all().iter().enumerate() {
            let target_pct = self.summary.target_pct(*q);
            let actual_pct = self.summary.actual_pct(*q);
            let delta = self.summary.delta_pct(*q);

            let delta_color = if delta.abs() < 1.0 {
                Color::Green
            } else if delta > 0.0 {
                Color::Yellow
            } else {
                Color::Red
            };

            let is_selected = selected && i == self.selected_row;
            let row_bg = if is_selected {
                Color::DarkGray
            } else {
                Color::Reset
            };

            let mut row_spans = vec![
                Span::styled(
                    format!(" {:<12}", q.tag()),
                    Style::default().fg(Color::Green).bg(row_bg),
                ),
                Span::styled(
                    format!(" {:>6.0}%", target_pct),
                    Style::default().fg(target_color).bg(row_bg),
                ),
                Span::styled(
                    format!(" {:>6.0}%", actual_pct),
                    Style::default().bg(row_bg),
                ),
                Span::styled(
                    format!(" {:>+5.0}%", delta),
                    Style::default().fg(delta_color).bg(row_bg),
                ),
            ];
            if is_selected {
                row_spans.push(Span::styled(" ◀", Style::default().fg(Color::Cyan)));
            }

            lines.push(Line::from(row_spans));
        }

        if !self.summary.has_targets {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  (suggested targets — press [t] to set your own)",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(Color::Cyan));

        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_split_attention(&self, f: &mut Frame, area: Rect) {
        let selected = self.section == Section::SplitAttention;
        let title = if selected {
            " Split Attention ▶ "
        } else {
            " Split Attention "
        };

        let total_str = format_duration(self.summary.split_attention_seconds);
        let mut lines = vec![
            Line::from(format!("  Multitasked: {} this week", total_str)),
            Line::from(""),
        ];

        if self.summary.top_split_tasks.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No split attention detected",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (i, task) in self.summary.top_split_tasks.iter().enumerate() {
                let style = if selected && i == self.selected_row {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };
                let dur = format_duration(task.overlap_seconds);
                let row = format!(
                    "  {}. {:<20} {:>8}  ({} sessions)",
                    i + 1,
                    task.name,
                    dur,
                    task.session_count
                );
                lines.push(Line::from(Span::styled(row, style)));
            }
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(Color::Cyan));

        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_notes(&self, f: &mut Frame, area: Rect) {
        let selected = self.section == Section::Notes;
        let title = if selected {
            " Review Notes ▶ "
        } else {
            " Review Notes "
        };

        let (lines, editor_info) = if let Some(EditState::Notes(editor)) = &self.editing {
            let mut l: Vec<Line> = vec![
                Line::from(Span::styled(
                    "  [Editing - Ctrl+S to save, Esc to cancel]",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from(""),
            ];
            for line in editor.text().split('\n') {
                l.push(Line::from(format!("  {}", line)));
            }
            (l, Some((editor.cursor_line(), editor.cursor_col())))
        } else {
            let notes = self
                .service
                .get_review(self.week_start)
                .ok()
                .flatten()
                .map(|r| r.notes)
                .unwrap_or_default();

            if notes.is_empty() {
                (
                    vec![Line::from(Span::styled(
                        "  [No notes for this week - press 'n' to add]",
                        Style::default().fg(Color::DarkGray),
                    ))],
                    None,
                )
            } else {
                (
                    notes
                        .lines()
                        .map(|l| Line::from(format!("  {}", l)))
                        .collect(),
                    None,
                )
            }
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(Color::Cyan));

        f.render_widget(Paragraph::new(lines).block(block), area);

        if let Some((line, col)) = editor_info {
            let cursor_y = area.y + 1 + 2 + line as u16;
            let cursor_x = area.x + 1 + 2 + col as u16;
            f.set_cursor(cursor_x, cursor_y);
        }
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let line = if let Some(edit) = &self.editing {
            match edit {
                EditState::Target { buffer, .. } => Line::from(vec![
                    Span::styled(
                        format!(" Target minutes: {}", buffer),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(" [Enter] Save [Esc] Cancel"),
                ]),
                EditState::Notes(editor) => Line::from(vec![
                    Span::styled(
                        format!(
                            " {}",
                            if editor.is_empty() {
                                "(empty)".to_string()
                            } else {
                                editor.text().replace('\n', " ")
                            }
                        ),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(" [Ctrl+S] Save [Esc] Cancel"),
                ]),
            }
        } else {
            Line::from(vec![
                Span::styled("[Tab]", Style::default().fg(Color::Yellow)),
                Span::raw(" Section  "),
                Span::styled("[j/k]", Style::default().fg(Color::Yellow)),
                Span::raw(" Navigate  "),
                Span::styled("[t]", Style::default().fg(Color::Yellow)),
                Span::raw(" Set Target  "),
                Span::styled("[n]", Style::default().fg(Color::Yellow)),
                Span::raw(" Notes  "),
                Span::styled("[h/l]", Style::default().fg(Color::Yellow)),
                Span::raw(" Week  "),
                Span::styled("[q]", Style::default().fg(Color::Yellow)),
                Span::raw(" Quit"),
            ])
        };

        let paragraph = Paragraph::new(line).block(Block::default().borders(Borders::ALL));

        f.render_widget(paragraph, area);
    }
}

fn format_duration(seconds: i64) -> String {
    if seconds == 0 {
        return "0m".to_string();
    }
    let hours = seconds / 3600;
    let mins = (seconds % 3600) / 60;
    match (hours, mins) {
        (0, m) => format!("{}m", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h {}m", h, m),
    }
}

pub fn run_review(service: ReviewService, week_start: Option<chrono::NaiveDate>) -> Result<()> {
    let ws = week_start.unwrap_or_else(|| monday_of_week(chrono::Local::now().date_naive()));
    ReviewView::new(service, ws)?.run()
}
