use crate::models::EmQuadrant;
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
pub enum EmSelectorError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Terminal error: {0}")]
    Terminal(String),
}

pub type Result<T> = std::result::Result<T, EmSelectorError>;

#[derive(Debug, Clone, PartialEq)]
pub enum EmSelectorResult {
    Selected(EmQuadrant),
    Cancelled,
}

const QUADRANTS: [(EmQuadrant, &str, &str); 4] = [
    (EmQuadrant::DoFirst, "11", "Urgent + Important (Do First)"),
    (
        EmQuadrant::Schedule,
        "01",
        "Not Urgent + Important (Schedule)",
    ),
    (
        EmQuadrant::Delegate,
        "10",
        "Urgent + Not Important (Delegate)",
    ),
    (
        EmQuadrant::Eliminate,
        "00",
        "Not Urgent + Not Important (Eliminate)",
    ),
];

pub struct EmSelector {
    list_state: ListState,
}

impl Default for EmSelector {
    fn default() -> Self {
        Self::new()
    }
}

impl EmSelector {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self { list_state }
    }

    pub fn run(self) -> Result<EmSelectorResult> {
        enable_raw_mode().map_err(|e| EmSelectorError::Terminal(e.to_string()))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| EmSelectorError::Terminal(e.to_string()))?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal =
            Terminal::new(backend).map_err(|e| EmSelectorError::Terminal(e.to_string()))?;

        let result = self.run_loop(&mut terminal);

        disable_raw_mode().map_err(|e| EmSelectorError::Terminal(e.to_string()))?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| EmSelectorError::Terminal(e.to_string()))?;

        result
    }

    pub fn run_on_terminal(
        self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<EmSelectorResult> {
        self.run_loop(terminal)
    }

    fn run_loop(
        mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<EmSelectorResult> {
        loop {
            terminal
                .draw(|f| self.render(f))
                .map_err(|e| EmSelectorError::Terminal(e.to_string()))?;

            if let Event::Key(key) =
                event::read().map_err(|e| EmSelectorError::Terminal(e.to_string()))?
            {
                match (key.modifiers, key.code) {
                    (_, KeyCode::Esc) => {
                        return Ok(EmSelectorResult::Cancelled);
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('j'))
                    | (KeyModifiers::NONE, KeyCode::Char('j')) => {
                        self.move_selection(1);
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('k'))
                    | (KeyModifiers::NONE, KeyCode::Char('k')) => {
                        self.move_selection(-1);
                    }
                    (_, KeyCode::Enter) => {
                        return Ok(self.select_current());
                    }
                    (_, KeyCode::Up) => {
                        self.move_selection(-1);
                    }
                    (_, KeyCode::Down) => {
                        self.move_selection(1);
                    }
                    (_, KeyCode::Char(c)) => {
                        if let Some(result) = self.handle_char(c, key.modifiers) {
                            return Ok(result);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle_char(&mut self, c: char, modifiers: KeyModifiers) -> Option<EmSelectorResult> {
        if modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match c {
            '1' => Some(EmSelectorResult::Selected(EmQuadrant::DoFirst)),
            '2' => Some(EmSelectorResult::Selected(EmQuadrant::Schedule)),
            '3' => Some(EmSelectorResult::Selected(EmQuadrant::Delegate)),
            '4' => Some(EmSelectorResult::Selected(EmQuadrant::Eliminate)),
            _ => None,
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let current = self.list_state.selected().unwrap_or(0);
        let max = (QUADRANTS.len() - 1) as i32;
        let new_selection = ((current as i32 + delta).clamp(0, max)) as usize;
        self.list_state.select(Some(new_selection));
    }

    fn select_current(self) -> EmSelectorResult {
        let idx = self.list_state.selected().unwrap_or(0);
        if idx < QUADRANTS.len() {
            EmSelectorResult::Selected(QUADRANTS[idx].0)
        } else {
            EmSelectorResult::Cancelled
        }
    }

    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(f.size());

        self.render_title(f, chunks[0]);
        self.render_list(f, chunks[1]);
        self.render_footer(f, chunks[2]);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(Line::from(vec![
            Span::styled("[1-4]", Style::default().fg(Color::Yellow)),
            Span::raw(" Select  "),
            Span::styled("[j/k/↑/↓]", Style::default().fg(Color::Yellow)),
            Span::raw(" Navigate  "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Confirm  "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Cancel"),
        ]))
        .block(Block::default().borders(Borders::ALL));

        f.render_widget(paragraph, area);
    }

    fn render_title(&self, f: &mut Frame, area: Rect) {
        let title = Paragraph::new(Line::from(vec![Span::styled(
            "EM quadrant required. Select:",
            Style::default().fg(Color::Yellow),
        )]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Task Classification ")
                .title_style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(title, area);
    }

    fn render_list(&self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = QUADRANTS
            .iter()
            .enumerate()
            .map(|(i, (_, code, desc))| {
                let num = (i + 1).to_string();
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{}. ", num),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("#em{} ", code),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("- {}", desc), Style::default().fg(Color::White)),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Options ")
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
}

pub fn select_em_quadrant() -> Result<EmSelectorResult> {
    EmSelector::new().run()
}

pub fn select_em_quadrant_on_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<EmSelectorResult> {
    EmSelector::new().run_on_terminal(terminal)
}

pub fn select_em_quadrant_or_default() -> Option<EmQuadrant> {
    match select_em_quadrant() {
        Ok(EmSelectorResult::Selected(quadrant)) => Some(quadrant),
        _ => None,
    }
}

pub fn select_em_quadrant_on_terminal_or_default(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Option<EmQuadrant> {
    match select_em_quadrant_on_terminal(terminal) {
        Ok(EmSelectorResult::Selected(quadrant)) => Some(quadrant),
        _ => None,
    }
}
