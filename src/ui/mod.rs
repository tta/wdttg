pub mod edit_form;
pub mod em_selector;
pub mod history_view;
pub mod picker;
pub mod session_edit_form;
pub mod tag_picker;
pub mod timeline;

pub use edit_form::{edit_task, EditFormResult, EditableTask};
pub use em_selector::{select_em_quadrant, EmSelectorResult};
pub use history_view::{print_history_static, HistoryView, HistoryViewError, HistoryViewResult};
pub use picker::{filter_tasks, PickerResult, TaskPicker};
pub use session_edit_form::{edit_session, EditableSession, SessionEditError, SessionEditResult};
pub use tag_picker::{print_tags_static, TagPicker, TagPickerError, TagPickerResult};
pub use timeline::{
    print_timeline_static, quadrant_color, TimelineError, TimelineResult, TimelineView, ZoomLevel,
};

pub use history_view::Period as HistoryViewPeriod;
pub use tag_picker::Period as TagPickerPeriod;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

pub fn suspend_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    Ok(())
}

pub fn resume_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    terminal.clear()?;
    Ok(())
}
