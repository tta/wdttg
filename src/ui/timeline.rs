use crate::clipboard::{copy_to_clipboard, resolve_note_path};
use crate::models::EmQuadrant;
use crate::services::TimeBlock;
use chrono::{Local, NaiveDate, TimeZone, Timelike};
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
    selected_index: usize,
    session_blocks: Vec<(usize, i64)>,
    search_query: String,
    search_active: bool,
    filtered_session_blocks: Vec<(usize, i64)>,
    filtered_selected_index: usize,
    status_message: Option<String>,
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

        Self {
            blocks,
            date,
            zoom: ZoomLevel::All,
            terminal_width: 80,
            selected_index: 0,
            session_blocks: session_blocks.clone(),
            search_query: String::new(),
            search_active: false,
            filtered_session_blocks: session_blocks,
            filtered_selected_index: 0,
            status_message: None,
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
                            (KeyModifiers::CONTROL, KeyCode::Char('j')) => {
                                self.select_next_filtered_session();
                            }
                            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
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
                                self.copy_selected_filtered_session_note();
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
                            (_, KeyCode::Char(c)) => {
                                if let Some(zoom) = ZoomLevel::from_key(c) {
                                    self.zoom = zoom;
                                } else if c == 'o' || c == 'O' {
                                    self.copy_selected_session_note();
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

    fn copy_selected_session_note(&mut self) {
        if let Some((block_idx, _)) = self.get_selected_session() {
            if let Some(block) = self.blocks.get(block_idx) {
                if let Some(note) = &block.note {
                    match resolve_note_path(note) {
                        Some(path) => match copy_to_clipboard(&path) {
                            Ok(()) => {
                                self.status_message = Some(format!("Copied: {}", path));
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Error: {}", e));
                            }
                        },
                        None => {
                            self.status_message = Some("Error: WDTTG_VAULT not set".to_string());
                        }
                    }
                } else {
                    self.status_message = Some("No note for this session".to_string());
                }
            }
        }
    }

    fn copy_selected_filtered_session_note(&mut self) {
        if let Some((block_idx, _)) = self.get_selected_filtered_session() {
            if let Some(block) = self.blocks.get(block_idx) {
                if let Some(note) = &block.note {
                    match resolve_note_path(note) {
                        Some(path) => match copy_to_clipboard(&path) {
                            Ok(()) => {
                                self.status_message = Some(format!("Copied: {}", path));
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Error: {}", e));
                            }
                        },
                        None => {
                            self.status_message = Some("Error: WDTTG_VAULT not set".to_string());
                        }
                    }
                } else {
                    self.status_message = Some("No note for this session".to_string());
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

    fn render_24h_bar(&self, width: usize) -> Vec<Span<'static>> {
        let (start_hour, end_hour) = self.zoom.time_range();
        let total_hours = (end_hour - start_hour) as f64;
        let mins_per_char = (total_hours * 60.0) / width as f64;
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
        let (start_hour, end_hour) = self.zoom.time_range();
        let total_hours = (end_hour - start_hour) as f64;
        let mut result = vec![' '; width];

        let hour_step = if total_hours <= 6.0 { 1 } else { 2 };
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
        let note = block.note.clone();

        let is_selected = block.session_id.is_some() && block.session_id == selected_session_id;

        let prefix = if is_selected {
            Span::styled("▶ ", Style::default().fg(Color::Yellow))
        } else {
            Span::raw("  ")
        };

        let name_style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };

        let mut spans = vec![
            prefix,
            Span::styled(time_str, Style::default().fg(Color::White)),
            Span::raw(" "),
            Span::styled(
                format!("[{}]", duration_str),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(name, name_style),
            Span::raw(" "),
            Span::styled(em_tag, Style::default().fg(Color::DarkGray)),
        ];

        if let Some(note_text) = note {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(note_text, Style::default().fg(Color::Magenta)));
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
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Edit  "),
                Span::styled("[O]", Style::default().fg(Color::Yellow)),
                Span::raw(" Open Note  "),
                Span::styled("[1-3/A]", Style::default().fg(Color::Yellow)),
                Span::raw(" Zoom  "),
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
                let local_start = Local.from_utc_datetime(&block.start.naive_utc());
                let time_str = local_start.format("%H:%M");
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

        let local_start = Local.from_utc_datetime(&block.start.naive_utc());
        let local_end = block.end.map(|e| Local.from_utc_datetime(&e.naive_utc()));

        let start_mins = local_start.hour() as f64 * 60.0 + local_start.minute() as f64;
        let end_mins =
            local_end.map_or(24.0 * 60.0, |e| e.hour() as f64 * 60.0 + e.minute() as f64);

        let start_idx = (start_mins / mins_per_char) as usize;
        let end_idx = ((end_mins / mins_per_char) as usize).min(width);

        if start_idx < end_idx {
            for ch in &mut bar[start_idx..end_idx] {
                *ch = '█';
            }
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
    use crate::models::Task;
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
            session_id: None,
            start,
            end: Some(end),
            task: None,
            quadrant: None,
            note: None,
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
                note: None,
            },
            TimeBlock {
                session_id: Some(1),
                start: now + chrono::Duration::minutes(30),
                end: Some(now + chrono::Duration::hours(1)),
                task: Some(create_test_task(1, "task-1")),
                quadrant: Some(EmQuadrant::Schedule),
                note: None,
            },
            TimeBlock {
                session_id: None,
                start: now + chrono::Duration::hours(1),
                end: Some(now + chrono::Duration::hours(2)),
                task: None,
                quadrant: None,
                note: None,
            },
            TimeBlock {
                session_id: Some(2),
                start: now + chrono::Duration::hours(2),
                end: Some(now + chrono::Duration::hours(3)),
                task: Some(create_test_task(2, "task-2")),
                quadrant: Some(EmQuadrant::DoFirst),
                note: None,
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
            note: None,
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
}
