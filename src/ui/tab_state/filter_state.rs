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
