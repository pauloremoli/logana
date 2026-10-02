use crate::db::LogManager;
use crate::gui::state::NavPage;
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
}

pub struct FileLoaded {
    pub path: PathBuf,
    pub reader: FileReader,
    pub log_manager: LogManager,
}
