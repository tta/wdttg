use crate::clipboard::{open_in_nvim, open_link, OpenLinkResult};
use crate::models::EmQuadrant;
use crate::services::TimeBlock;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Timelike, Utc};
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
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::collections::HashSet;
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
    EditSession(i64),
    PrevDay,
    NextDay,
    Today,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomLevel {
    All,
    Fit,
}

impl ZoomLevel {
    pub fn title(&self) -> &'static str {
        match self {
            ZoomLevel::All => "full day",
            ZoomLevel::Fit => "fit",
        }
    }

    pub fn from_key(key: char) -> Option<Self> {
        match key {
            'a' | 'A' => Some(ZoomLevel::All),
            'f' | 'F' => Some(ZoomLevel::Fit),
            _ => None,
        }
    }
}

pub struct TimelineView {
    blocks: Vec<TimeBlock>,
    date: NaiveDate,
    zoom: ZoomLevel,
    terminal_width: u16,
    selected_index: usize,
    session_blocks: Vec<(usize, i64)>,
    search_query: String,
    search_active: bool,
    filtered_session_blocks: Vec<(usize, i64)>,
    filtered_selected_index: usize,
    status_message: Option<String>,
    overlapping_session_ids: HashSet<i64>,
}

impl TimelineView {
    pub fn new(blocks: Vec<TimeBlock>, date: NaiveDate) -> Self {
        let session_blocks: Vec<(usize, i64)> = blocks
            .iter()
            .enumerate()
            .filter_map(|(idx, b)| {
                if !b.is_gap() {
                    b.session_id.map(|id| (idx, id))
                } else {
                    None
                }
            })
            .collect();

        let overlapping_session_ids = detect_overlapping_sessions(&blocks);

        Self {
            blocks,
            date,
            zoom: ZoomLevel::Fit,
            terminal_width: 80,
            selected_index: 0,
            session_blocks: session_blocks.clone(),
            search_query: String::new(),
            search_active: false,
            filtered_session_blocks: session_blocks,
            filtered_selected_index: 0,
            status_message: None,
            overlapping_session_ids,
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
                    if self.search_active {
                        match (key.modifiers, key.code) {
                            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                                self.cancel_search();
                            }
                            (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
                                self.select_next_filtered_session();
                            }
                            (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                                self.select_prev_filtered_session();
                            }
                            (_, KeyCode::Enter) => {
                                if let Some((_, session_id)) = self.get_selected_filtered_session()
                                {
                                    return Ok(TimelineResult::EditSession(session_id));
                                }
                            }
                            (_, KeyCode::Backspace) => {
                                self.search_query.pop();
                                self.apply_search_filter();
                            }
                            (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                                self.delete_last_word();
                                self.apply_search_filter();
                            }
                            (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
                                if let Some(link) = self.open_selected_filtered_session_link() {
                                    super::suspend_tui(terminal)
                                        .map_err(|e| TimelineError::Terminal(e.to_string()))?;
                                    let result = open_in_nvim(&link);
                                    super::resume_tui(terminal)
                                        .map_err(|e| TimelineError::Terminal(e.to_string()))?;
                                    self.status_message = Some(match result {
                                        Ok(()) => format!("Opened: {}", link),
                                        Err(e) => format!("Error: {}", e),
                                    });
                                }
                            }
                            (_, KeyCode::Char(c)) => {
                                self.search_query.push(c);
                                self.apply_search_filter();
                            }
                            _ => {}
                        }
                    } else {
                        match (key.modifiers, key.code) {
                            (KeyModifiers::CONTROL, KeyCode::Char('c'))
                            | (_, KeyCode::Char('q') | KeyCode::Char('Q')) => {
                                return Ok(TimelineResult::Cancelled);
                            }
                            (_, KeyCode::Esc) => {
                                return Ok(TimelineResult::Cancelled);
                            }
                            (_, KeyCode::Char('/')) => {
                                self.enter_search_mode();
                            }
                            (_, KeyCode::Enter) => {
                                if let Some((_, session_id)) = self.get_selected_session() {
                                    return Ok(TimelineResult::EditSession(session_id));
                                }
                            }
                            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                                self.select_prev_session();
                            }
                            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                                self.select_next_session();
                            }
                            (_, KeyCode::Left) | (_, KeyCode::Char('h')) => {
                                return Ok(TimelineResult::PrevDay);
                            }
                            (_, KeyCode::Right) | (_, KeyCode::Char('l')) => {
                                return Ok(TimelineResult::NextDay);
                            }
                            (_, KeyCode::Char(c)) => {
                                if let Some(zoom) = ZoomLevel::from_key(c) {
                                    self.zoom = zoom;
                                } else if c == 'o' || c == 'O' {
                                    if let Some(link) = self.open_selected_session_link() {
                                        super::suspend_tui(terminal)
                                            .map_err(|e| TimelineError::Terminal(e.to_string()))?;
                                        let result = open_in_nvim(&link);
                                        super::resume_tui(terminal)
                                            .map_err(|e| TimelineError::Terminal(e.to_string()))?;
                                        self.status_message = Some(match result {
                                            Ok(()) => format!("Opened: {}", link),
                                            Err(e) => format!("Error: {}", e),
                                        });
                                    }
                                } else if c == 't' || c == 'T' {
                                    return Ok(TimelineResult::Today);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    fn enter_search_mode(&mut self) {
        self.search_active = true;
        self.search_query.clear();
        self.filtered_session_blocks = self.session_blocks.clone();
        self.filtered_selected_index = 0;
    }

    fn cancel_search(&mut self) {
        self.search_active = false;
        self.search_query.clear();
        self.filtered_session_blocks = self.session_blocks.clone();
        self.filtered_selected_index = 0;
    }

    fn delete_last_word(&mut self) {
        let trimmed = self.search_query.trim_end();
        if trimmed.is_empty() {
            self.search_query.clear();
        } else if let Some(pos) = trimmed.rfind(|c: char| c.is_whitespace()) {
            self.search_query.truncate(pos);
        } else {
            self.search_query.clear();
        }
    }

    fn apply_search_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_session_blocks = self.session_blocks.clone();
        } else {
            let matcher = SkimMatcherV2::default();
            let mut scored: Vec<(i64, (usize, i64))> = Vec::new();

            for &(block_idx, session_id) in &self.session_blocks {
                if let Some(block) = self.blocks.get(block_idx) {
                    if let Some(task) = &block.task {
                        if let Some(_score) = matcher.fuzzy_match(&task.name, &self.search_query) {
                            scored.push((scored.len() as i64, (block_idx, session_id)));
                        }
                    }
                }
            }

            self.filtered_session_blocks = scored.into_iter().map(|(_, pair)| pair).collect();
        }
        self.filtered_selected_index = 0;
    }

    fn get_selected_filtered_session(&self) -> Option<(usize, i64)> {
        self.filtered_session_blocks
            .get(self.filtered_selected_index)
            .copied()
    }

    fn select_prev_filtered_session(&mut self) {
        if self.filtered_selected_index > 0 {
            self.filtered_selected_index -= 1;
        }
    }

    fn select_next_filtered_session(&mut self) {
        if self.filtered_selected_index + 1 < self.filtered_session_blocks.len() {
            self.filtered_selected_index += 1;
        }
    }

    fn get_selected_session(&self) -> Option<(usize, i64)> {
        self.session_blocks.get(self.selected_index).copied()
    }

    fn select_prev_session(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    fn select_next_session(&mut self) {
        if self.selected_index + 1 < self.session_blocks.len() {
            self.selected_index += 1;
        }
    }

    /// Returns Some(link) if the link is a wiki-link to open in nvim.
    fn open_selected_session_link(&mut self) -> Option<String> {
        if let Some((block_idx, _)) = self.get_selected_session() {
            if let Some(block) = self.blocks.get(block_idx) {
                if let Some(link) = &block.link {
                    match open_link(link) {
                        Ok(OpenLinkResult::OpenedUrl) => {
                            self.status_message = Some("Opened in browser".to_string());
                        }
                        Ok(OpenLinkResult::OpenInEditor { link }) => {
                            return Some(link);
                        }
                        Ok(OpenLinkResult::CopiedPath(path)) => {
                            self.status_message = Some(format!("Copied: {}", path));
                        }
                        Ok(OpenLinkResult::CopiedRaw(text)) => {
                            self.status_message = Some(format!("Copied: {}", text));
                        }
                        Err(e) => {
                            self.status_message = Some(format!("Error: {}", e));
                        }
                    }
                } else {
                    self.status_message = Some("No link for this session".to_string());
                }
            }
        }
        None
    }

    /// Returns Some(link) if the link is a wiki-link to open in nvim.
    fn open_selected_filtered_session_link(&mut self) -> Option<String> {
        if let Some((block_idx, _)) = self.get_selected_filtered_session() {
            if let Some(block) = self.blocks.get(block_idx) {
                if let Some(link) = &block.link {
                    match open_link(link) {
                        Ok(OpenLinkResult::OpenedUrl) => {
                            self.status_message = Some("Opened in browser".to_string());
                        }
                        Ok(OpenLinkResult::OpenInEditor { link }) => {
                            return Some(link);
                        }
                        Ok(OpenLinkResult::CopiedPath(path)) => {
                            self.status_message = Some(format!("Copied: {}", path));
                        }
                        Ok(OpenLinkResult::CopiedRaw(text)) => {
                            self.status_message = Some(format!("Copied: {}", text));
                        }
                        Err(e) => {
                            self.status_message = Some(format!("Error: {}", e));
                        }
                    }
                } else {
                    self.status_message = Some("No link for this session".to_string());
                }
            }
        }
        None
    }

    fn effective_time_range(&self) -> (u32, u32) {
        match self.zoom {
            ZoomLevel::All => (0, 24),
            ZoomLevel::Fit => {
                let mut earliest_hour: Option<u32> = None;
                let mut latest_hour: Option<u32> = None;

                for block in &self.blocks {
                    if block.is_gap() {
                        continue;
                    }
                    let local_start = Local.from_utc_datetime(&block.start.naive_utc());
                    let start_hour = local_start.hour();

                    let end_hour = if let Some(end) = block.end {
                        let local_end = Local.from_utc_datetime(&end.naive_utc());
                        let h = local_end.hour();
                        let m = local_end.minute();
                        if m > 0 {
                            h + 1
                        } else {
                            h
                        }
                    } else {
                        let now = Local::now();
                        let h = now.hour();
                        let m = now.minute();
                        if m > 0 {
                            h + 1
                        } else {
                            h
                        }
                    };

                    if earliest_hour.is_none_or(|e| start_hour < e) {
                        earliest_hour = Some(start_hour);
                    }
                    if latest_hour.is_none_or(|l| end_hour > l) {
                        latest_hour = Some(end_hour);
                    }
                }

                match (earliest_hour, latest_hour) {
                    (Some(s), Some(e)) => {
                        let start = s;
                        let end = e.max(start + 1).min(24);
                        (start, end)
                    }
                    _ => (0, 24),
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
        let title = match self.zoom {
            ZoomLevel::All => format!(
                "Timeline for {} (full day)",
                self.date.format("%a %Y-%m-%d"),
            ),
            ZoomLevel::Fit => {
                let (s, e) = self.effective_time_range();
                format!(
                    "Timeline for {} ({:02}:00-{:02}:00)",
                    self.date.format("%a %Y-%m-%d"),
                    s,
                    e,
                )
            }
        };

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
        let arrow_pos = self.arrow_position(available_width);
        if let Some(pos) = arrow_pos {
            let chars: Vec<char> = markers.chars().collect();
            if pos < chars.len() {
                let before: String = chars[..pos].iter().collect();
                let after: String = chars[pos + 1..].iter().collect();
                lines.push(Line::from(vec![
                    Span::styled(before, Style::default().fg(Color::DarkGray)),
                    Span::styled("▲", Style::default().fg(Color::Yellow)),
                    Span::styled(after, Style::default().fg(Color::DarkGray)),
                ]));
            } else {
                lines.push(Line::styled(markers, Style::default().fg(Color::DarkGray)));
            }
        } else {
            lines.push(Line::styled(markers, Style::default().fg(Color::DarkGray)));
        }

        lines.push(Line::styled(
            separator.clone(),
            Style::default().fg(Color::DarkGray),
        ));

        let (start_hour, end_hour) = self.effective_time_range();
        let visible_blocks: Vec<&TimeBlock> = self
            .blocks
            .iter()
            .filter(|b| {
                let local_start = Local.from_utc_datetime(&b.start.naive_utc());
                let block_start_hour = local_start.hour();
                block_start_hour < end_hour && block_start_hour >= start_hour
            })
            .collect();

        let mut session_blocks: Vec<&TimeBlock> =
            visible_blocks.into_iter().filter(|b| !b.is_gap()).collect();
        session_blocks.sort_by(|a, b| a.start.cmp(&b.start));

        if self.search_active {
            let filtered_blocks: Vec<&TimeBlock> = session_blocks
                .iter()
                .filter(|b| {
                    self.filtered_session_blocks.iter().any(|(idx, _)| {
                        self.blocks
                            .get(*idx)
                            .is_some_and(|fb| fb.session_id == b.session_id && fb.start == b.start)
                    })
                })
                .copied()
                .collect();

            if filtered_blocks.is_empty() {
                lines.push(Line::styled(
                    format!("No matches for \"{}\"", self.search_query),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                let selected_session_id = self.get_selected_filtered_session().map(|(_, id)| id);
                for block in &filtered_blocks {
                    if let Some(line) = self.render_session_line(block, selected_session_id) {
                        lines.push(line);
                    }
                }
            }
        } else if session_blocks.is_empty() {
            lines.push(Line::styled(
                "No sessions in this time range",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            let selected_session_id = self.get_selected_session().map(|(_, id)| id);
            for block in &session_blocks {
                if let Some(line) = self.render_session_line(block, selected_session_id) {
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
        ]));

        let paragraph = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
        f.render_widget(paragraph, area);
    }

    fn arrow_position(&self, width: usize) -> Option<usize> {
        let block_idx = if self.search_active {
            self.get_selected_filtered_session().map(|(idx, _)| idx)
        } else {
            self.get_selected_session().map(|(idx, _)| idx)
        }?;

        let block = self.blocks.get(block_idx)?;
        let (start_hour, end_hour) = self.effective_time_range();
        let total_hours = (end_hour - start_hour) as f64;
        let mins_per_char = (total_hours * 60.0) / width as f64;
        let local_start = Local.from_utc_datetime(&block.start.naive_utc());
        let start_mins = local_start.hour() as f64 * 60.0 + local_start.minute() as f64;
        let zoom_start_mins = start_hour as f64 * 60.0;
        let pos = ((start_mins - zoom_start_mins) / mins_per_char).round() as usize;
        Some(pos.min(width.saturating_sub(1)))
    }

    fn render_24h_bar(&self, width: usize) -> Vec<Span<'static>> {
        let (start_hour, end_hour) = self.effective_time_range();
        let total_hours = (end_hour - start_hour) as f64;
        let mins_per_char = (total_hours * 60.0) / width as f64;
        let mut positions: Vec<(bool, Option<Color>)> = vec![(false, None); width];

        for block in &self.blocks {
            if block.is_gap() {
                continue;
            }

            let color = block.quadrant.as_ref().map(quadrant_color);

            let local_start = Local.from_utc_datetime(&block.start.naive_utc());
            let local_end = block.end.map(|e| Local.from_utc_datetime(&e.naive_utc()));

            let block_start_mins = local_start.hour() as f64 * 60.0 + local_start.minute() as f64;
            let block_end_mins =
                local_end.map_or(24.0 * 60.0, |e| e.hour() as f64 * 60.0 + e.minute() as f64);

            let zoom_start_mins = start_hour as f64 * 60.0;
            let zoom_end_mins = end_hour as f64 * 60.0;

            if block_end_mins <= zoom_start_mins || block_start_mins >= zoom_end_mins {
                continue;
            }

            let visible_start = (block_start_mins - zoom_start_mins).max(0.0);
            let visible_end = (block_end_mins - zoom_start_mins).min(total_hours * 60.0);

            let start_idx = (visible_start / mins_per_char) as usize;
            let end_idx = ((visible_end / mins_per_char) as usize).min(width);

            if start_idx < end_idx {
                for pos in &mut positions[start_idx..end_idx] {
                    if pos.1.is_some() {
                        pos.0 = true;
                    } else {
                        pos.1 = color;
                    }
                }
            }
        }

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current_color: Option<Color> = None;
        let mut current_is_overlap = false;
        let mut current_chars = String::new();

        for (has_overlap, color) in positions {
            let target_color = if has_overlap {
                Some(OVERLAP_COLOR)
            } else {
                color
            };

            let ch = if has_overlap { OVERLAP_CHAR } else { '█' };
            match target_color {
                Some(c) => {
                    if current_color == Some(c) && current_is_overlap == has_overlap {
                        current_chars.push(ch);
                    } else {
                        if !current_chars.is_empty() {
                            let style = current_color
                                .map(|col| Style::default().fg(col))
                                .unwrap_or_default();
                            spans.push(Span::styled(std::mem::take(&mut current_chars), style));
                        }
                        current_color = Some(c);
                        current_is_overlap = has_overlap;
                        current_chars.push(ch);
                    }
                }
                None => {
                    if current_color.is_some() {
                        if !current_chars.is_empty() {
                            let style = current_color
                                .map(|col| Style::default().fg(col))
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
        let (start_hour, end_hour) = self.effective_time_range();
        let total_hours = (end_hour - start_hour) as f64;
        let mut result = vec![' '; width];

        let chars_per_hour = width as f64 / total_hours;
        let hour_step = if chars_per_hour >= 5.0 {
            1
        } else if chars_per_hour >= 2.5 {
            2
        } else if chars_per_hour >= 1.5 {
            3
        } else {
            4
        };
        let mut hour = start_hour;
        while hour <= end_hour {
            let pos = (((hour - start_hour) as f64 / total_hours) * width as f64) as usize;
            let label = hour.to_string();
            let start = pos.min(width.saturating_sub(label.len()));
            for (i, ch) in label.chars().enumerate() {
                if start + i < width {
                    result[start + i] = ch;
                }
            }
            hour += hour_step;
        }

        result.into_iter().collect()
    }

    fn render_session_line(
        &self,
        block: &TimeBlock,
        selected_session_id: Option<i64>,
    ) -> Option<Line<'static>> {
        let task = block.task.as_ref()?;
        let duration_str = format_duration(block.duration_seconds()?);
        let local_start = Local.from_utc_datetime(&block.start.naive_utc());
        let time_str = local_start.format("%H:%M").to_string();
        let em_tag = task.em_quadrant().tag().to_string();
        let color = quadrant_color(&block.quadrant?);
        let name = task.name.clone();
        let link = block.link.clone();
        let bg_indicator = if block.is_background { " [bg]" } else { "" };

        let is_selected = block.session_id.is_some() && block.session_id == selected_session_id;

        let sel_bg = if is_selected {
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

        let prefix = if is_selected {
            Span::styled("▶ ", apply_bg(Style::default().fg(Color::White)))
        } else {
            Span::raw("  ")
        };

        let name_style = if is_selected {
            Style::default()
                .fg(color)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };

        let mut spans = vec![
            prefix,
            Span::styled(time_str, apply_bg(Style::default().fg(Color::White))),
            Span::raw(" "),
            Span::styled(
                format!("[{}]", duration_str),
                apply_bg(Style::default().fg(Color::Cyan)),
            ),
            Span::raw("  "),
            Span::styled(name, name_style),
            Span::raw(" "),
            Span::styled(
                em_tag,
                apply_bg(Style::default().fg(if is_selected {
                    Color::Gray
                } else {
                    Color::DarkGray
                })),
            ),
        ];

        if !bg_indicator.is_empty() {
            spans.push(Span::styled(
                bg_indicator,
                apply_bg(Style::default().fg(Color::Magenta)),
            ));
        }

        let is_overlapping = block
            .session_id
            .map(|id| self.overlapping_session_ids.contains(&id))
            .unwrap_or(false);

        if is_overlapping {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "⚡",
                apply_bg(Style::default().fg(Color::Yellow)),
            ));
        }

        if let Some(link_text) = link {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                link_text,
                apply_bg(Style::default().fg(Color::Magenta)),
            ));
        }

        Some(Line::from(spans))
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let paragraph = if let Some(msg) = &self.status_message {
            Paragraph::new(Line::from(vec![Span::styled(
                msg.clone(),
                Style::default().fg(Color::Green),
            )]))
            .block(Block::default().borders(Borders::ALL))
        } else if self.search_active {
            Paragraph::new(Line::from(vec![
                Span::styled("Search: ", Style::default().fg(Color::Cyan)),
                Span::styled(self.search_query.clone(), Style::default().fg(Color::White)),
                Span::styled("█", Style::default().fg(Color::White)),
            ]))
            .block(Block::default().borders(Borders::ALL))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
                Span::raw(" Select  "),
                Span::styled("[T/H/L]", Style::default().fg(Color::Yellow)),
                Span::raw(" Day  "),
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Edit  "),
                Span::styled("[O]", Style::default().fg(Color::Yellow)),
                Span::raw(" Open Link  "),
                Span::styled("[F/A]", Style::default().fg(Color::Yellow)),
                Span::raw(" Zoom (Fit/All)  "),
                Span::styled("[/]", Style::default().fg(Color::Yellow)),
                Span::raw(" Search  "),
                Span::styled("[Q]", Style::default().fg(Color::Yellow)),
                Span::raw(" Quit"),
            ]))
            .block(Block::default().borders(Borders::ALL))
        };

        f.render_widget(paragraph, area);
    }

    fn calculate_totals(&self) -> (i64, i64) {
        let intervals: Vec<(DateTime<Utc>, DateTime<Utc>)> = self
            .blocks
            .iter()
            .filter(|b| !b.is_gap())
            .filter_map(|b| b.end.map(|e| (b.start, e)))
            .collect();

        let tracked = merge_and_sum_intervals(&intervals);
        (tracked, untracked_from_blocks(&self.blocks, tracked))
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

const OVERLAP_COLOR: Color = Color::Cyan;
const OVERLAP_CHAR: char = '▓';

fn detect_overlapping_sessions(blocks: &[TimeBlock]) -> HashSet<i64> {
    let mut overlapping = HashSet::new();
    let sessions: Vec<&TimeBlock> = blocks.iter().filter(|b| !b.is_gap()).collect();

    for (i, a) in sessions.iter().enumerate() {
        for b in sessions.iter().skip(i + 1) {
            if let (Some(a_end), Some(b_end)) = (a.end, b.end) {
                let overlap_start = a.start.max(b.start);
                let overlap_end = a_end.min(b_end);
                if overlap_start < overlap_end {
                    if let Some(id) = a.session_id {
                        overlapping.insert(id);
                    }
                    if let Some(id) = b.session_id {
                        overlapping.insert(id);
                    }
                }
            }
        }
    }

    overlapping
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

    let overlapping = detect_overlapping_sessions(blocks);

    let mut session_blocks: Vec<&TimeBlock> = blocks.iter().filter(|b| !b.is_gap()).collect();
    session_blocks.sort_by(|a, b| b.start.cmp(&a.start));

    if session_blocks.is_empty() {
        println!("No sessions found for this date");
    } else {
        for block in &session_blocks {
            if let Some(task) = &block.task {
                let local_start = Local.from_utc_datetime(&block.start.naive_utc());
                let time_str = local_start.format("%H:%M");
                let duration_str = block
                    .duration_seconds()
                    .map(format_duration)
                    .unwrap_or_else(|| "active".to_string());
                let em_tag = task.em_quadrant().tag();
                let bg_indicator = if block.is_background { " [bg]" } else { "" };
                let overlap_indicator = block
                    .session_id
                    .map(|id| {
                        if overlapping.contains(&id) {
                            " ⚡"
                        } else {
                            ""
                        }
                    })
                    .unwrap_or("");
                println!(
                    "{} [{}]  {} {}{}{}",
                    time_str, duration_str, task.name, em_tag, bg_indicator, overlap_indicator
                );
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
    let mut positions: Vec<(bool, Option<Color>)> = vec![(false, None); width];

    for block in blocks {
        if block.is_gap() {
            continue;
        }

        let color = block.quadrant.as_ref().map(quadrant_color);

        let local_start = Local.from_utc_datetime(&block.start.naive_utc());
        let local_end = block.end.map(|e| Local.from_utc_datetime(&e.naive_utc()));

        let start_mins = local_start.hour() as f64 * 60.0 + local_start.minute() as f64;
        let end_mins =
            local_end.map_or(24.0 * 60.0, |e| e.hour() as f64 * 60.0 + e.minute() as f64);

        let start_idx = (start_mins / mins_per_char) as usize;
        let end_idx = ((end_mins / mins_per_char) as usize).min(width);

        if start_idx < end_idx {
            for pos in &mut positions[start_idx..end_idx] {
                if pos.1.is_some() {
                    pos.0 = true;
                } else {
                    pos.1 = color;
                }
            }
        }
    }

    let mut result = String::new();
    let mut current_color: Option<Color> = None;

    for (has_overlap, color) in positions {
        let target_color = if has_overlap {
            Some(OVERLAP_COLOR)
        } else {
            color
        };
        let ch = if has_overlap { OVERLAP_CHAR } else { '█' };

        if current_color != target_color {
            if current_color.is_some() {
                result.push_str("\x1b[0m");
            }
            if let Some(c) = target_color {
                result.push_str(color_to_ansi(c));
            }
            current_color = target_color;
        }
        result.push(ch);
    }

    if current_color.is_some() {
        result.push_str("\x1b[0m");
    }

    println!("{}", result);
}

fn color_to_ansi(color: Color) -> &'static str {
    match color {
        Color::Red => "\x1b[31m",
        Color::Green => "\x1b[32m",
        Color::Yellow => "\x1b[33m",
        Color::DarkGray => "\x1b[90m",
        Color::Magenta => "\x1b[35m",
        Color::Cyan => "\x1b[36m",
        Color::White => "\x1b[37m",
        _ => "\x1b[0m",
    }
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
    let intervals: Vec<(DateTime<Utc>, DateTime<Utc>)> = blocks
        .iter()
        .filter(|b| !b.is_gap())
        .filter_map(|b| b.end.map(|e| (b.start, e)))
        .collect();

    let tracked = merge_and_sum_intervals(&intervals);
    (tracked, untracked_from_blocks(blocks, tracked))
}

fn merge_and_sum_intervals(intervals: &[(DateTime<Utc>, DateTime<Utc>)]) -> i64 {
    if intervals.is_empty() {
        return 0;
    }

    let mut sorted: Vec<_> = intervals.to_vec();
    sorted.sort_by_key(|(s, _)| *s);

    let mut total: i64 = 0;
    let mut merged_start = sorted[0].0;
    let mut merged_end = sorted[0].1;

    for (start, end) in sorted.iter().skip(1) {
        if *start < merged_end {
            merged_end = merged_end.max(*end);
        } else {
            total += (merged_end - merged_start).num_seconds();
            merged_start = *start;
            merged_end = *end;
        }
    }
    total += (merged_end - merged_start).num_seconds();

    total
}

fn untracked_from_blocks(blocks: &[TimeBlock], _tracked: i64) -> i64 {
    blocks
        .iter()
        .filter(|b| b.is_gap())
        .filter_map(|b| b.duration_seconds())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Task;
    use chrono::Utc;

    #[test]
    fn test_effective_time_range_all() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());
        view.zoom = ZoomLevel::All;
        assert_eq!(view.effective_time_range(), (0, 24));
    }

    #[test]
    fn test_effective_time_range_fit() {
        let local_tz = Local::now().timezone();
        let today = Local::now().date_naive();

        let start_local = today.and_hms_opt(9, 15, 0).unwrap();
        let end_local = today.and_hms_opt(16, 40, 0).unwrap();

        let start_utc = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        let blocks = vec![TimeBlock {
            session_id: Some(1),
            start: start_utc,
            end: Some(end_utc),
            task: Some(create_test_task(1, "test-task")),
            quadrant: Some(EmQuadrant::Schedule),
            link: None,
            is_background: false,
        }];

        let mut view = TimelineView::new(blocks, today);
        view.zoom = ZoomLevel::Fit;
        assert_eq!(view.effective_time_range(), (9, 17));
    }

    #[test]
    fn test_effective_time_range_fit_empty() {
        let now = Utc::now();
        let blocks = vec![TimeBlock {
            session_id: None,
            start: now,
            end: Some(now + chrono::Duration::hours(1)),
            task: None,
            quadrant: None,
            link: None,
            is_background: false,
        }];

        let mut view = TimelineView::new(blocks, Utc::now().date_naive());
        view.zoom = ZoomLevel::Fit;
        assert_eq!(view.effective_time_range(), (0, 24));
    }

    #[test]
    fn test_zoom_level_from_key() {
        assert_eq!(ZoomLevel::from_key('a'), Some(ZoomLevel::All));
        assert_eq!(ZoomLevel::from_key('A'), Some(ZoomLevel::All));
        assert_eq!(ZoomLevel::from_key('f'), Some(ZoomLevel::Fit));
        assert_eq!(ZoomLevel::from_key('F'), Some(ZoomLevel::Fit));
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
            session_id: None,
            start,
            end: Some(end),
            task: None,
            quadrant: None,
            link: None,
            is_background: false,
        };

        assert_eq!(block.duration_seconds(), Some(3600));
        assert!(block.is_gap());
    }

    fn create_test_task(id: i64, name: &str) -> Task {
        Task {
            id,
            name: name.to_string(),
            notes: None,
            em_urgent: false,
            em_important: true,
            status: crate::models::TaskStatus::Active,
            recurring: false,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn create_test_blocks() -> Vec<TimeBlock> {
        let now = Utc::now();
        vec![
            TimeBlock {
                session_id: None,
                start: now,
                end: Some(now + chrono::Duration::minutes(30)),
                task: None,
                quadrant: None,
                link: None,
                is_background: false,
            },
            TimeBlock {
                session_id: Some(1),
                start: now + chrono::Duration::minutes(30),
                end: Some(now + chrono::Duration::hours(1)),
                task: Some(create_test_task(1, "task-1")),
                quadrant: Some(EmQuadrant::Schedule),
                link: None,
                is_background: false,
            },
            TimeBlock {
                session_id: None,
                start: now + chrono::Duration::hours(1),
                end: Some(now + chrono::Duration::hours(2)),
                task: None,
                quadrant: None,
                link: None,
                is_background: false,
            },
            TimeBlock {
                session_id: Some(2),
                start: now + chrono::Duration::hours(2),
                end: Some(now + chrono::Duration::hours(3)),
                task: Some(create_test_task(2, "task-2")),
                quadrant: Some(EmQuadrant::DoFirst),
                link: None,
                is_background: false,
            },
        ]
    }

    #[test]
    fn test_timeline_view_session_blocks_filters_gaps() {
        let blocks = create_test_blocks();
        let view = TimelineView::new(blocks, Utc::now().date_naive());

        assert_eq!(view.session_blocks.len(), 2);
        assert_eq!(view.session_blocks[0], (1, 1));
        assert_eq!(view.session_blocks[1], (3, 2));
    }

    #[test]
    fn test_timeline_view_get_selected_session() {
        let blocks = create_test_blocks();
        let view = TimelineView::new(blocks, Utc::now().date_naive());

        assert_eq!(view.get_selected_session(), Some((1, 1)));
    }

    #[test]
    fn test_timeline_view_select_next_session() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());

        assert_eq!(view.get_selected_session(), Some((1, 1)));

        view.select_next_session();
        assert_eq!(view.get_selected_session(), Some((3, 2)));

        view.select_next_session();
        assert_eq!(view.get_selected_session(), Some((3, 2)));
    }

    #[test]
    fn test_timeline_view_select_prev_session() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());

        view.selected_index = 1;
        assert_eq!(view.get_selected_session(), Some((3, 2)));

        view.select_prev_session();
        assert_eq!(view.get_selected_session(), Some((1, 1)));

        view.select_prev_session();
        assert_eq!(view.get_selected_session(), Some((1, 1)));
    }

    #[test]
    fn test_timeline_view_no_session_blocks() {
        let now = Utc::now();
        let blocks = vec![TimeBlock {
            session_id: None,
            start: now,
            end: Some(now + chrono::Duration::hours(1)),
            task: None,
            quadrant: None,
            link: None,
            is_background: false,
        }];

        let view = TimelineView::new(blocks, Utc::now().date_naive());

        assert!(view.session_blocks.is_empty());
        assert_eq!(view.get_selected_session(), None);
    }

    #[test]
    fn test_timeline_view_enter_search_mode() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());

        assert!(!view.search_active);
        view.enter_search_mode();
        assert!(view.search_active);
        assert!(view.search_query.is_empty());
    }

    #[test]
    fn test_timeline_view_cancel_search() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());

        view.enter_search_mode();
        view.search_query = "test".to_string();
        view.cancel_search();
        assert!(!view.search_active);
        assert!(view.search_query.is_empty());
    }

    #[test]
    fn test_timeline_view_search_filter() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());

