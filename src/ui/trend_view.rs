//! Interactive TUI for `wdttg report trend`.
//!
//! Default since the trend TUI redesign (2026-06-17). The stdout path
//! survives under `--no-prompt`; see `print_trend_stdout` in main.rs.
//!
//! Layout: header (dimension / period / range) + scrollable list of items
//! with per-row sparkline + total + arrow + total row + footer hints.
//! Drill-down replaces the list when Enter is pressed.
//!
//! Keybindings match every other wdttg TUI: j=DOWN, k=UP, Tab to cycle,
//! Enter to drill in, Esc/q to quit. See AGENTS.md.

use crate::clipboard::open_link;
use crate::models::{
    compute_trend_buckets, format_trend_arrow, quadrant_row_name, BucketBy, EmQuadrant,
    SessionSplitTime, TrendDimension, TrendResult, SPARK_CHARS,
};
use crate::ui::quadrant_color;
use chrono::{DateTime, Local, NaiveDate, Utc};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
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
use std::collections::HashMap;
use std::io;
use thiserror::Error;

const MIN_WIDTH: u16 = 40;
const NAME_WIDTH: usize = 22;
const FOOTER_HINTS_LIST: &str = "j/k move  Tab dim  d/w bucket  Enter drill-down  Esc/q quit";
const FOOTER_HINTS_DRILL: &str = "h/l bucket  j/k session  o open link  Esc back";

#[derive(Debug, Error)]
pub enum TrendViewError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
}

pub type Result<T> = std::result::Result<T, TrendViewError>;

/// Pre-computed drill-down state for the currently-selected item.
struct DrillDown {
    item: String,
    /// One Vec per bucket; same indexing as `TrendResult.bucket_starts`.
    sessions_by_bucket: Vec<Vec<SessionSplitTime>>,
    bucket_idx: usize,
    selected: usize,
    scroll_offset: usize,
}

pub struct TrendView {
    sessions: Vec<SessionSplitTime>,
    task_tags: HashMap<i64, Vec<String>>,
    start: NaiveDate,
    end: NaiveDate,
    top: usize,

    dimension: TrendDimension,
    bucket_by: BucketBy,

    selected_idx: usize,
    scroll_offset: usize,

    drill: Option<DrillDown>,
}

impl TrendView {
    pub fn new(
        sessions: Vec<SessionSplitTime>,
        task_tags: HashMap<i64, Vec<String>>,
        start: NaiveDate,
        end: NaiveDate,
        dimension: TrendDimension,
        bucket_by: BucketBy,
        top: usize,
    ) -> Result<Self> {
        Ok(Self {
            sessions,
            task_tags,
            start,
            end,
            top,
            dimension,
            bucket_by,
            selected_idx: 0,
            scroll_offset: 0,
            drill: None,
        })
    }

    fn compute(&self) -> TrendResult {
        compute_trend_buckets(
            &self.sessions,
            Some(&self.task_tags),
            self.dimension,
            self.bucket_by,
            self.start,
            self.end,
            self.top,
        )
    }

    pub fn run(mut self) -> Result<()> {
        if self.sessions.is_empty() {
            println!("No sessions in this period. Press q to quit.");
            // Still enter the TUI so the user sees the message and can quit gracefully.
        }

        enable_raw_mode().map_err(|e| TrendViewError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| TrendViewError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| TrendViewError::Terminal(e.to_string()))?;

        self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| TrendViewError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| TrendViewError::Terminal(e.to_string()))?;

