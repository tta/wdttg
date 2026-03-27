use crate::models::Task;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
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
pub enum PickerError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
}

pub type Result<T> = std::result::Result<T, PickerError>;

#[derive(Debug, Clone, PartialEq)]
pub enum PickerResult {
    Selected(Task),
    CreateNew(String),
    Cancelled,
}

pub struct TaskPicker {
    tasks: Vec<Task>,
    query: String,
    filtered_indices: Vec<usize>,
    list_state: ListState,
    matcher: SkimMatcherV2,
    allow_create: bool,
}

impl TaskPicker {
    pub fn new(tasks: Vec<Task>) -> Self {
        Self::with_options(tasks, true)
    }

    pub fn without_create(tasks: Vec<Task>) -> Self {
        Self::with_options(tasks, false)
    }

    fn with_options(tasks: Vec<Task>, allow_create: bool) -> Self {
        let filtered_indices: Vec<usize> = (0..tasks.len()).collect();
        let mut list_state = ListState::default();
        if !filtered_indices.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            tasks,
            query: String::new(),
            filtered_indices,
            list_state,
            matcher: SkimMatcherV2::default().smart_case().ignore_case(),
            allow_create,
        }
    }

    pub fn run(self) -> Result<PickerResult> {
        enable_raw_mode().map_err(|e| PickerError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| PickerError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| PickerError::Terminal(e.to_string()))?;

        let result = self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| PickerError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| PickerError::Terminal(e.to_string()))?;

        result
    }

    fn run_loop(
        mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<PickerResult> {
        loop {
            terminal
                .draw(|f| self.render(f))
                .map_err(|e| PickerError::Terminal(e.to_string()))?;

            if let Event::Key(key) =
                event::read().map_err(|e| PickerError::Terminal(e.to_string()))?
            {
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                        return Ok(PickerResult::Cancelled);
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('j')) => {
                        self.move_selection(1);
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                        self.move_selection(-1);
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                        self.delete_last_word();
                        self.update_filter();
                    }
                    (_, KeyCode::Enter) => {
                        return Ok(self.select_current());
                    }
                    (_, KeyCode::Char(c)) => {
                        self.query.push(c);
                        self.update_filter();
                    }
                    (_, KeyCode::Backspace) => {
                        self.query.pop();
                        self.update_filter();
                    }
                    (_, KeyCode::Up) => {
                        self.move_selection(-1);
                    }
                    (_, KeyCode::Down) => {
                        self.move_selection(1);
                    }
                    _ => {}
                }
            }
        }
    }

    fn update_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered_indices = (0..self.tasks.len()).collect();
        } else {
            let mut scored: Vec<(usize, i64)> = self
                .tasks
                .iter()
                .enumerate()
                .filter_map(|(i, task)| {
                    self.matcher
                        .fuzzy_match(&task.name, &self.query)
                        .map(|score| (i, score))
                })
                .collect();

            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered_indices = scored.into_iter().map(|(i, _)| i).collect();
        }

        if self.filtered_indices.is_empty() && !self.query.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let current = self.list_state.selected().unwrap_or(0);
        let max = if self.query.is_empty() {
            (self.filtered_indices.len() as i32 - 1).max(0)
        } else if !self.filtered_indices.is_empty() {
            self.filtered_indices.len() as i32
        } else {
            0
        };

        let new_selection = ((current as i32 + delta).clamp(0, max)) as usize;
        self.list_state.select(Some(new_selection));
    }

    fn delete_last_word(&mut self) {
        let trimmed = self.query.trim_end();
        if trimmed.is_empty() {
            self.query.clear();
        } else if let Some(pos) = trimmed.rfind(|c: char| c.is_whitespace()) {
            self.query.truncate(pos);
        } else {
            self.query.clear();
        }
    }

    fn select_current(self) -> PickerResult {
        let selected_idx = self.list_state.selected();

        if self.query.is_empty() {
            if let Some(idx) = selected_idx {
                if idx < self.filtered_indices.len() {
                    let task_idx = self.filtered_indices[idx];
                    return PickerResult::Selected(self.tasks[task_idx].clone());
                }
            }
            return PickerResult::Cancelled;
        }

        if self.filtered_indices.is_empty() {
            if self.allow_create {
                return PickerResult::CreateNew(self.query);
            }
            return PickerResult::Cancelled;
        }

        if !self.allow_create {
            if let Some(idx) = selected_idx {
                if idx < self.filtered_indices.len() {
                    let task_idx = self.filtered_indices[idx];
                    return PickerResult::Selected(self.tasks[task_idx].clone());
                }
            }
            return PickerResult::Cancelled;
        }

        match selected_idx {
            Some(0) => PickerResult::CreateNew(self.query),
            Some(idx) => {
                if idx - 1 < self.filtered_indices.len() {
                    let task_idx = self.filtered_indices[idx - 1];
                    PickerResult::Selected(self.tasks[task_idx].clone())
                } else {
                    PickerResult::CreateNew(self.query)
                }
            }
            None => PickerResult::Cancelled,
        }
    }

    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(f.size());

        self.render_input(f, chunks[0]);
        self.render_list(f, chunks[1]);
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        let input = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(&self.query),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Search Tasks ")
                .title_style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(input, area);
    }

    fn render_list(&self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = if self.query.is_empty() {
            self.tasks
                .iter()
                .map(|task| self.task_to_list_item(task))
                .collect()
        } else if self.filtered_indices.is_empty() {
            if self.allow_create {
                vec![ListItem::new(Line::from(vec![
                    Span::styled("Create new: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        &self.query,
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]))]
            } else {
                vec![ListItem::new(Line::from(Span::styled(
                    "No matching tasks",
                    Style::default().fg(Color::DarkGray),
                )))]
            }
        } else {
            let mut items: Vec<ListItem> = self
                .filtered_indices
                .iter()
                .map(|&idx| self.task_to_list_item(&self.tasks[idx]))
                .collect();

            if self.allow_create {
                let has_exact_match = self.tasks.iter().any(|t| t.name == self.query);
                if !has_exact_match {
                    items.insert(
                        0,
                        ListItem::new(Line::from(vec![
                            Span::styled("Create new: ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                &self.query,
                                Style::default()
                                    .fg(Color::Green)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ])),
                    );
                }
            }

            items
        };

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Tasks ")
                    .title_style(Style::default().fg(Color::Cyan)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        f.render_stateful_widget(list, area, &mut self.list_state.clone());
    }

    fn task_to_list_item<'a>(&self, task: &'a Task) -> ListItem<'a> {
        let em_tag = task.em_quadrant().tag();
        let status_indicator = if task.recurring { "↻ " } else { "" };

        ListItem::new(Line::from(vec![
            Span::styled(status_indicator, Style::default().fg(Color::Yellow)),
            Span::raw(&task.name),
            Span::raw(" "),
            Span::styled(em_tag, Style::default().fg(Color::DarkGray)),
        ]))
    }
}