        view.enter_search_mode();
        view.search_query = "task-1".to_string();
        view.apply_search_filter();

        assert_eq!(view.filtered_session_blocks.len(), 1);
    }

    #[test]
    fn test_timeline_view_search_filter_empty_query() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());

        view.search_query = String::new();
        view.apply_search_filter();

        assert_eq!(
            view.filtered_session_blocks.len(),
            view.session_blocks.len()
        );
    }

    #[test]
    fn test_timeline_view_filtered_session_navigation() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());

        view.search_query = "task".to_string();
        view.apply_search_filter();

        assert_eq!(view.filtered_selected_index, 0);
        view.select_next_filtered_session();
        assert_eq!(view.filtered_selected_index, 1);
        view.select_next_filtered_session();
        assert_eq!(view.filtered_selected_index, 1);
        view.select_prev_filtered_session();
        assert_eq!(view.filtered_selected_index, 0);
    }

    #[test]
    fn test_timeline_view_delete_last_word() {
        let blocks = create_test_blocks();
        let mut view = TimelineView::new(blocks, Utc::now().date_naive());

        view.search_query = "hello world test".to_string();
        view.delete_last_word();
        assert_eq!(view.search_query, "hello world");

        view.delete_last_word();
        assert_eq!(view.search_query, "hello");

        view.delete_last_word();
        assert_eq!(view.search_query, "");

        view.search_query = "   ".to_string();
        view.delete_last_word();
        assert_eq!(view.search_query, "");
    }

    #[test]
    fn test_detect_overlapping_sessions_no_overlap() {
        let now = Utc::now();
        let blocks = vec![
            TimeBlock {
                session_id: Some(1),
                start: now,
                end: Some(now + chrono::Duration::hours(1)),
                task: Some(create_test_task(1, "task-1")),
                quadrant: Some(EmQuadrant::Schedule),
                link: None,
                is_background: false,
            },
            TimeBlock {
                session_id: Some(2),
                start: now + chrono::Duration::hours(1),
                end: Some(now + chrono::Duration::hours(2)),
                task: Some(create_test_task(2, "task-2")),
                quadrant: Some(EmQuadrant::DoFirst),
                link: None,
                is_background: false,
            },
        ];

        let overlapping = detect_overlapping_sessions(&blocks);
        assert!(overlapping.is_empty());
    }

    #[test]
    fn test_detect_overlapping_sessions_full_overlap() {
        let now = Utc::now();
        let blocks = vec![
            TimeBlock {
                session_id: Some(1),
                start: now,
                end: Some(now + chrono::Duration::hours(2)),
                task: Some(create_test_task(1, "bg-task")),
                quadrant: Some(EmQuadrant::Schedule),
                link: None,
                is_background: true,
            },
            TimeBlock {
                session_id: Some(2),
                start: now,
                end: Some(now + chrono::Duration::hours(2)),
                task: Some(create_test_task(2, "fg-task")),
                quadrant: Some(EmQuadrant::DoFirst),
                link: None,
                is_background: false,
            },
        ];

        let overlapping = detect_overlapping_sessions(&blocks);
        assert_eq!(overlapping.len(), 2);
        assert!(overlapping.contains(&1));
        assert!(overlapping.contains(&2));
    }

    #[test]
    fn test_detect_overlapping_sessions_partial_overlap() {
        let now = Utc::now();
        let blocks = vec![
            TimeBlock {
                session_id: Some(1),
                start: now,
                end: Some(now + chrono::Duration::hours(1)),
                task: Some(create_test_task(1, "bg-task")),
                quadrant: Some(EmQuadrant::Schedule),
                link: None,
                is_background: true,
            },
            TimeBlock {
                session_id: Some(2),
                start: now + chrono::Duration::minutes(30),
                end: Some(now + chrono::Duration::hours(1) + chrono::Duration::minutes(30)),
                task: Some(create_test_task(2, "fg-task")),
                quadrant: Some(EmQuadrant::DoFirst),
                link: None,
                is_background: false,
            },
        ];

        let overlapping = detect_overlapping_sessions(&blocks);
        assert_eq!(overlapping.len(), 2);
        assert!(overlapping.contains(&1));
        assert!(overlapping.contains(&2));
    }

    #[test]
    fn test_render_24h_bar_midnight_spanning_session_no_panic() {
        let local_tz = Local::now().timezone();
        let today = Local::now().date_naive();
        let tomorrow = today + chrono::Duration::days(1);

        let start_local = today.and_hms_opt(23, 0, 0).unwrap();
        let end_local = tomorrow.and_hms_opt(1, 0, 0).unwrap();

        let start_utc = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        let blocks = vec![TimeBlock {
            session_id: Some(1),
            start: start_utc,
            end: Some(end_utc),
            task: Some(create_test_task(1, "midnight-task")),
            quadrant: Some(EmQuadrant::Schedule),
            link: None,
            is_background: false,
        }];

        let view = TimelineView::new(blocks, today);
        let _spans = view.render_24h_bar(78);
    }

    #[test]
    fn test_render_24h_bar_clamped_session_renders_correctly() {
        let local_tz = Local::now().timezone();
        let today = Local::now().date_naive();

        let start_local = today.and_hms_opt(23, 0, 0).unwrap();
        let end_local = today.and_hms_opt(23, 59, 59).unwrap();

        let start_utc = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        let blocks = vec![TimeBlock {
            session_id: Some(1),
            start: start_utc,
            end: Some(end_utc),
            task: Some(create_test_task(1, "clamped-task")),
            quadrant: Some(EmQuadrant::Schedule),
            link: None,
            is_background: false,
        }];

        let view = TimelineView::new(blocks, today);
        let spans = view.render_24h_bar(78);

        let filled: String = spans
            .iter()
            .flat_map(|s| s.content.chars())
            .filter(|c| *c != ' ')
            .collect();
        assert!(!filled.is_empty());
    }

    #[test]
    fn test_print_24h_bar_static_midnight_spanning_no_panic() {
        let local_tz = Local::now().timezone();
        let today = Local::now().date_naive();
        let tomorrow = today + chrono::Duration::days(1);

        let start_local = today.and_hms_opt(23, 0, 0).unwrap();
        let end_local = tomorrow.and_hms_opt(1, 0, 0).unwrap();

        let start_utc = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        let blocks = vec![TimeBlock {
            session_id: Some(1),
            start: start_utc,
            end: Some(end_utc),
            task: Some(create_test_task(1, "midnight-task")),
            quadrant: Some(EmQuadrant::Schedule),
            link: None,
            is_background: false,
        }];

        print_24h_bar_static(&blocks, 78);
    }
}
