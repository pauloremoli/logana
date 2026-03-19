use std::collections::HashMap;

use ratatui::text::Line;

use super::CachedParsedLine;

#[derive(Default)]
pub struct CacheState {
    pub parse_gen: u64,
    pub parse: HashMap<usize, (u64, CachedParsedLine)>,
    pub render_gen: u64,
    pub render_line: HashMap<usize, (u64, u64, Option<usize>, Line<'static>)>,
    pub search_result_gen: u64,
    pub field_names: Option<(u64, Vec<String>)>,
}
