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
        let available_width = (area.width as usize).saturating_sub(2);
        let mut lines: Vec<Line> = Vec::new();

        let separator = "─".repeat(available_width);
        lines.push(Line::styled(
            separator.clone(),
            Style::default().fg(Color::DarkGray),
        ));

        let bar_spans = self.render_24h_bar(available_width);
        lines.push(Line::from(bar_spans));

        let markers = self.render_hour_markers(available_width);
        lines.push(Line::styled(markers, Style::default().fg(Color::DarkGray)));

        lines.push(Line::styled(
            separator.clone(),
            Style::default().fg(Color::DarkGray),
        ));

        let (start_hour, end_hour) = self.zoom.time_range();
        let visible_blocks: Vec<&TimeBlock> = self
            .blocks
            .iter()
            .filter(|b| {
                let block_start_hour = b.start.hour();
                block_start_hour < end_hour && block_start_hour >= start_hour
            })
            .collect();

        let mut session_blocks: Vec<&TimeBlock> =
            visible_blocks.into_iter().filter(|b| !b.is_gap()).collect();
        session_blocks.sort_by(|a, b| b.start.cmp(&a.start));

        if session_blocks.is_empty() {
            lines.push(Line::styled(
                "No sessions in this time range",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            for block in &session_blocks {
                if let Some(line) = self.render_session_line(block) {
                    lines.push(line);
                }
            }
        }

        lines.push(Line::styled(
            separator,
            Style::default().fg(Color::DarkGray),
        ));

        let (tracked_seconds, untracked_seconds) = self.calculate_totals();
        let tracked_str = format_duration(tracked_seconds);
        let untracked_str = format_duration(untracked_seconds);

        lines.push(Line::from(vec![
            Span::styled("Tracked: ", Style::default().fg(Color::White)),
            Span::styled(tracked_str, Style::default().fg(Color::Green)),
            Span::styled("  |  Untracked: ", Style::default().fg(Color::White)),
            Span::styled(untracked_str, Style::default().fg(Color::DarkGray)),
            Span::styled("  |  ", Style::default().fg(Color::White)),
            Span::styled("[1-3/A]", Style::default().fg(Color::Yellow)),
            Span::styled(" Zoom  ", Style::default().fg(Color::White)),
            Span::styled("[Q]", Style::default().fg(Color::Yellow)),
            Span::styled(" Quit", Style::default().fg(Color::White)),
        ]));

        let paragraph = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
        f.render_widget(paragraph, area);
    }

    fn render_24h_bar(&self, width: usize) -> Vec<Span<'static>> {
        let mins_per_char = (24.0 * 60.0) / width as f64;
        let mut positions: Vec<Option<(Color, char)>> = vec![None; width];

        for block in &self.blocks {
            if block.is_gap() {
                continue;
            }

            let quadrant = match &block.quadrant {
                Some(q) => q,
                None => continue,
            };
            let color = quadrant_color(quadrant);

            let start_mins = block.start.hour() as f64 * 60.0 + block.start.minute() as f64;
            let end_mins = block
                .end
                .map_or(24.0 * 60.0, |e| e.hour() as f64 * 60.0 + e.minute() as f64);

            let start_idx = (start_mins / mins_per_char) as usize;
            let end_idx = ((end_mins / mins_per_char) as usize).min(width);

            for pos in &mut positions[start_idx..end_idx] {
                *pos = Some((color, '█'));
            }
        }

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current_color: Option<Color> = None;
        let mut current_chars = String::new();

        for pos in positions {
            match pos {
                Some((color, ch)) => {
                    if current_color == Some(color) {
                        current_chars.push(ch);
                    } else {
                        if !current_chars.is_empty() {
                            let style = current_color
                                .map(|c| Style::default().fg(c))
                                .unwrap_or_default();
                            spans.push(Span::styled(std::mem::take(&mut current_chars), style));
                        }
                        current_color = Some(color);
                        current_chars.push(ch);
                    }
                }
                None => {
                    if current_color.is_some() {
                        if !current_chars.is_empty() {
                            let style = current_color
                                .map(|c| Style::default().fg(c))
                                .unwrap_or_default();
                            spans.push(Span::styled(std::mem::take(&mut current_chars), style));
                        }
                        current_color = None;
                    }
                    current_chars.push(' ');
                }
            }
        }

        if !current_chars.is_empty() {
            let style = current_color
                .map(|c| Style::default().fg(c))
                .unwrap_or_default();
            spans.push(Span::styled(current_chars, style));
        }

        spans
    }

    fn render_hour_markers(&self, width: usize) -> String {
        let hours = [0, 4, 8, 12, 16, 20, 24];
        let mut result = vec![' '; width];

        for &hour in &hours {
            let pos = ((hour as f64 / 24.0) * width as f64) as usize;
            let label = hour.to_string();
            let start = pos.min(width.saturating_sub(label.len()));
            for (i, ch) in label.chars().enumerate() {
                if start + i < width {
                    result[start + i] = ch;
                }
            }
        }

        result.into_iter().collect()
    }

    fn render_session_line(&self, block: &TimeBlock) -> Option<Line<'static>> {
        let task = block.task.as_ref()?;
        let duration_str = format_duration(block.duration_seconds()?);
        let time_str = block.start.format("%H:%M").to_string();
        let em_tag = task.em_quadrant().tag().to_string();
        let color = quadrant_color(&block.quadrant?);
        let name = task.name.clone();

        Some(Line::from(vec![
            Span::styled(time_str, Style::default().fg(Color::White)),
            Span::raw(" "),
            Span::styled(
                format!("[{}]", duration_str),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(name, Style::default().fg(color)),
            Span::raw(" "),
            Span::styled(em_tag, Style::default().fg(Color::DarkGray)),
        ]))
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(Line::from(vec![
            Span::styled("[1]", Style::default().fg(Color::Yellow)),
            Span::raw(" Morning  "),
            Span::styled("[2]", Style::default().fg(Color::Yellow)),
            Span::raw(" Afternoon  "),
            Span::styled("[3]", Style::default().fg(Color::Yellow)),
            Span::raw(" Evening  "),
            Span::styled("[A]", Style::default().fg(Color::Yellow)),
            Span::raw(" All  "),
            Span::styled("[Q]", Style::default().fg(Color::Yellow)),
            Span::raw(" Quit"),
        ]))
        .block(Block::default().borders(Borders::ALL));

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
    println!("Timeline for {}", date.format("%Y-%m-%d"));
    let width = 78;
    println!("{}", "─".repeat(width));

    print_24h_bar_static(blocks, width);
    print_hour_markers_static(width);

    println!("{}", "─".repeat(width));

    let mut session_blocks: Vec<&TimeBlock> = blocks.iter().filter(|b| !b.is_gap()).collect();
    session_blocks.sort_by(|a, b| b.start.cmp(&a.start));

    if session_blocks.is_empty() {
        println!("No sessions found for this date");
    } else {
        for block in &session_blocks {
            if let Some(task) = &block.task {
                let time_str = block.start.format("%H:%M");
                let duration_str = block
                    .duration_seconds()
                    .map(format_duration)
                    .unwrap_or_else(|| "active".to_string());
                let em_tag = task.em_quadrant().tag();
                println!("{} [{}]  {} {}", time_str, duration_str, task.name, em_tag);
            }
        }
    }

    println!("{}", "─".repeat(width));

    let (tracked, untracked) = calculate_totals_static(blocks);
    println!(
        "Tracked: {}  |  Untracked: {}",
        format_duration(tracked),
        format_duration(untracked)
    );
}

