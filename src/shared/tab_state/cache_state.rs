use std::collections::HashMap;

#[cfg(feature = "tui")]
use ratatui::text::Line;

use super::CachedParsedLine;

#[cfg(feature = "gui")]
pub type GuiFilterStyleCache =
    HashMap<usize, ((u64, usize, usize), Vec<crate::gui::color::LineStyle>)>;

#[derive(Default)]
pub struct CacheState {
    pub parse_gen: u64,
    pub parse: HashMap<usize, (u64, CachedParsedLine)>,
    pub render_gen: u64,
    #[cfg(feature = "gui")]
    pub gui_filter_styles: std::sync::Mutex<GuiFilterStyleCache>,
    #[cfg(feature = "tui")]
    pub render_line: HashMap<usize, (u64, u64, Option<usize>, Line<'static>)>,
    pub search_result_gen: u64,
    pub field_names: Option<(u64, Vec<String>)>,
}
