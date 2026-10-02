use crate::db::Database;
use crate::gui::column_widths::ColumnWidths;
use crate::gui::time_range::TimeRangePreset;
use crate::theme::Theme as TuiTheme;
use crate::ui::TabState;
use std::collections::HashMap;
use std::sync::Arc;

pub struct GuiState {
    pub db: Arc<Database>,
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub status: Option<StatusMessage>,
    pub theme: TuiTheme,
    pub nav_page: NavPage,
    pub time_range_open: bool,
    pub time_range_preset: TimeRangePreset,
    pub sidebar_tab: SidebarTab,
    /// Per-field expand/collapse state for the sidebar's "Available
    /// Filters" facet categories — missing/absent means collapsed, so a
    /// newly-seen field starts collapsed without needing to be
    /// pre-populated.
    pub facet_expanded: HashMap<String, bool>,
    /// The log table's current column widths — re-fit to content whenever
    /// a file loads (`gui::column_widths::fit_column_widths`), overridden
    /// per-column by dragging a header resize handle.
    pub column_widths: ColumnWidths,
    /// The mouse's x position as of the last `ColumnResizeMoved` dispatch
    /// during an active column-resize drag — `None` outside a drag. Each
    /// move event's width change is the delta from this to the event's
    /// own position, so the handle only needs its *current* position per
    /// event, not its position when the drag started.
    pub column_resize_last_x: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusMessage {
    Error(String),
    /// A non-error notification — e.g. `:path`'s current-file-path
    /// response, which the TUI shows via `tab.set_notification` rather
    /// than its command-error slot.
    Info(String),
}

/// The left nav rail's current page. Only `Logs` has real content today —
/// the others render a plain placeholder (see `gui::view::nav_rail`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavPage {
    #[default]
    Logs,
    Bookmarks,
    Annotations,
    Searches,
    Settings,
}

/// The right sidebar's active tab — Groups replaces the mockup's
/// Annotations tab (no annotations UI exists yet; logana already has real
/// filter-group data to show here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Filters,
    Groups,
}

impl GuiState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            tabs: Vec::new(),
            active_tab: 0,
            status: None,
            theme: TuiTheme::default(),
            nav_page: NavPage::default(),
            time_range_open: false,
            time_range_preset: TimeRangePreset::default(),
            sidebar_tab: SidebarTab::default(),
            facet_expanded: HashMap::new(),
            column_widths: ColumnWidths::default(),
            column_resize_last_x: None,
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
