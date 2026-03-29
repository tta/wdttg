use crate::models::{EmQuadrant, Task, TaskStatus};
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
pub enum EditFormError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
}

pub type Result<T> = std::result::Result<T, EditFormError>;

#[derive(Debug, Clone, PartialEq)]
pub enum EditFormResult {
    Saved(Task),
    Deleted(i64),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct EditableTask {
    pub id: i64,
    pub name: String,
    pub notes: String,
    pub em_quadrant: EmQuadrant,
    pub status: TaskStatus,
    pub recurring: bool,
}

impl From<Task> for EditableTask {
    fn from(task: Task) -> Self {
        Self {
            id: task.id,
            name: task.name.clone(),
            notes: task.notes.clone().unwrap_or_default(),
            em_quadrant: task.em_quadrant(),
            status: task.status,
            recurring: task.recurring,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Field {
    Name,
    Notes,
    EmQuadrant,
    Status,
    Recurring,
}

const FIELDS: [Field; 5] = [
    Field::Name,
    Field::Notes,
    Field::EmQuadrant,
    Field::Status,
    Field::Recurring,
];

const EM_OPTIONS: [(EmQuadrant, &str); 4] = [
    (EmQuadrant::DoFirst, "Do First"),
    (EmQuadrant::Schedule, "Schedule"),
    (EmQuadrant::Delegate, "Delegate"),
    (EmQuadrant::Eliminate, "Eliminate"),
];

const STATUS_OPTIONS: [(TaskStatus, &str); 3] = [
    (TaskStatus::Active, "active"),
    (TaskStatus::Completed, "completed"),
    (TaskStatus::Archived, "archived"),
];

pub struct EditForm {
    task: EditableTask,
    original: EditableTask,
    current_field: Field,
    editing_text: bool,
    text_buffer: String,
    cursor_pos: usize,
    em_index: usize,
    status_index: usize,
    pending_cancel: bool,
    pending_delete: bool,
}

impl EditForm {
    pub fn new(task: Task) -> Self {
        let editable: EditableTask = task.into();
        let original = editable.clone();
        let em_index = EM_OPTIONS
            .iter()
            .position(|(q, _)| *q == editable.em_quadrant)
            .unwrap_or(0);
        let status_index = STATUS_OPTIONS
            .iter()
            .position(|(s, _)| *s == editable.status)
            .unwrap_or(0);

        Self {
            task: editable,
            original,
            current_field: Field::Name,
            editing_text: false,
            text_buffer: String::new(),
            cursor_pos: 0,
            em_index,
            status_index,
            pending_cancel: false,
            pending_delete: false,
        }
    }

    fn has_changes(&self) -> bool {
        self.task.name != self.original.name
            || self.task.notes != self.original.notes
            || self.task.em_quadrant != self.original.em_quadrant
            || self.task.status != self.original.status
            || self.task.recurring != self.original.recurring
    }

    pub fn run(mut self) -> Result<EditFormResult> {
        enable_raw_mode().map_err(|e| EditFormError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| EditFormError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| EditFormError::Terminal(e.to_string()))?;

        let result = self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| EditFormError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| EditFormError::Terminal(e.to_string()))?;

        result
    }

    fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<EditFormResult> {
        loop {
            terminal
                .draw(|f| self.render(f))
                .map_err(|e| EditFormError::Terminal(e.to_string()))?;

            if let Event::Key(key) =
                event::read().map_err(|e| EditFormError::Terminal(e.to_string()))?
            {
                if self.pending_cancel {
                    match (key.modifiers, key.code) {
                        (_, KeyCode::Char('y') | KeyCode::Char('Y')) => {
                            return Ok(EditFormResult::Cancelled);
                        }
                        (_, KeyCode::Char('n') | KeyCode::Char('N')) | (_, KeyCode::Esc) => {
                            self.pending_cancel = false;
                        }
                        _ => {}
                    }
                } else if self.pending_delete {
                    match (key.modifiers, key.code) {
                        (_, KeyCode::Char('y') | KeyCode::Char('Y')) => {
                            return Ok(EditFormResult::Deleted(self.task.id));
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
                        }
                        (_, KeyCode::Enter) => {
                            self.apply_text_edit();
                            self.editing_text = false;
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
                        (_, KeyCode::Delete) => {
                            if self.cursor_pos < self.text_buffer.len() {
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
                                return Ok(EditFormResult::Cancelled);
                            }
                        }
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                            return Ok(EditFormResult::Cancelled);
                        }
                        (_, KeyCode::Char('j')) => {
                            self.move_field(1);
                        }
                        (_, KeyCode::Char('k')) => {
                            self.move_field(-1);
                        }
                        (_, KeyCode::Char('s')) => {
                            return Ok(EditFormResult::Saved(Task {
                                id: self.task.id,
                                name: self.task.name.clone(),
                                notes: if self.task.notes.is_empty() {
                                    None
                                } else {
                                    Some(self.task.notes.clone())
                                },
                                em_urgent: self.task.em_quadrant.is_urgent(),
                                em_important: self.task.em_quadrant.is_important(),
                                status: self.task.status,
                                recurring: self.task.recurring,
                                tags: vec![],
                                created_at: chrono::Utc::now(),
                                updated_at: chrono::Utc::now(),
                            }));
                        }
                        (_, KeyCode::Char('d')) => {
                            self.pending_delete = true;
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
                        (_, KeyCode::Left) => {
                            self.adjust_field(-1);
                        }
                        (_, KeyCode::Char('1' | '2' | '3' | '4')) => {
                            if self.current_field == Field::EmQuadrant {
                                use crate::models::EmQuadrant;
                                let quadrant = match key.code {
                                    KeyCode::Char('1') => EmQuadrant::DoFirst,
                                    KeyCode::Char('2') => EmQuadrant::Schedule,
                                    KeyCode::Char('3') => EmQuadrant::Delegate,
                                    KeyCode::Char('4') => EmQuadrant::Eliminate,
                                    _ => unreachable!(),
                                };
                                self.task.em_quadrant = quadrant;
                                self.em_index = EM_OPTIONS
                                    .iter()
                                    .position(|(q, _)| *q == quadrant)
                                    .unwrap_or(0);
                            }
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
            Field::Name => {
                self.text_buffer = self.task.name.clone();
                self.cursor_pos = self.text_buffer.len();
                self.editing_text = true;
            }
            Field::Notes => {
                self.text_buffer = self.task.notes.clone();
                self.cursor_pos = self.text_buffer.len();
                self.editing_text = true;
            }
            Field::EmQuadrant => {
                self.em_index = (self.em_index + 1) % EM_OPTIONS.len();
                self.task.em_quadrant = EM_OPTIONS[self.em_index].0;
            }
            Field::Status => {
                self.status_index = (self.status_index + 1) % STATUS_OPTIONS.len();
                self.task.status = STATUS_OPTIONS[self.status_index].0;
            }
            Field::Recurring => {
                self.task.recurring = !self.task.recurring;
            }
        }
    }

    fn adjust_field(&mut self, delta: i32) {
        match self.current_field {
            Field::EmQuadrant => {
                let new_idx =
                    ((self.em_index as i32 + delta).rem_euclid(EM_OPTIONS.len() as i32)) as usize;
                self.em_index = new_idx;
                self.task.em_quadrant = EM_OPTIONS[self.em_index].0;
            }
            Field::Status => {
                let new_idx = ((self.status_index as i32 + delta)
                    .rem_euclid(STATUS_OPTIONS.len() as i32))
                    as usize;
                self.status_index = new_idx;
                self.task.status = STATUS_OPTIONS[self.status_index].0;
            }
            _ => {}
        }
    }

    fn apply_text_edit(&mut self) {
        match self.current_field {
            Field::Name => {
                self.task.name = self.text_buffer.clone();
            }
            Field::Notes => {
                self.task.notes = self.text_buffer.clone();
            }
            _ => {}
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

    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(12),
                Constraint::Length(3),
            ])
            .split(f.size());

        self.render_title(f, chunks[0]);
        self.render_fields(f, chunks[1]);
        self.render_help(f, chunks[2]);

        if self.pending_cancel {
            self.render_confirm_dialog(f);
        }
        if self.pending_delete {
            self.render_delete_dialog(f);
        }
    }

    fn render_confirm_dialog(&self, f: &mut Frame) {
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
            "Delete this task? (y/n)",
            Style::default().fg(Color::Red),
        )]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Delete Task ")
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
            format!("Editing: {}", self.task.name),
            Style::default().fg(Color::Yellow),
        )]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Edit Task ")
                .title_style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(title, area);
    }

    fn render_fields(&self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = FIELDS
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

                let sel_bg = if is_selected || is_editing {
                    Some(Color::DarkGray)
                } else {
                    None
                };

                let apply_bg = |style: Style| {
                    if let Some(bg) = sel_bg {
                        style.bg(bg)
                    } else {
                        style
                    }
                };

                let value_style = if is_selected || is_editing {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                ListItem::new(Line::from(vec![
                    Span::styled(prefix, apply_bg(Style::default().fg(Color::Cyan))),
                    Span::styled(
                        format!("{:<10}: ", label),
                        apply_bg(Style::default().fg(Color::Gray)),
                    ),
                    Span::styled(display_value, value_style),
                ]))
            })
            .collect();

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
            Field::Name => ("Name".to_string(), self.task.name.clone()),
            Field::Notes => ("Notes".to_string(), self.task.notes.clone()),
            Field::EmQuadrant => (
                "EM".to_string(),
                format!(
                    "{} - {}",
                    self.task.em_quadrant.tag(),
                    self.task.em_quadrant.description()
                ),
            ),
            Field::Status => ("Status".to_string(), self.task.status.to_string()),
            Field::Recurring => (
                "Recurring".to_string(),
                if self.task.recurring { "Yes" } else { "No" }.to_string(),
            ),
        }
    }

    fn render_help(&self, f: &mut Frame, area: Rect) {
        let help_text = if self.editing_text {
            "Enter: confirm | Esc: cancel | ←→: move | Ctrl+W: delete word"
        } else {
            "↑↓/j/k: navigate | Enter/Space: edit | ←: cycle | s: save | d: delete | q/Esc: back"
        };

        let help = Paragraph::new(Line::from(vec![Span::styled(
            help_text,
            Style::default().fg(Color::DarkGray),
        )]))
        .block(Block::default().borders(Borders::ALL));

        f.render_widget(help, area);
    }
}

pub fn edit_task(task: Task) -> Result<EditFormResult> {
    EditForm::new(task).run()
}