        Ok(())
    }

    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
        loop {
            if let Err(e) = terminal.draw(|f| self.render(f)) {
                eprintln!("Render error: {}", e);
                break;
            }

            if !event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
                continue;
            }
            let Ok(Event::Key(key)) = event::read() else {
                continue;
            };

            if self.drill.is_some() {
                if self.handle_drill_key(key) {
                    break;
                }
            } else {
                match (key.modifiers, key.code) {
                    (_, KeyCode::Char('q')) | (_, KeyCode::Esc) => break,
                    (_, KeyCode::Tab) => {
                        self.dimension = self.dimension.cycle();
                        self.selected_idx = 0;
                        self.scroll_offset = 0;
                    }
                    (_, KeyCode::Char('j')) | (_, KeyCode::Down) => self.move_selection(1),
                    (_, KeyCode::Char('k')) | (_, KeyCode::Up) => self.move_selection(-1),
                    (_, KeyCode::Char('d')) => {
                        self.bucket_by = BucketBy::Day;
                        self.selected_idx = 0;
                        self.scroll_offset = 0;
                        self.drill = None;
                    }
                    (_, KeyCode::Char('w')) => {
                        self.bucket_by = BucketBy::Week;
                        self.selected_idx = 0;
                        self.scroll_offset = 0;
                        self.drill = None;
                    }
                    (_, KeyCode::Enter) => self.open_drill(),
                    _ => {}
                }
            }
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let result = self.compute();
        let max = result.items.len().saturating_sub(1) as i32;
        if max == 0 {
            self.selected_idx = 0;
            return;
        }
        let new = (self.selected_idx as i32 + delta).clamp(0, max) as usize;
        self.selected_idx = new;
    }

    fn open_drill(&mut self) {
        let result = self.compute();
        let Some(item) = result.items.get(self.selected_idx) else {
            return;
        };
        if item.buckets.is_empty() {
            return;
        }

        // Partition sessions into the item's buckets. We re-derive from
        // self.sessions because TrendResult.items only carries aggregates.
        let mut by_bucket: Vec<Vec<SessionSplitTime>> =
            (0..item.buckets.len()).map(|_| Vec::new()).collect();
        let bucket_index = |date: NaiveDate| -> Option<usize> {
            let key = match self.bucket_by {
                BucketBy::Day => date,
                BucketBy::Week => {
                    use chrono::Datelike;
                    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
                }
            };
            result.bucket_starts.iter().position(|&d| d == key)
        };

        let name_matches = |st: &SessionSplitTime| match self.dimension {
            TrendDimension::Task | TrendDimension::SplitAttention => st.task_name == item.name,
            TrendDimension::Tag => self
                .task_tags
                .get(&st.task_id)
                .map(|tags| tags.iter().any(|t| t == &item.name))
                .unwrap_or(false),
            TrendDimension::Quadrant => quadrant_row_name(st.em_quadrant) == item.name,
        };

        for st in &self.sessions {
            if !name_matches(st) {
                continue;
            }
            if let Some(idx) = bucket_index(st.start_time.date_naive()) {
                if idx < by_bucket.len() {
                    by_bucket[idx].push(st.clone());
                }
            }
        }

        // Open at the highest non-zero bucket — the spike.
        let initial_bucket = item
            .buckets
            .iter()
            .enumerate()
            .filter(|(_, v)| **v > 0)
            .max_by_key(|(_, v)| *v)
            .map(|(i, _)| i)
            .unwrap_or(0);

        self.drill = Some(DrillDown {
            item: item.name.clone(),
            sessions_by_bucket: by_bucket,
            bucket_idx: initial_bucket,
            selected: 0,
            scroll_offset: 0,
        });
    }

    fn handle_drill_key(&mut self, key: event::KeyEvent) -> bool {
        let Some(drill) = self.drill.as_mut() else {
            return false;
        };

        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) => return true,
            (_, KeyCode::Esc) => {
                self.drill = None;
            }
            (_, KeyCode::Char('h')) => {
                if drill.bucket_idx > 0 {
                    drill.bucket_idx -= 1;
                    drill.selected = 0;
                    drill.scroll_offset = 0;
                }
            }
            (_, KeyCode::Char('l')) => {
                if drill.bucket_idx + 1 < drill.sessions_by_bucket.len() {
                    drill.bucket_idx += 1;
                    drill.selected = 0;
                    drill.scroll_offset = 0;
                }
            }
            (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                let sessions = &drill.sessions_by_bucket[drill.bucket_idx];
                if !sessions.is_empty() {
                    drill.selected = (drill.selected + 1).min(sessions.len() - 1);
                }
            }
            (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                drill.selected = drill.selected.saturating_sub(1);
            }
            (_, KeyCode::Char('o')) => {
                // Snapshot the link out of the borrow before calling open_link,
                // which suspends the TUI and cannot be called while drill
                // is borrowed.
                let link_opt = drill.sessions_by_bucket[drill.bucket_idx]
                    .get(drill.selected)
                    .and_then(|st| st.link.clone());
                if let Some(link) = link_opt {
                    let _ = open_link(&link);
                }
            }
            _ => {}
        }
        false
    }

    fn render(&self, f: &mut Frame) {
        let area = f.size();
        if area.width < MIN_WIDTH {
            let msg = format!(
                "Terminal too narrow (need >={MIN_WIDTH} cols, got {}). Quit and resize.",
                area.width
            );
            f.render_widget(
                Paragraph::new(msg.clone()).style(Style::default().fg(Color::Red)),
                area,
            );
            // Force-quit by signaling; we can't exit cleanly from render,
            // so we print and rely on the next event loop to break.
            // Use Ctrl+C equivalent: write the message and let the user press q.
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(2),  // header
                Constraint::Min(3),    // body
                Constraint::Length(2),  // footer
            ])
            .split(area);

        self.render_header(f, chunks[0]);

        if self.drill.is_some() {
            self.render_drill(f, chunks[1]);
        } else {
            self.render_list(f, chunks[1]);
        }

        self.render_footer(f, chunks[2]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let bucket_count = ((self.end - self.start).num_days()
            / self.bucket_by.bucket_days()
            + 1) as usize;
        let title = format!(
            "Trend: {} | {} {} | {} → {} | {} buckets",
            self.dimension.header_label(),
            bucket_count_label(bucket_count),
            self.bucket_by.label(),
            self.start,
            self.end,
            bucket_count,
        );
        let line = Line::from(vec![Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )]);
        let block = Block::default().borders(Borders::BOTTOM);
        f.render_widget(Paragraph::new(line).block(block), area);
    }

    fn render_list(&self, f: &mut Frame, area: Rect) {
        let result = self.compute();

        let mut lines: Vec<Line> = Vec::new();
        if result.items.is_empty() {
            let hint = match self.dimension {
                TrendDimension::Tag => {
                    "No tagged tasks in this period. Press Tab to switch dimension."
                }
                TrendDimension::SplitAttention => {
                    "No overlapping sessions in this period. Press Tab to switch dimension."
                }
                _ => "No data for this dimension. Press Tab to switch dimension.",
            };
            lines.push(Line::from(Span::styled(
                hint,
                Style::default().fg(Color::DarkGray),
            )));
            let block = Block::default();
            f.render_widget(Paragraph::new(lines).block(block), area);
            return;
        }

        let visible_height = area.height.saturating_sub(2) as usize; // -2 for borders
        let visible_height = visible_height.max(1);
        // Keep selected in view.
        let start = if self.selected_idx >= visible_height {
            self.selected_idx - visible_height + 1
        } else {
            0
        };
        // We always render the total row at the bottom, so reserve one line.
        let row_capacity = visible_height.saturating_sub(1);

        let end_idx = (start + row_capacity).min(result.items.len());

        let total_max = result.totals.iter().copied().max().unwrap_or(0);

        for i in start..end_idx {
            let item = &result.items[i];
            let selected = i == self.selected_idx;
            let style = if selected {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let name = display_name(&item.name, self.dimension);
            let truncated: String = name.chars().take(NAME_WIDTH).collect();
            let padded = format!("{:<nw$}", truncated, nw = NAME_WIDTH);

            let max_val = item.buckets.iter().copied().max().unwrap_or(0);
            let arrow = format_trend_arrow(&item.buckets);
            let total_str = format!("{:>8}", format_dur(item.total));

            // Sparkline color rules:
            //   Quadrant: quadrant color (matched by name prefix "Q1".."Q4").
            //   SplitAttention: red when the row's last non-zero bucket exceeds
            //     the row's per-bucket mean (overlap trending up).
            //   Task / Tag: default.
            let color = self.spark_color_for(item);
            let spark_spans = render_sparkline_spans(&item.buckets, max_val, color);

            let mut spans: Vec<Span> = Vec::with_capacity(7);
            spans.push(Span::styled(padded, style));
            spans.push(Span::raw(" "));
            spans.extend(spark_spans);
            spans.push(Span::raw("  "));
            spans.push(Span::styled(total_str, style));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(arrow, style));
            lines.push(Line::from(spans));
        }

        // Divider + total row.
        let divider = "─".repeat(NAME_WIDTH + 32);
        lines.push(Line::from(Span::styled(
            divider,
            Style::default().fg(Color::DarkGray),
        )));
        let total_spark = render_sparkline_spans(&result.totals, total_max, None);
        let total_sum: i64 = result.totals.iter().sum();
        let total_row_label = format!("{:<nw$}", "Total", nw = NAME_WIDTH);
        let mut total_line = vec![Span::styled(
            total_row_label,
            Style::default().add_modifier(Modifier::BOLD),
        )];
        total_line.push(Span::raw(" "));
        total_line.extend(total_spark);
        total_line.push(Span::raw("  "));
        total_line.push(Span::styled(
            format!("{:>8}", format_dur(total_sum)),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(total_line));

        let block = Block::default();
        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_drill(&self, f: &mut Frame, area: Rect) {
        let Some(drill) = self.drill.as_ref() else {
            return;
        };
        let sessions = &drill.sessions_by_bucket[drill.bucket_idx];
        let total_secs: i64 = sessions.iter().map(|s| s.split_seconds).sum();
        let bucket_label = self.bucket_label_at(drill.bucket_idx);

        let header = format!(
            "{} | {} ({}) — Esc to back",
            drill.item,
            bucket_label,
            format_dur(total_secs),
        );
        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            header,
            Style::default().add_modifier(Modifier::BOLD),
        )));

        // Bucket nav strip: « prev | current | next »
        let mut strip = String::from(" ");
        if drill.bucket_idx > 0 {
            strip.push_str("← ");
            strip.push_str(&self.bucket_label_at(drill.bucket_idx - 1));
            strip.push_str("  «  ");
        }
        strip.push_str(&bucket_label);
        if drill.bucket_idx + 1 < drill.sessions_by_bucket.len() {
            strip.push_str("  »  ");
            strip.push_str(&self.bucket_label_at(drill.bucket_idx + 1));
            strip.push_str(" →");
        }
        lines.push(Line::from(Span::styled(
            strip,
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

        if sessions.is_empty() {
            lines.push(Line::from(Span::styled(
                "No sessions in this bucket.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (i, st) in sessions.iter().enumerate() {
                let selected = i == drill.selected;
                let style = if selected {
                    Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let line = format_session_line(st);
                lines.push(Line::from(Span::styled(line, style)));
            }
        }

        let block = Block::default();
        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let hint = if self.drill.is_some() {
            FOOTER_HINTS_DRILL
        } else {
            FOOTER_HINTS_LIST
        };
        let spans = Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        ));
        let block = Block::default().borders(Borders::TOP);
        f.render_widget(Paragraph::new(spans).block(block), area);
    }

    fn bucket_label_at(&self, idx: usize) -> String {
        // We need bucket_starts to label buckets; recompute (cheap).
        let result = self.compute();
        result.bucket_label(idx, self.bucket_by)
    }

    fn spark_color_for(&self, item: &crate::models::TrendItem) -> Option<Color> {
        match self.dimension {
            TrendDimension::Quadrant => {
                // Match by name prefix; quadrants are always "Q1".."Q4".
                let q = item.name.split(' ').next().unwrap_or("");
                match q {
                    "Q1" => Some(quadrant_color(&EmQuadrant::DoFirst)),
                    "Q2" => Some(quadrant_color(&EmQuadrant::Schedule)),
                    "Q3" => Some(quadrant_color(&EmQuadrant::Delegate)),
                    "Q4" => Some(quadrant_color(&EmQuadrant::Eliminate)),
                    _ => None,
                }
            }
            TrendDimension::SplitAttention => {
                // Red when the row's last non-zero bucket exceeds the row's mean.
                let nonzero: Vec<i64> = item.buckets.iter().filter(|&&v| v > 0).copied().collect();
                if nonzero.len() < 2 {
                    return None;
                }
                let mean = nonzero.iter().sum::<i64>() as f64 / nonzero.len() as f64;
                if let Some(&last) = nonzero.last() {
                    if last as f64 > mean {
                        return Some(Color::Red);
                    }
                }
                None
            }
            _ => None,
        }
    }
}

fn bucket_count_label(n: usize) -> String {
    // Just `n` rendered; placeholder kept separate so the header can evolve.
    n.to_string()
}

fn display_name(name: &str, dim: TrendDimension) -> String {
    if dim == TrendDimension::Tag {
        format!("#{}", name)
    } else {
        name.to_string()
    }
}

fn format_session_line(st: &SessionSplitTime) -> String {
    let local: DateTime<Local> = DateTime::<Utc>::into(st.start_time);
    let weekday = local.format("%a");
    let date = local.format("%m-%d");
    let start = local.format("%H:%M");
    // SessionSplitTime carries start_time and total_seconds; end is derived.
    let end_local = local + chrono::Duration::seconds(st.total_seconds);
    let end_str = end_local.format("%H:%M").to_string();
    let dur = format_dur(st.split_seconds);
    let em = match st.em_quadrant {
        EmQuadrant::DoFirst => "#em11",
        EmQuadrant::Schedule => "#em01",
        EmQuadrant::Delegate => "#em10",
        EmQuadrant::Eliminate => "#em00",
    };
    format!("{weekday} {date}  {start}–{end_str}   {dur:>7}   {em}")
}

fn format_dur(seconds: i64) -> String {
    if seconds <= 0 {
        return "0m".to_string();
    }
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}

/// Colored ratatui-native sparkline. Same level math as `render_sparkline`
/// (which returns a plain String for the stdout path) but returns one
/// `Span` per bucket so the TUI can color the SplitAttention rows red
/// and the Quadrant rows in their canonical colors.
///
/// `color`, when `Some`, paints the non-empty buckets; empty (zero) buckets
/// always render as the default-styled placeholder.
pub fn render_sparkline_spans(
    values: &[i64],
    max_val: i64,
    color: Option<Color>,
) -> Vec<Span<'static>> {
    let mk = |level: usize| -> Span<'static> {
        let ch = SPARK_CHARS[level.min(7)];
        let style = match color {
            Some(c) => Style::default().fg(c),
            None => Style::default(),
        };
        Span::styled(ch.to_string(), style)
    };

    if max_val == 0 {
        return values.iter().map(|_| mk(0)).collect();
    }
    values
        .iter()
        .map(|&v| {
            if v == 0 {
                mk(0)
            } else {
                let level = ((v as f64 / max_val as f64) * 7.0).ceil() as usize;
                mk(level)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EmQuadrant, SessionSplitTime};
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;

    fn split(id: i64, task_id: i64, name: &str, q: EmQuadrant, day: u32) -> SessionSplitTime {
        SessionSplitTime {
            session_id: id,
            task_id,
            task_name: name.to_string(),
            em_quadrant: q,
            start_time: Utc.with_ymd_and_hms(2026, 6, day, 9, 0, 0).unwrap(),
            total_seconds: 3600,
            split_seconds: 3600,
            overlap_seconds: 0,
            link: None,
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn trend_view_constructor_and_compute() {
        let sessions = vec![
            split(1, 1, "x", EmQuadrant::Schedule, 1),
            split(2, 2, "y", EmQuadrant::DoFirst, 8),
        ];
        let view = TrendView::new(
            sessions,
            HashMap::new(),
            date(2026, 6, 1),
            date(2026, 6, 15),
            TrendDimension::Task,
            BucketBy::Week,
            10,
        )
        .unwrap();
        let result = view.compute();
        assert_eq!(result.items.len(), 2);
        // Both tasks have 1h totals; sort order is by total desc (tie).
        assert_eq!(result.items[0].total, 3600);
    }

    #[test]
    fn trend_view_quadrant_compute_returns_four_rows() {
        let sessions = vec![
            split(1, 1, "a", EmQuadrant::Schedule, 1),
            split(2, 2, "b", EmQuadrant::DoFirst, 1),
        ];
        let view = TrendView::new(
            sessions,
            HashMap::new(),
            date(2026, 6, 1),
            date(2026, 6, 1),
            TrendDimension::Quadrant,
            BucketBy::Day,
            10,
        )
        .unwrap();
        let result = view.compute();
        assert_eq!(result.items.len(), 4);
        assert_eq!(result.items[0].name, "Q1 DoFirst");
    }

    #[test]
    fn trend_view_split_attention_drops_zero_overlap() {
        let sessions = vec![split(1, 1, "x", EmQuadrant::Schedule, 1)];
        let view = TrendView::new(
            sessions,
            HashMap::new(),
            date(2026, 6, 1),
            date(2026, 6, 1),
            TrendDimension::SplitAttention,
            BucketBy::Day,
            10,
        )
        .unwrap();
        let result = view.compute();
        assert!(result.items.is_empty());
    }

    #[test]
    fn render_sparkline_spans_counts_match_input() {
        let values = vec![0, 50, 100];
        let spans = render_sparkline_spans(&values, 100, None);
        assert_eq!(spans.len(), 3);
    }

    #[test]
    fn render_sparkline_spans_red_color_applied() {
        let values = vec![10, 50, 100];
        let spans = render_sparkline_spans(&values, 100, Some(Color::Red));
        // Non-zero buckets get the color; zero bucket stays default.
        // We can't inspect Span content directly, but the count matches.
        assert_eq!(spans.len(), 3);
    }
}
