use crate::models::TagWithTime;
use chrono::NaiveDate;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TagPickerError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
}

pub type Result<T> = std::result::Result<T, TagPickerError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagPickerResult {
    Selected(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
}

impl Period {
    pub fn from_key(key: char) -> Option<Self> {
        match key.to_lowercase().next() {
            Some('d') => Some(Period::Day),
            Some('w') => Some(Period::Week),
            Some('m') => Some(Period::Month),
            _ => None,
        }
    }

    pub fn to_period_str(&self) -> &'static str {
        match self {
            Period::Day => "day",
            Period::Week => "week",
            Period::Month => "month",
        }
    }

    pub fn date_range(&self) -> (NaiveDate, NaiveDate) {
        let today = chrono::Local::now().date_naive();
        match self {
            Period::Day => (today, today),
            Period::Week => {
                let start = today - chrono::Duration::days(6);
                (start, today)
            }
            Period::Month => {
                let start = today - chrono::Duration::days(29);
                (start, today)
            }
        }
    }

    pub fn display_str(&self) -> String {
        match self {
            Period::Day => "Today".to_string(),
            Period::Week => "Last 7 days".to_string(),
            Period::Month => "Last 30 days".to_string(),
        }
    }
}

pub struct TagPicker {
    tags: Vec<TagWithTime>,
    state: ListState,
    period: Period,
}

impl TagPicker {
    pub fn new(tags: Vec<TagWithTime>) -> Self {
        let mut state = ListState::default();
        if !tags.is_empty() {
            state.select(Some(0));
        }
        Self {
            tags,
            state,
            period: Period::Week,
        }
    }

    pub fn with_period(mut self, period: Period) -> Self {
        self.period = period;
        self
    }

    pub fn period(&self) -> Period {
        self.period
    }

    pub fn run(mut self) -> Result<TagPickerResult> {
        if self.tags.is_empty() {
            return Ok(TagPickerResult::Cancelled);
        }

        enable_raw_mode().map_err(|e| TagPickerError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| TagPickerError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| TagPickerError::Terminal(e.to_string()))?;

        let result = self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| TagPickerError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| TagPickerError::Terminal(e.to_string()))?;

        result
    }

    fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<TagPickerResult> {
        loop {
            terminal
                .draw(|f| self.render(f))
                .map_err(|e| TagPickerError::Terminal(e.to_string()))?;

            if event::poll(Duration::from_millis(100))
                .map_err(|e| TagPickerError::Terminal(e.to_string()))?
            {
                if let Event::Key(key) =
                    event::read().map_err(|e| TagPickerError::Terminal(e.to_string()))?
                {
                    match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c'))
                        | (_, KeyCode::Esc)
                        | (_, KeyCode::Char('q') | KeyCode::Char('Q')) => {
                            return Ok(TagPickerResult::Cancelled);
                        }
                        (_, KeyCode::Up) => {
                            self.move_up();
                        }
                        (_, KeyCode::Down) => {
                            self.move_down();
                        }
                        (_, KeyCode::Char('j')) => {
                            self.move_down();
                        }
                        (_, KeyCode::Char('k')) => {
                            self.move_up();
                        }
                        (_, KeyCode::Enter) => {
                            if let Some(tag) = self.selected_tag() {
                                return Ok(TagPickerResult::Selected(tag.tag.clone()));
                            }
                        }
                        (_, KeyCode::Char(c)) => {
                            if let Some(period) = Period::from_key(c) {
                                self.period = period;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn move_up(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.tags.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn move_down(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.tags.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn selected_tag(&self) -> Option<&TagWithTime> {
        self.state.selected().and_then(|i| self.tags.get(i))
    }

    fn render(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(f.size());

        self.render_title(f, chunks[0]);
        self.render_list(f, chunks[1]);
        self.render_footer(f, chunks[2]);
    }

    fn render_title(&self, f: &mut Frame, area: Rect) {
        let title = format!("Select Tag ({})", self.period.display_str());

        let paragraph = Paragraph::new(title).block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(paragraph, area);
    }

    fn render_list(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .tags
            .iter()
            .map(|tag| {
                let time_str = format_duration(tag.total_seconds);
                let content = format!(
                    "  {:<20} {:>10}  ({:>3} sessions)",
                    tag.tag, time_str, tag.session_count
                );
                ListItem::new(content)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(Style::default().fg(Color::Yellow))
            .highlight_symbol("> ");

        f.render_stateful_widget(list, area, &mut self.state);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(Line::from(vec![
            Span::styled("[D]", Style::default().fg(Color::Yellow)),
            Span::raw(" Day  "),
            Span::styled("[W]", Style::default().fg(Color::Yellow)),
            Span::raw(" Week  "),
            Span::styled("[M]", Style::default().fg(Color::Yellow)),
            Span::raw(" Month  "),
            Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
            Span::raw(" Navigate  "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Select  "),
            Span::styled("[Q]", Style::default().fg(Color::Yellow)),
            Span::raw(" Quit"),
        ]))
        .block(Block::default().borders(Borders::ALL));

        f.render_widget(paragraph, area);
    }
}

fn format_duration(seconds: i64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

pub fn print_tags_static(tags: &[TagWithTime], period: Period) {
    println!("Tags ({})", period.display_str());
    println!("{}", "=".repeat(50));

    if tags.is_empty() {
        println!("No tagged sessions found for this period");
        return;
    }

    for tag in tags {
        let time_str = format_duration(tag.total_seconds);
        println!(
            "  {:<20} {:>10}  ({:>3} sessions)",
            tag.tag, time_str, tag.session_count
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_period_from_key() {
        assert_eq!(Period::from_key('d'), Some(Period::Day));
        assert_eq!(Period::from_key('D'), Some(Period::Day));
        assert_eq!(Period::from_key('w'), Some(Period::Week));
        assert_eq!(Period::from_key('W'), Some(Period::Week));
        assert_eq!(Period::from_key('m'), Some(Period::Month));
        assert_eq!(Period::from_key('M'), Some(Period::Month));
        assert_eq!(Period::from_key('x'), None);
    }

    #[test]
    fn test_period_date_range() {
        let today = chrono::Local::now().date_naive();

        let (start, end) = Period::Day.date_range();
        assert_eq!(start, today);
        assert_eq!(end, today);

        let (start, end) = Period::Week.date_range();
        assert_eq!(end, today);
        assert_eq!((end - start).num_days(), 6);

        let (start, end) = Period::Month.date_range();
        assert_eq!(end, today);
        assert_eq!((end - start).num_days(), 29);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3600), "1h 0m");
        assert_eq!(format_duration(3661), "1h 1m");
    }
}