fn print_24h_bar_static(blocks: &[TimeBlock], width: usize) {
    let mins_per_char = (24.0 * 60.0) / width as f64;
    let mut bar: Vec<char> = vec![' '; width];

    for block in blocks {
        if block.is_gap() {
            continue;
        }

        let start_mins = block.start.hour() as f64 * 60.0 + block.start.minute() as f64;
        let end_mins = block
            .end
            .map_or(24.0 * 60.0, |e| e.hour() as f64 * 60.0 + e.minute() as f64);

        let start_idx = (start_mins / mins_per_char) as usize;
        let end_idx = ((end_mins / mins_per_char) as usize).min(width);

        for ch in &mut bar[start_idx..end_idx] {
            *ch = '█';
        }
    }

    println!("{}", bar.into_iter().collect::<String>());
}

fn print_hour_markers_static(width: usize) {
    let hours = [0, 4, 8, 12, 16, 20, 24];
    let mut result = vec![' '; width];

    for &hour in &hours {
        let pos = ((hour as f64 / 24.0) * width as f64) as usize;
        let label = hour.to_string();
        let start = pos.min(width.saturating_sub(label.len()));
        for (i, ch) in label.chars().enumerate() {
            if start + i < width {
                result[start + i] = ch;
            }
        }
    }

    println!("{}", result.into_iter().collect::<String>());
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
