use crate::clipboard::{open_link, OpenLinkResult};
use crate::models::Session;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
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
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionEditError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
    #[error("Invalid time format: {0}")]
    InvalidTimeFormat(String),
    #[error("End time must be after start time")]
    EndBeforeStart,
}

pub type Result<T> = std::result::Result<T, SessionEditError>;

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEditResult {
    Saved {
        start: DateTime<Utc>,
        end: Option<DateTime<Utc>>,
        notes: Option<String>,
        link: Option<String>,
        is_background: bool,
    },
    Deleted(i64),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct EditableSession {
    pub id: i64,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub notes: String,
    pub link: String,
    pub is_background: bool,
}

impl From<Session> for EditableSession {
    fn from(session: Session) -> Self {
        Self {
            id: session.id,
            start: session.start_time,
            end: session.end_time,
            notes: session.notes.clone().unwrap_or_default(),
            link: session.link.clone().unwrap_or_default(),
            is_background: session.is_background,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Field {
    StartTime,
    EndTime,
    Background,
    Notes,
    Link,
}

const FIELDS: [Field; 5] = [
    Field::StartTime,
    Field::EndTime,
    Field::Background,
    Field::Notes,
    Field::Link,
];

pub struct SessionEditForm {
    session: EditableSession,
    original: EditableSession,
    task_name: String,
    current_field: Field,
    editing_text: bool,
    text_buffer: String,
    cursor_pos: usize,
    error_message: Option<String>,
    pending_cancel: bool,
    pending_delete: bool,
}

impl SessionEditForm {
    pub fn new(session: Session, task_name: String) -> Self {
        let editable: EditableSession = session.into();
        let original = editable.clone();
        Self {
            session: editable,
            original,
            task_name,
            current_field: Field::StartTime,
            editing_text: false,
            text_buffer: String::new(),
            cursor_pos: 0,
            error_message: None,
            pending_cancel: false,
            pending_delete: false,
        }
    }

    fn has_changes(&self) -> bool {
        self.session.start != self.original.start
            || self.session.end != self.original.end
            || self.session.notes != self.original.notes
            || self.session.link != self.original.link
            || self.session.is_background != self.original.is_background
    }

    pub fn run(mut self) -> Result<SessionEditResult> {
        enable_raw_mode().map_err(|e| SessionEditError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| SessionEditError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| SessionEditError::Terminal(e.to_string()))?;

        let result = self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| SessionEditError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| SessionEditError::Terminal(e.to_string()))?;

        result
    }

    fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<SessionEditResult> {
        loop {
            terminal
                .draw(|f| self.render(f))
                .map_err(|e| SessionEditError::Terminal(e.to_string()))?;

            if let Event::Key(key) =
                event::read().map_err(|e| SessionEditError::Terminal(e.to_string()))?
            {
                if self.pending_cancel {
                    match (key.modifiers, key.code) {
                        (_, KeyCode::Char('y') | KeyCode::Char('Y')) => {
                            return Ok(SessionEditResult::Cancelled);
                        }
                        (_, KeyCode::Char('n') | KeyCode::Char('N')) | (_, KeyCode::Esc) => {
                            self.pending_cancel = false;
                        }
                        _ => {}
                    }
                } else if self.pending_delete {
                    match (key.modifiers, key.code) {
                        (_, KeyCode::Char('y') | KeyCode::Char('Y')) => {
                            return Ok(SessionEditResult::Deleted(self.session.id));
                        }
                        (_, KeyCode::Char('n') | KeyCode::Char('N')) | (_, KeyCode::Esc) => {
                            self.pending_delete = false;
                        }
                        _ => {}
                    }
                } else if self.editing_text {
                    match (key.modifiers, key.code) {
                        (_, KeyCode::Esc) => {
                            self.editing_text = false;
                            self.error_message = None;
                        }
                        (_, KeyCode::Enter) => {
                            if self.apply_text_edit() {
                                self.editing_text = false;
                                self.error_message = None;
                            }
                        }
                        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                            self.delete_prev_word();
                        }
                        (_, KeyCode::Char(c)) => {
                            self.text_buffer.insert(self.cursor_pos, c);
                            self.cursor_pos += c.len_utf8();
                        }
                        (_, KeyCode::Backspace) => {
                            if self.cursor_pos > 0 {
                                let prev_char_len = self.text_buffer[..self.cursor_pos]
                                    .chars()
                                    .last()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(0);
                                self.cursor_pos -= prev_char_len;
                                self.text_buffer.remove(self.cursor_pos);
                            }
                        }
                        (_, KeyCode::Left) => {
                            if self.cursor_pos > 0 {
                                let prev_char_len = self.text_buffer[..self.cursor_pos]
                                    .chars()
                                    .last()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(0);
                                self.cursor_pos -= prev_char_len;
                            }
                        }
                        (_, KeyCode::Right) => {
                            if self.cursor_pos < self.text_buffer.len() {
                                let next_char_len = self.text_buffer[self.cursor_pos..]
                                    .chars()
                                    .next()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(0);
                                self.cursor_pos += next_char_len;
                            }
                        }
                        _ => {}
                    }
                } else {
                    match (key.modifiers, key.code) {
                        (_, KeyCode::Esc) | (_, KeyCode::Char('q') | KeyCode::Char('Q')) => {
                            if self.has_changes() {
                                self.pending_cancel = true;
                            } else {
                                return Ok(SessionEditResult::Cancelled);
                            }
                        }
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                            return Ok(SessionEditResult::Cancelled);
                        }
                        (_, KeyCode::Char('d')) => {
                            self.pending_delete = true;
                        }
                        (_, KeyCode::Char('j')) => {
                            self.move_field(1);
                        }
                        (_, KeyCode::Char('k')) => {
                            self.move_field(-1);
                        }
                        (_, KeyCode::Up) => {
                            self.move_field(-1);
                        }
                        (_, KeyCode::Down) => {
                            self.move_field(1);
                        }
                        (_, KeyCode::Enter) | (_, KeyCode::Char(' ')) | (_, KeyCode::Right) => {
                            self.edit_current_field();
                        }
                        (_, KeyCode::Char('s')) => {
                            if let Some(result) = self.validate_and_save() {
                                return Ok(result);
                            }
                        }
                        (_, KeyCode::Char('o')) => {
                            self.open_current_link();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn move_field(&mut self, delta: i32) {
        let current_idx = FIELDS
            .iter()
            .position(|f| *f == self.current_field)
            .unwrap_or(0);
        let new_idx = ((current_idx as i32 + delta).clamp(0, (FIELDS.len() - 1) as i32)) as usize;
        self.current_field = FIELDS[new_idx];
    }

    fn edit_current_field(&mut self) {
        match self.current_field {
            Field::StartTime => {
                let local = Local.from_utc_datetime(&self.session.start.naive_utc());
                self.text_buffer = local.format("%Y-%m-%d %H:%M").to_string();
                self.cursor_pos = self.text_buffer.len();
                self.editing_text = true;
            }
            Field::EndTime => {
                self.text_buffer = match self.session.end {
                    Some(end) => {
                        let local = Local.from_utc_datetime(&end.naive_utc());
                        local.format("%Y-%m-%d %H:%M").to_string()
                    }
                    None => String::new(),
                };
                self.cursor_pos = self.text_buffer.len();
                self.editing_text = true;
            }
            Field::Background => {
                self.session.is_background = !self.session.is_background;
            }
            Field::Notes => {
                self.text_buffer = self.session.notes.clone();
                self.cursor_pos = self.text_buffer.len();
                self.editing_text = true;
            }
            Field::Link => {
                self.text_buffer = self.session.link.clone();
                self.cursor_pos = self.text_buffer.len();
                self.editing_text = true;
            }
        }
    }

    fn apply_text_edit(&mut self) -> bool {
        match self.current_field {
            Field::StartTime => match parse_datetime_input(&self.text_buffer) {
                Ok(dt) => {
                    self.session.start = dt;
                    true
                }
                Err(e) => {
                    self.error_message = Some(format!("Invalid start time: {}", e));
                    false
                }
            },
            Field::EndTime => {
                if self.text_buffer.trim().is_empty() {
                    self.session.end = None;
                    true
                } else {
                    match parse_datetime_input(&self.text_buffer) {
                        Ok(dt) => {
                            if dt <= self.session.start {
                                self.error_message =
                                    Some("End time must be after start time".to_string());
                                false
                            } else {
                                self.session.end = Some(dt);
                                true
                            }
                        }
                        Err(e) => {
                            self.error_message = Some(format!("Invalid end time: {}", e));
                            false
                        }
                    }
                }
            }
            Field::Background => true,
            Field::Notes => {
                self.session.notes = self.text_buffer.clone();
                true
            }
            Field::Link => {
                self.session.link = self.text_buffer.clone();
                true
            }
        }
    }

    fn delete_prev_word(&mut self) {
        let before_cursor = self.text_buffer[..self.cursor_pos].trim_end_matches(' ');
        if let Some(pos) = before_cursor.rfind(' ') {
            let delete_len = self.cursor_pos - (pos + 1);
            self.text_buffer.drain((pos + 1)..self.cursor_pos);
            self.cursor_pos -= delete_len;
        } else {
            self.text_buffer.drain(0..self.cursor_pos);
            self.cursor_pos = 0;
        }
    }

    fn validate_and_save(&mut self) -> Option<SessionEditResult> {
        if let Some(end) = self.session.end {
            if end <= self.session.start {
                self.error_message = Some("End time must be after start time".to_string());
                return None;
            }
        }
        Some(SessionEditResult::Saved {
            start: self.session.start,
            end: self.session.end,
            notes: if self.session.notes.is_empty() {
                None
            } else {
                Some(self.session.notes.clone())
            },
            link: if self.session.link.is_empty() {
                None
            } else {
                Some(self.session.link.clone())
            },
            is_background: self.session.is_background,
        })
    }

    fn open_current_link(&mut self) {
        if self.session.link.is_empty() {
            self.error_message = Some("No link to open".to_string());
            return;
        }

        match open_link(&self.session.link) {
            Ok(OpenLinkResult::OpenedUrl) => {
                self.error_message = Some("Opened in browser".to_string());
            }
            Ok(OpenLinkResult::CopiedPath(path)) => {
                self.error_message = Some(format!("Copied: {}", path));
            }
            Ok(OpenLinkResult::CopiedRaw(text)) => {
                self.error_message = Some(format!("Copied: {}", text));
            }
            Err(e) => {
                self.error_message = Some(e);
            }
        }
    }

    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(if self.error_message.is_some() { 10 } else { 8 }),
                Constraint::Length(3),
            ])
            .split(f.size());

        self.render_title(f, chunks[0]);
        self.render_fields(f, chunks[1]);
        self.render_help(f, chunks[2]);

        if self.pending_cancel {
            self.render_cancel_dialog(f);
        }
        if self.pending_delete {
            self.render_delete_dialog(f);
        }
    }

    fn render_cancel_dialog(&self, f: &mut Frame) {
        let dialog = Paragraph::new(Line::from(vec![Span::styled(
            "Discard unsaved changes? (y/n)",
            Style::default().fg(Color::Yellow),
        )]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Confirm ")
                .title_style(Style::default().fg(Color::Red))
                .style(Style::default().bg(Color::Black)),
        );

        let area = ratatui::layout::Rect {
            x: (f.size().width.saturating_sub(40)) / 2,
            y: (f.size().height.saturating_sub(3)) / 2,
            width: 40.min(f.size().width),
            height: 3,
        };

        f.render_widget(dialog, area);
    }

    fn render_delete_dialog(&self, f: &mut Frame) {
        let dialog = Paragraph::new(Line::from(vec![Span::styled(
            "Delete this session? (y/n)",
            Style::default().fg(Color::Red),
        )]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Delete Session ")
                .title_style(Style::default().fg(Color::Red))
                .style(Style::default().bg(Color::Black)),
        );

        let area = ratatui::layout::Rect {
            x: (f.size().width.saturating_sub(40)) / 2,
            y: (f.size().height.saturating_sub(3)) / 2,
            width: 40.min(f.size().width),
            height: 3,
        };

        f.render_widget(dialog, area);
    }

    fn render_title(&self, f: &mut Frame, area: Rect) {
        let title = Paragraph::new(Line::from(vec![Span::styled(
            format!("Editing Session: {}", self.task_name),
            Style::default().fg(Color::Yellow),
        )]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Edit Session ")
                .title_style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(title, area);
    }

    fn render_fields(&self, f: &mut Frame, area: Rect) {
        let mut items: Vec<ListItem> = FIELDS
            .iter()
            .map(|field| {
                let (label, value) = self.get_field_display(field);
                let is_selected = *field == self.current_field && !self.editing_text;
                let is_editing = *field == self.current_field && self.editing_text;

                let prefix = if is_selected {
                    "▶ "
                } else if is_editing {
                    "✎ "
                } else {
                    "  "
                };

                let display_value = if is_editing {
                    let before: String = self.text_buffer.chars().take(self.cursor_pos).collect();
                    let after: String = self.text_buffer.chars().skip(self.cursor_pos).collect();
                    format!("{}|{}", before, after)
                } else {
                    value
                };

                let style = if is_selected || is_editing {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                ListItem::new(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(Color::Cyan)),
                    Span::styled(format!("{:<10}: ", label), Style::default().fg(Color::Gray)),
                    Span::styled(display_value, style),
                ]))
            })
            .collect();

        if let Some(ref error) = self.error_message {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(error, Style::default().fg(Color::Red)),
            ])));
        }

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Fields ")
                .title_style(Style::default().fg(Color::Cyan)),
        );

        let mut state = ListState::default();
        state.select(Some(0));
        f.render_stateful_widget(list, area, &mut state);
    }

    fn get_field_display(&self, field: &Field) -> (String, String) {
        match field {
            Field::StartTime => {
                let local = Local.from_utc_datetime(&self.session.start.naive_utc());
                (
                    "Start".to_string(),
                    local.format("%Y-%m-%d %H:%M").to_string(),
                )
            }
            Field::EndTime => {
                let value = match self.session.end {
                    Some(end) => {
                        let local = Local.from_utc_datetime(&end.naive_utc());
                        local.format("%Y-%m-%d %H:%M").to_string()
                    }
                    None => "(active - no end time)".to_string(),
                };
                ("End".to_string(), value)
            }
            Field::Background => {
                let display = if self.session.is_background {
                    "[x]".to_string()
                } else {
                    "[ ]".to_string()
                };
                ("Background".to_string(), display)
            }
            Field::Notes => {
                let display = if self.session.notes.is_empty() {
                    "(none)".to_string()
                } else {
                    self.session.notes.clone()
                };
                ("Notes".to_string(), display)
            }
            Field::Link => {
                let display = if self.session.link.is_empty() {
                    "(none)".to_string()
                } else {
                    self.session.link.clone()
                };
                ("Link".to_string(), display)
            }
        }
    }

    fn render_help(&self, f: &mut Frame, area: Rect) {
        let help_text = if self.editing_text {
            "Enter: confirm | Esc: cancel | ←→: move | Ctrl+W: delete word"
        } else {
            "↑↓/j/k: navigate | Enter/Space: edit | s: save | d: delete | o: open link | q/Esc: back"
        };

        let help = Paragraph::new(Line::from(vec![Span::styled(
            help_text,
            Style::default().fg(Color::DarkGray),
        )]))
        .block(Block::default().borders(Borders::ALL));

        f.render_widget(help, area);
    }
}

fn parse_datetime_input(input: &str) -> std::result::Result<DateTime<Utc>, String> {
    let input = input.trim();

    let formats = [
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%m/%d %H:%M",
        "%m/%d %H:%M:%S",
        "%m/%d/%Y %H:%M",
        "%m/%d/%Y %H:%M:%S",
        "%d.%m.%Y %H:%M",
        "%d.%m.%Y %H:%M:%S",
    ];

    for fmt in &formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(input, fmt) {
            let local = Local
                .from_local_datetime(&dt)
                .single()
                .ok_or_else(|| format!("Ambiguous or invalid local time: {}", input))?;
            return Ok(local.with_timezone(&Utc));
        }
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(input) {
        return Ok(dt.with_timezone(&Utc));
    }

    Err("Invalid format. Use: YYYY-MM-DD HH:MM or M/D HH:MM".to_string())
}

pub fn edit_session(session: Session, task_name: String) -> Result<SessionEditResult> {
    SessionEditForm::new(session, task_name).run()
}
