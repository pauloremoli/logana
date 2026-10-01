use crate::db::LogManager;
use crate::gui::key::{GuiKey, GuiModifiers};
use crate::ingestion::FileReader;
use std::path::PathBuf;

pub enum Message {
    OpenFileDialog,
    FileDialogResult(Option<PathBuf>),
    FileLoaded(Result<FileLoaded, String>),
    TabSelected(usize),
    TabClosed(usize),
    KeyPressed(GuiKey, GuiModifiers),
    CommandInputChanged(String),
    CommandSubmitted,
    CommandExecuted(usize, Result<LogManager, String>),

    FilterToggled(usize, usize),
    FiltersMutated(usize, LogManager),

    GroupToggled(usize, String),
    GroupDeleted(usize, String),
    GroupsMutated(usize, LogManager),
}

pub struct FileLoaded {
    pub path: PathBuf,
    pub reader: FileReader,
    pub log_manager: LogManager,
}
