use crate::db::LogManager;
use crate::gui::column_widths::ResizableColumn;
use crate::gui::state::{NavPage, SidebarTab};
use crate::gui::time_range::TimeRangePreset;
use crate::ingestion::FileReader;
use crate::input::{KeyCode, KeyModifiers};
use std::path::PathBuf;

pub enum Message {
    OpenFileDialog,
    FileDialogResult(Option<PathBuf>),
    FileLoaded(Result<FileLoaded, String>),
    TabSelected(usize),
    TabClosed(usize),
    /// Routed into the active tab's real `Mode::handle_key` — see
    /// `gui::update::dispatch_key`. Command-mode text entry, search
    /// queries, filter/group-management navigation, and every other
    /// mode's input all flow through this one variant now, same as the
    /// TUI's single crossterm key-event entry point.
    KeyPressed(KeyCode, KeyModifiers),
    CommandExecuted(usize, Result<LogManager, String>),

    FilterToggled(usize, usize),
    FiltersMutated(usize, LogManager),

    GroupToggled(usize, String),
    GroupDeleted(usize, String),
    GroupsMutated(usize, LogManager),

    NavPageSelected(NavPage),

    TimeRangeToggled,
    TimeRangeSelected(TimeRangePreset),

    /// Pause/resume the active tab's live tail, toggling based on its
    /// current `tab.stream.paused` state.
    StreamToggled(usize),

    SidebarTabSelected(SidebarTab),
    SidebarToggled,
    ClearAllFilters(usize),
    FilterRemoved(usize, usize),

    FacetToggled(String),
    FacetValueToggled(usize, String, String),

    /// Fired on every `on_drag_move` while dragging a log-table column's
    /// resize handle — `mouse_x` is that event's absolute window x
    /// position, which the reducer diffs against the previous event's
    /// (`GuiState::column_resize_last_x`) to get this step's delta. See
    /// `GuiState::column_resize_last_x`'s doc comment for why a delta
    /// instead of an absolute start/end width.
    ColumnResizeMoved(ResizableColumn, f32),
    /// Fired on `on_mouse_up_out`, ending the drag.
    ColumnResizeEnded,
}

pub struct FileLoaded {
    pub path: PathBuf,
    pub reader: FileReader,
    pub log_manager: LogManager,
    pub watch: crate::ui::FileWatchState,
}
