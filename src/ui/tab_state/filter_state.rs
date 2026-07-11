use std::sync::Arc;

use ratatui::style::Style;

use crate::filters::DateFilterStyle;
use crate::filters::FilterManager;

use super::{CachedScanResult, FilterHandle, VisibleLines};

pub struct FilterState {
    pub visible_indices: VisibleLines,
    pub enabled: bool,
    pub show_marks_only: bool,
    pub filter_context: Option<usize>,
    pub editing_filter_id: Option<usize>,
    /// Scroll offset (in wrapped display rows) the sidebar was last drawn
    /// at, persisted so it only re-scrolls when the selection leaves the
    /// current viewport instead of re-centering on every render.
    pub sidebar_scroll: usize,
    /// Index of the last-highlighted filter in the sidebar, persisted so
    /// leaving and re-entering filter management mode (or just browsing
    /// with the mode bar closed) keeps the same row selected instead of
    /// resetting to the top of the list.
    pub last_selected_filter: usize,
    /// When true, all filters (Include/Exclude/Highlight) act as pure
    /// highlighters: every line stays visible, but filter colors still
    /// render. A temporary "preview" toggle, orthogonal to
    /// `FilterType::Highlight`. Toggled via 'H' in filter management mode.
    pub highlight_mode: bool,
    pub manager: Arc<FilterManager>,
    pub text_styles: Vec<Style>,
    pub date_styles: Vec<DateFilterStyle>,
    pub field_styles: Vec<crate::filters::FieldFilterStyle>,
    pub match_counts: Vec<usize>,
    pub saved_view: Option<FilterViewSnapshot>,
    pub cached_scan: Option<CachedScanResult>,
    pub handle: Option<FilterHandle>,
}

pub type FilterViewSnapshot = (
    VisibleLines,
    Arc<FilterManager>,
    Vec<Style>,
    Vec<DateFilterStyle>,
    Vec<crate::filters::FieldFilterStyle>,
);

impl Default for FilterState {
    fn default() -> Self {
        Self {
            visible_indices: VisibleLines::default(),
            enabled: true,
            show_marks_only: false,
            filter_context: None,
            editing_filter_id: None,
            sidebar_scroll: 0,
            last_selected_filter: 0,
            highlight_mode: false,
            manager: Arc::new(FilterManager::empty()),
            text_styles: Vec::new(),
            date_styles: Vec::new(),
            field_styles: Vec::new(),
            match_counts: Vec::new(),
            saved_view: None,
            cached_scan: None,
            handle: None,
        }
    }
}
