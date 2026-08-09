use std::collections::HashSet;
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
    /// Persisted sidebar viewport row count, mirrors `ScrollState::visible_height`.
    /// Set during `render_sidebar`; used by filter-mode half/full-page motions.
    pub sidebar_visible_height: usize,
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
    /// Parent line indices whose continuation-line visibility has been
    /// individually flipped away from `DisplayConfig::collapse_continuations`
    /// (the default) via the normal-mode `<` and `>` keys. A parent's
    /// *effective* collapsed state is `collapse_continuations XOR
    /// overridden_groups.contains(parent)`, so `<`/`>` behave the same
    /// whether the global default is expanded or collapsed.
    pub overridden_groups: HashSet<usize>,
    /// Snapshot of `visible_indices` from before the collapse mask was
    /// applied. `None` when nothing is currently collapsed (the common
    /// case), letting filter refreshes skip collapse work entirely. Set
    /// lazily by `<`/`>` even when `collapse_continuations` is off, so a
    /// single overridden group still has a baseline to mask against.
    pub pre_collapse_visible: Option<VisibleLines>,
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
            sidebar_visible_height: 0,
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
            overridden_groups: HashSet::new(),
            pre_collapse_visible: None,
        }
    }
}
