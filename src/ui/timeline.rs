use crate::models::EmQuadrant;
use crate::services::TimeBlock;
use chrono::{NaiveDate, Timelike};
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
use std::io;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TimelineError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
}

pub type Result<T> = std::result::Result<T, TimelineError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineResult {
    Closed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomLevel {
    All,
    Morning,
    Afternoon,
    Evening,
}

impl ZoomLevel {
    pub fn time_range(&self) -> (u32, u32) {
        match self {
            ZoomLevel::All => (0, 24),
            ZoomLevel::Morning => (6, 12),
            ZoomLevel::Afternoon => (12, 18),
            ZoomLevel::Evening => (18, 24),
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            ZoomLevel::All => "full day",
            ZoomLevel::Morning => "morning 06:00-12:00",
            ZoomLevel::Afternoon => "afternoon 12:00-18:00",
            ZoomLevel::Evening => "evening 18:00-24:00",
        }
    }

    pub fn key(&self) -> char {
        match self {
            ZoomLevel::All => 'a',
            ZoomLevel::Morning => '1',
            ZoomLevel::Afternoon => '2',
            ZoomLevel::Evening => '3',
        }
    }

    pub fn from_key(key: char) -> Option<Self> {
        match key {
            '1' => Some(ZoomLevel::Morning),
            '2' => Some(ZoomLevel::Afternoon),
            '3' => Some(ZoomLevel::Evening),
            'a' | 'A' => Some(ZoomLevel::All),
            _ => None,
        }
    }
}

pub struct TimelineView {
    blocks: Vec<TimeBlock>,
    date: NaiveDate,
    zoom: ZoomLevel,
    terminal_width: u16,
}

impl TimelineView {
    pub fn new(blocks: Vec<TimeBlock>, date: NaiveDate) -> Self {
        Self {
            blocks,
            date,
            zoom: ZoomLevel::All,
            terminal_width: 80,
        }
    }

    pub fn run(mut self) -> Result<TimelineResult> {
        enable_raw_mode().map_err(|e| TimelineError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| TimelineError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| TimelineError::Terminal(e.to_string()))?;

        self.terminal_width = terminal.size().map(|s| s.width).unwrap_or(80);

        let result = self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| TimelineError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| TimelineError::Terminal(e.to_string()))?;

        result
    }

    fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<TimelineResult> {
        loop {
            terminal
                .draw(|f| self.render(f))
                .map_err(|e| TimelineError::Terminal(e.to_string()))?;

            if event::poll(Duration::from_millis(100))
                .map_err(|e| TimelineError::Terminal(e.to_string()))?
            {
                if let Event::Key(key) =
                    event::read().map_err(|e| TimelineError::Terminal(e.to_string()))?
                {
                    match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c'))
                        | (_, KeyCode::Esc)
                        | (_, KeyCode::Char('q') | KeyCode::Char('Q')) => {
                            return Ok(TimelineResult::Cancelled);
                        }
                        (_, KeyCode::Char(c)) => {
                            if let Some(zoom) = ZoomLevel::from_key(c) {
                                self.zoom = zoom;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn render(&self, f: &mut Frame) {
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
        self.render_timeline(f, chunks[1]);
        self.render_footer(f, chunks[2]);
    }

    fn render_title(&self, f: &mut Frame, area: Rect) {
        let title = format!(
            "Timeline for {} ({})",
            self.date.format("%Y-%m-%d"),
            self.zoom.title()
        );

        let paragraph = Paragraph::new(title).block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(paragraph, area);
    }

    fn render_timeline(&self, f: &mut Frame, area: Rect) {
        let (start_hour, end_hour) = self.zoom.time_range();
        let total_minutes = (end_hour - start_hour) as i64 * 60;
        let available_width = (area.width as usize).saturating_sub(2);

        let visible_blocks: Vec<&TimeBlock> = self
            .blocks
            .iter()
            .filter(|b| {
                let block_start_hour = b.start.hour() as u32;
                block_start_hour < end_hour && start_hour < end_hour
            })
            .collect();

        let mut lines: Vec<Line> = Vec::new();

        let separator_line = "═".repeat(available_width);
        lines.push(Line::styled(
            separator_line.clone(),
            Style::default().fg(Color::Cyan),
        ));

        if visible_blocks.is_empty() {
            lines.push(Line::styled(
                "No sessions in this time range",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            for block in &visible_blocks {
                if let Some(line) =
                    self.render_block_line(block, start_hour, total_minutes, available_width)
                {
                    lines.push(line);
                }
            }
        }

        lines.push(Line::styled(
            separator_line,
            Style::default().fg(Color::Cyan),
        ));

        let (tracked_seconds, untracked_seconds) = self.calculate_totals();
        let tracked_str = format_duration(tracked_seconds);
        let untracked_str = format_duration(untracked_seconds);

        lines.push(Line::from(vec![
            Span::styled("Total: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{} tracked", tracked_str),
                Style::default().fg(Color::Green),
            ),
            Span::styled(", ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{} untracked", untracked_str),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        let paragraph = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));

        f.render_widget(paragraph, area);
    }

    fn render_block_line(
        &self,
        block: &TimeBlock,
        start_hour: u32,
        total_minutes: i64,
        width: usize,
    ) -> Option<Line<'static>> {
        let end = block.end?;

        let block_start_minutes = (block.start.hour() as i64 * 60 + block.start.minute() as i64)
            - (start_hour as i64 * 60);
        let block_end_minutes =
            (end.hour() as i64 * 60 + end.minute() as i64) - (start_hour as i64 * 60);

        if block_end_minutes <= 0 || block_start_minutes >= total_minutes {
            return None;
        }

        let visible_start = block_start_minutes.max(0) as usize;
        let visible_end = (block_end_minutes.min(total_minutes) as usize).min(width);

        let chars_before = (visible_start * width / total_minutes as usize).min(width);
        let chars_block = ((visible_end - visible_start) * width / total_minutes as usize)
            .min(width.saturating_sub(chars_before));

        let (fill_char, color, task_info) = if block.is_gap() {
            ('░', Color::DarkGray, None)
        } else {
            let quadrant = block.quadrant?;
            let color = quadrant_color(&quadrant);
            let task = block.task.as_ref()?;
            let duration_str = format_duration(block.duration_seconds().unwrap_or(0));
            let em_tag = task.em_quadrant().tag();
            let info = format!("{} {} {}", duration_str, task.name, em_tag);
            ('█', color, Some(info))
        };

        let mut spans: Vec<Span> = Vec::new();

        if chars_before > 0 {
            spans.push(Span::raw(" ".repeat(chars_before)));
        }

        if chars_block > 0 {
            spans.push(Span::styled(
                fill_char.to_string().repeat(chars_block),
                Style::default().fg(color),
            ));
        }

        if let Some(info) = task_info {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(info, Style::default().fg(Color::White)));
        }

        Some(Line::from(spans))
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let mut spans: Vec<Span> = Vec::new();

        spans.push(Span::styled("[1]", Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(" Morning  "));

        spans.push(Span::styled("[2]", Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(" Afternoon  "));

        spans.push(Span::styled("[3]", Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(" Evening  "));

        spans.push(Span::styled("[A]", Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(" All day  "));

        spans.push(Span::styled("[Q]", Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(" Quit"));

        let paragraph =
            Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));

        f.render_widget(paragraph, area);
    }

    fn calculate_totals(&self) -> (i64, i64) {
        let mut tracked: i64 = 0;
        let mut untracked: i64 = 0;

        for block in &self.blocks {
            if let Some(seconds) = block.duration_seconds() {
                if block.is_gap() {
                    untracked += seconds;
                } else {
                    tracked += seconds;
                }
            }
        }

        (tracked, untracked)
    }
}

pub fn quadrant_color(quadrant: &EmQuadrant) -> Color {
    match quadrant {
        EmQuadrant::DoFirst => Color::Red,
        EmQuadrant::Schedule => Color::Green,
        EmQuadrant::Delegate => Color::Yellow,
        EmQuadrant::Eliminate => Color::DarkGray,
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

pub fn print_timeline_static(blocks: &[TimeBlock], date: NaiveDate) {
    println!("Timeline for {} (full day)", date.format("%Y-%m-%d"));
    println!("{}", "═".repeat(70));

    if blocks.is_empty() {
        println!("No sessions found for this date");
    } else {
        for block in blocks {
            let start_time = block.start.format("%H:%M");
            let end_time = block
                .end
                .map(|e| e.format("%H:%M").to_string())
                .unwrap_or_else(|| "now".to_string());

            if block.is_gap() {
                println!("{} - {} [untracked]", start_time, end_time);
            } else if let Some(task) = &block.task {
                let duration_str = block
                    .duration_seconds()
                    .map(|s| format_duration(s))
                    .unwrap_or_else(|| "active".to_string());
                let em_tag = task.em_quadrant().tag();
                println!(
                    "{} - {} [{}] {} {}",
                    start_time, end_time, duration_str, task.name, em_tag
                );
            }
        }
    }

    println!("{}", "═".repeat(70));

    let (tracked, untracked) = calculate_totals_static(blocks);
    println!(
        "Total: {} tracked, {} untracked",
        format_duration(tracked),
        format_duration(untracked)
    );
}

fn calculate_totals_static(blocks: &[TimeBlock]) -> (i64, i64) {
    let mut tracked: i64 = 0;
    let mut untracked: i64 = 0;

    for block in blocks {
        if let Some(seconds) = block.duration_seconds() {
            if block.is_gap() {
                untracked += seconds;
            } else {
                tracked += seconds;
            }
        }
    }

    (tracked, untracked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_zoom_level_time_range() {
        assert_eq!(ZoomLevel::All.time_range(), (0, 24));
        assert_eq!(ZoomLevel::Morning.time_range(), (6, 12));
        assert_eq!(ZoomLevel::Afternoon.time_range(), (12, 18));
        assert_eq!(ZoomLevel::Evening.time_range(), (18, 24));
    }

    #[test]
    fn test_zoom_level_from_key() {
        assert_eq!(ZoomLevel::from_key('1'), Some(ZoomLevel::Morning));
        assert_eq!(ZoomLevel::from_key('2'), Some(ZoomLevel::Afternoon));
        assert_eq!(ZoomLevel::from_key('3'), Some(ZoomLevel::Evening));
        assert_eq!(ZoomLevel::from_key('a'), Some(ZoomLevel::All));
        assert_eq!(ZoomLevel::from_key('A'), Some(ZoomLevel::All));
        assert_eq!(ZoomLevel::from_key('x'), None);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3600), "1h 0m");
        assert_eq!(format_duration(3661), "1h 1m");
        assert_eq!(format_duration(7320), "2h 2m");
    }

    #[test]
    fn test_time_block_duration() {
        let start = Utc::now();
        let end = start + chrono::Duration::hours(1);

        let block = TimeBlock {
            start,
            end: Some(end),
            task: None,
            quadrant: None,
        };

        assert_eq!(block.duration_seconds(), Some(3600));
        assert!(block.is_gap());
    }
}
