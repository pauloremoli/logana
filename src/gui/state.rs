use crate::db::Database;
use crate::theme::Theme as TuiTheme;
use crate::ui::TabState;
use std::sync::Arc;

pub struct GuiState {
    pub db: Arc<Database>,
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub status: Option<StatusMessage>,
    pub theme: TuiTheme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusMessage {
    Error(String),
}

impl GuiState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            tabs: Vec::new(),
            active_tab: 0,
            status: None,
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
