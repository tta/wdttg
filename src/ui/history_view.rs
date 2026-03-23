use crate::models::SessionWithTask;
use crate::ui::quadrant_color;
use chrono::{Local, NaiveDate, TimeZone};
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
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::collections::BTreeMap;
use std::io;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HistoryViewError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
}

pub type Result<T> = std::result::Result<T, HistoryViewError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryViewResult {
    Done,
    Back,
    PeriodChanged(Period),
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

pub struct HistoryView {
    sessions: Vec<SessionWithTask>,
    tag: String,
    period: Period,
    scroll_offset: usize,
}

impl HistoryView {
    pub fn new(sessions: Vec<SessionWithTask>, tag: String) -> Self {
        Self {
            sessions,
            tag,
            period: Period::Week,
            scroll_offset: 0,
        }
    }

    pub fn with_period(mut self, period: Period) -> Self {
        self.period = period;
        self
    }

    pub fn period(&self) -> Period {
        self.period
    }

    pub fn run(mut self) -> Result<HistoryViewResult> {
        enable_raw_mode().map_err(|e| HistoryViewError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| HistoryViewError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| HistoryViewError::Terminal(e.to_string()))?;

        let result = self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| HistoryViewError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| HistoryViewError::Terminal(e.to_string()))?;

        result
    }

    fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<HistoryViewResult> {
        loop {
            terminal
                .draw(|f| self.render(f))
                .map_err(|e| HistoryViewError::Terminal(e.to_string()))?;

            if event::poll(Duration::from_millis(100))
                .map_err(|e| HistoryViewError::Terminal(e.to_string()))?
            {
                if let Event::Key(key) =
                    event::read().map_err(|e| HistoryViewError::Terminal(e.to_string()))?
                {
                    match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c'))
                        | (_, KeyCode::Char('q') | KeyCode::Char('Q')) => {
                            return Ok(HistoryViewResult::Done);
                        }
                        (_, KeyCode::Esc) => {
                            return Ok(HistoryViewResult::Back);
                        }
                        (_, KeyCode::Up) => {
                            self.scroll_up();
                        }
                        (_, KeyCode::Down) => {
                            self.scroll_down();
                        }
                        (_, KeyCode::Char('j')) => {
                            self.scroll_down();
                        }
                        (_, KeyCode::Char('k')) => {
                            self.scroll_up();
                        }
                        (_, KeyCode::Char(c)) => {
                            if let Some(new_period) = Period::from_key(c) {
                                self.period = new_period;
                                return Ok(HistoryViewResult::PeriodChanged(new_period));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    fn scroll_down(&mut self) {
        let total_lines = self.count_display_lines();
        if self.scroll_offset + 1 < total_lines {
            self.scroll_offset += 1;
        }
    }

    fn count_display_lines(&self) -> usize {
        let grouped = self.group_by_date();
        let mut count = 0;
        for (_, sessions) in grouped {
            count += 1;
            count += sessions.len();
        }
        count
    }

    fn group_by_date(&self) -> BTreeMap<NaiveDate, Vec<&SessionWithTask>> {
        let mut grouped: BTreeMap<NaiveDate, Vec<&SessionWithTask>> = BTreeMap::new();

        for session in &self.sessions {
            let local_start = Local.from_utc_datetime(&session.session.start_time.naive_utc());
            let date = local_start.date_naive();
            grouped.entry(date).or_default().push(session);
        }

        grouped
    }

    fn render(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(3),
            ])
            .split(f.size());

        self.render_title(f, chunks[0]);
        self.render_content(f, chunks[1]);
        self.render_footer(f, chunks[2]);
    }

    fn render_title(&self, f: &mut Frame, area: Rect) {
        let title = format!("Tag: {} ({})", self.tag, self.period.display_str());

        let paragraph = Paragraph::new(title).block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(paragraph, area);
    }

    fn render_content(&mut self, f: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        if self.sessions.is_empty() {
            lines.push(Line::styled(
                format!("No sessions found for tag '{}' in this period", self.tag),
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            let grouped = self.group_by_date();
            let mut current_line = 0;
            let visible_height = area.height as usize;
            let visible_lines =
                self.get_visible_lines(&grouped, self.scroll_offset, visible_height);

            for (date_str, session_lines) in visible_lines {
                if current_line >= self.scroll_offset
                    && current_line < self.scroll_offset + visible_height
                {
                    lines.push(Line::styled(
                        date_str,
                        Style::default().fg(Color::White).bg(Color::DarkGray),
                    ));
                }
                current_line += 1;

                for line in session_lines {
                    if current_line >= self.scroll_offset
                        && current_line < self.scroll_offset + visible_height
                    {
                        lines.push(line);
                    }
                    current_line += 1;
                }
            }
        }

        let paragraph = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
        f.render_widget(paragraph, area);
    }

    fn get_visible_lines(
        &self,
        grouped: &BTreeMap<NaiveDate, Vec<&SessionWithTask>>,
        offset: usize,
        max_lines: usize,
    ) -> Vec<(String, Vec<Line<'static>>)> {
        let mut result = Vec::new();
        let mut line_count = 0;

        for (date, sessions) in grouped.iter().rev() {
            let date_str = date.format("%Y-%m-%d").to_string();
            let mut session_lines = Vec::new();

            for swt in sessions {
                let session = &swt.session;
                let task = &swt.task;

                let local_start = Local.from_utc_datetime(&session.start_time.naive_utc());
                let time_str = local_start.format("%H:%M").to_string();

                let duration_str = session
                    .duration_seconds()
                    .map(format_duration)
                    .unwrap_or_else(|| "active".to_string());

                let em_quadrant = task.em_quadrant();
                let color = quadrant_color(&em_quadrant);
                let em_tag = em_quadrant.tag();

                session_lines.push(Line::from(vec![
                    Span::styled(format!("  {}", time_str), Style::default().fg(Color::White)),
                    Span::raw(" "),
                    Span::styled(
                        format!("[{}]", duration_str),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw("  "),
                    Span::styled(task.name.clone(), Style::default().fg(color)),
                    Span::raw(" "),
                    Span::styled(em_tag.to_string(), Style::default().fg(Color::DarkGray)),
                ]));
            }

            let session_count = session_lines.len();
            if line_count + 1 + session_count > offset && result.len() < max_lines {
                result.push((date_str, session_lines));
            }
            line_count += 1 + session_count;

            if result.len() >= max_lines {
                break;
            }
        }

        result
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
            Span::raw(" Scroll  "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Back  "),
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

pub fn print_history_static(sessions: &[SessionWithTask], tag: &str, period: Period) {
    println!("Tag: {} ({})", tag, period.display_str());
    println!("{}", "=".repeat(60));

    if sessions.is_empty() {
        println!("No sessions found for tag '{}' in this period", tag);
        return;
    }

    let mut grouped: BTreeMap<NaiveDate, Vec<&SessionWithTask>> = BTreeMap::new();
    for session in sessions {
        let local_start = Local.from_utc_datetime(&session.session.start_time.naive_utc());
        let date = local_start.date_naive();
        grouped.entry(date).or_default().push(session);
    }

    for (date, date_sessions) in grouped.iter().rev() {
        println!("\n{}", date.format("%Y-%m-%d"));

        for swt in date_sessions {
            let session = &swt.session;
            let task = &swt.task;

            let local_start = Local.from_utc_datetime(&session.start_time.naive_utc());
            let time_str = local_start.format("%H:%M").to_string();

            let duration_str = session
                .duration_seconds()
                .map(format_duration)
                .unwrap_or_else(|| "active".to_string());

            let em_tag = task.em_quadrant().tag();

            println!(
                "  {} [{}]  {} {}",
                time_str, duration_str, task.name, em_tag
            );
        }
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
    fn test_format_duration() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3600), "1h 0m");
        assert_eq!(format_duration(3661), "1h 1m");
    }
}
