use crate::commands::Commands;
use crate::db::LogManager;
use std::path::PathBuf;

/// What `update()` wants the gpui glue layer (`app.rs`) to actually do —
/// async work or an imperative widget action — without `update()` itself
/// depending on any gpui type. Mirrors what iced's `Task<Message>` did for
/// the discarded prototype, but as a plain, directly-testable enum.
#[derive(Debug)]
pub enum Effect {
    None,
    Quit,
    FocusCommandBar,
    /// Scroll the active tab's log pane so line `usize` is the topmost
    /// visible row — an absolute index, already clamped by `update()`
    /// against the tab's visible-line count.
    Scroll(usize),
    OpenFileDialog,
    LoadFile(PathBuf),
    ToggleFilter {
        tab_idx: usize,
        log_manager: LogManager,
        id: usize,
    },
    ToggleGroup {
        tab_idx: usize,
        log_manager: LogManager,
        name: String,
    },
    RemoveGroup {
        tab_idx: usize,
        log_manager: LogManager,
        name: String,
    },
    ExecuteCommand {
        tab_idx: usize,
        log_manager: LogManager,
        command: Commands,
    },
}