pub fn filter_tasks(tasks: &[Task], query: &str) -> Vec<Task> {
    if query.is_empty() {
        return tasks.to_vec();
    }

    let matcher = SkimMatcherV2::default().smart_case().ignore_case();
    let mut scored: Vec<(Task, i64)> = tasks
        .iter()
        .filter_map(|task| {
            matcher
                .fuzzy_match(&task.name, query)
                .map(|score| (task.clone(), score))
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(task, _)| task).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskStatus;
    use chrono::Utc;

    fn create_test_task(id: i64, name: &str) -> Task {
        Task {
            id,
            name: name.to_string(),
            notes: None,
            em_urgent: false,
            em_important: true,
            status: TaskStatus::Active,
            recurring: false,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_filter_tasks_empty_query() {
        let tasks = vec![
            create_test_task(1, "meeting/standup"),
            create_test_task(2, "coding/jira"),
        ];

        let result = filter_tasks(&tasks, "");
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_tasks_exact_match() {
        let tasks = vec![
            create_test_task(1, "meeting/standup"),
            create_test_task(2, "coding/jira"),
        ];

        let result = filter_tasks(&tasks, "standup");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "meeting/standup");
    }

    #[test]
    fn test_filter_tasks_fuzzy_match() {
        let tasks = vec![
            create_test_task(1, "meeting/standup"),
            create_test_task(2, "coding/jira-123"),
            create_test_task(3, "code review"),
        ];

        let result = filter_tasks(&tasks, "jra");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "coding/jira-123");
    }

    #[test]
    fn test_filter_tasks_no_match() {
        let tasks = vec![
            create_test_task(1, "meeting/standup"),
            create_test_task(2, "coding/jira"),
        ];

        let result = filter_tasks(&tasks, "nonexistent");
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_tasks_sorted_by_score() {
        let tasks = vec![
            create_test_task(1, "abc"),
            create_test_task(2, "xabcy"),
            create_test_task(3, "xyz"),
        ];

        let result = filter_tasks(&tasks, "abc");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "abc");
        assert_eq!(result[1].name, "xabcy");
    }

    #[test]
    fn test_filter_tasks_case_insensitive() {
        let tasks = vec![
            create_test_task(1, "MEETING/Standup"),
            create_test_task(2, "Coding/JIRA"),
        ];

        let result = filter_tasks(&tasks, "standup");
        assert_eq!(result.len(), 1);

        let result = filter_tasks(&tasks, "coding");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_picker_new() {
        let tasks = vec![create_test_task(1, "task1"), create_test_task(2, "task2")];

        let picker = TaskPicker::new(tasks);
        assert_eq!(picker.tasks.len(), 2);
        assert_eq!(picker.query, "");
        assert_eq!(picker.filtered_indices.len(), 2);
    }

    #[test]
    fn test_picker_result_equality() {
        let task = create_test_task(1, "test");

        let selected = PickerResult::Selected(task.clone());
        let create_new = PickerResult::CreateNew("new task".to_string());
        let cancelled = PickerResult::Cancelled;

        assert!(matches!(selected, PickerResult::Selected(_)));
        assert!(matches!(create_new, PickerResult::CreateNew(_)));
        assert!(matches!(cancelled, PickerResult::Cancelled));
    }
}
