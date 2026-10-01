use crate::db::{Database, LogManager};
use crate::filters::{FilterDef, GroupDef};
use crate::ingestion::FileReader;
use crate::theme::Theme as TuiTheme;
use std::path::PathBuf;
use std::sync::Arc;

pub struct GuiState {
    pub db: Arc<Database>,
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub status: Option<StatusMessage>,
    pub mode: InteractionMode,
    /// Tracks a pending leading `g` in Normal mode, for the `gg` chord
    /// (jump to top) — mirrors the TUI's `g_key_pressed` flag.
    pub g_pending: bool,
    /// The TUI's currently-configured theme, applied to the whole GUI so
    /// both frontends look the same. Defaults on construction; use
    /// `with_theme` to load the real one (see `gui::theme::load_current`).
    pub theme: TuiTheme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionMode {
    Normal,
    Command { input: String },
}

pub struct TabState {
    pub file_path: PathBuf,
    pub reader: FileReader,
    pub log_manager: LogManager,
    pub visible_lines: Vec<usize>,
    pub filter_defs: Vec<FilterDef>,
    pub group_defs: Vec<GroupDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusMessage {
    Error(String),
}

impl TabState {
    pub fn new(file_path: PathBuf, reader: FileReader, log_manager: LogManager) -> Self {
        Self {
            file_path,
            reader,
            log_manager,
            visible_lines: Vec::new(),
            filter_defs: Vec::new(),
            group_defs: Vec::new(),
        }
    }

    pub fn title(&self) -> String {
        title_for_path(&self.file_path)
    }
}

/// A tab's label is its file name, not the full path — falls back to the
/// full path for the rare case a `PathBuf` has no file-name component.
fn title_for_path(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl GuiState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            tabs: Vec::new(),
            active_tab: 0,
            status: None,
            mode: InteractionMode::Normal,
            g_pending: false,
            theme: TuiTheme::default(),
        }
    }

    pub fn with_theme(mut self, theme: TuiTheme) -> Self {
        self.theme = theme;
        self
    }

    pub fn active_tab(&self) -> Option<&TabState> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut TabState> {
        self.tabs.get_mut(self.active_tab)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_uses_file_name_not_full_path() {
        assert_eq!(
            title_for_path(std::path::Path::new("/var/log/app/service.log")),
            "service.log"
        );
    }

    #[test]
    fn title_falls_back_to_full_path_with_no_file_name() {
        assert_eq!(title_for_path(std::path::Path::new("/")), "/");
    }

    #[tokio::test]
    async fn active_tab_is_none_when_no_tabs_open() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let state = GuiState::new(db);
        assert!(state.active_tab().is_none());
        assert!(state.tabs.is_empty());
    }

    #[tokio::test]
    async fn with_theme_overrides_the_default_theme() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let theme = TuiTheme {
            root_bg: ratatui::style::Color::Blue,
            ..TuiTheme::default()
        };
        let state = GuiState::new(db).with_theme(theme.clone());
        assert_eq!(state.theme.root_bg, theme.root_bg);
    }
}
