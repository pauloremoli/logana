use crate::db::LogManager;
use crate::gui::column_widths::ResizableColumn;
use crate::gui::state::{NavPage, SidebarTab};
use crate::gui::time_range::TimeRangePreset;
use crate::ingestion::FileReader;
use crate::input::{KeyCode, KeyModifiers};
use std::path::PathBuf;

pub enum Message {
    DockerListed(u64, Result<Vec<(String, String)>, String>),
    SourceLoaded(Result<Box<crate::shared::TabState>, String>),
    DismissNotification(u64),
    GuiFinished(u64, Result<String, String>),
    GuiAction(crate::gui::actions::Action),
    EditFilter(usize),
    EditAnnotation(usize),
    EditGroup(String),
    EditorText(usize, String),
    EditorToggle(usize),
    EditorKind(crate::filters::FilterType),
    EditorField(usize),
    EditorMember(usize),
    EditorMove(usize, isize),
    EditorAll(bool),
    EditorReset,
    EditorConditionAdd,
    EditorConditionRemove(usize),
    EditorPreview(String),
    EditorApply,
    EditorNavigate(crate::gui::actions::Action),
    EditorCancel,
    GuiCommand(crate::commands::Commands, String),
    SearchNext(bool),
    SelectLog(usize, bool),
    OpenCommands,
    PaletteEdited(String),
    PaletteKey(KeyCode, KeyModifiers),
    PaletteInsert(usize),
    PaletteSubmit,
    PaletteClose,
    PaletteClear,
    PaletteExecuted(u64, crate::shared::TabId, Result<LogManager, String>),
    OpenFileDialog,
    FileDialogResult(Option<PathBuf>),
    FileLoaded(Result<FileLoaded, String>),
    TabSelected(usize),
    TabClosed(usize),
    /// Legacy key messages are ignored; native controls own GUI input.
    KeyPressed(KeyCode, KeyModifiers),
    /// Native input edits reuse the same incremental search engine.
    SearchEdited(String),
    CommandExecuted(usize, Result<LogManager, String>),

    FilterToggled(usize, usize),
    FilterReordered(usize, usize, usize),
    FilterPersistenceFailed(String),
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
    pub watch: crate::shared::FileWatchState,
}
