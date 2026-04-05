use super::App;
use super::TabState;
use super::VisibleLines;
use crate::db::LogManager;

impl App {
    pub(crate) fn apply_tab_defaults(&self, tab: &mut TabState) {
        tab.interaction.keybindings = self.keybindings.clone();
        tab.display.show_mode_bar = self.display.show_mode_bar;
        tab.display.show_borders = self.display.show_borders_default;
        tab.display.show_line_numbers = self.display.show_line_numbers;
        tab.display.show_sidebar = self.display.show_sidebar;
        tab.display.wrap = self.display.wrap;
        tab.display.sidebar_side = self.display.sidebar_side;
    }

    pub fn tab(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }

    pub fn tab_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    pub async fn close_tab(&mut self) -> bool {
        use std::sync::atomic::Ordering;
        self.save_tab_context(&self.tabs[self.active_tab]).await;

        let tab = &self.tabs[self.active_tab];
        if let Some(ref h) = tab.search.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(ref h) = tab.filter.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }

        if let Some(ref fls) = self.tabs[self.active_tab].load_state {
            fls.cancel.store(true, Ordering::Relaxed);
        }

        if self.tabs.len() <= 1 {
            return true;
        }
        self.tabs.remove(self.active_tab);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        false
    }

    pub(super) fn handle_open_merge_select(&mut self) {
        use crate::mode::merge_select_mode::MergeSelectMode;
        let (tabs, tab_indices): (Vec<(String, bool)>, Vec<usize>) = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.merged.is_none())
            .map(|(i, t)| ((t.title.clone(), i == self.active_tab), i))
            .unzip();
        if tabs.len() < 2 {
            self.tabs[self.active_tab].interaction.command_error =
                Some("No other tabs to merge".to_string());
            return;
        }
        self.tabs[self.active_tab].interaction.mode =
            Box::new(MergeSelectMode::new(tabs, tab_indices));
    }

    pub(crate) async fn open_merge_tab(&mut self, source_tab_indices: Vec<usize>) {
        use crate::ui::tab_state::merged::{MergedState, build_merged_index};

        let sources: Vec<crate::ingestion::FileReader> = source_tab_indices
            .iter()
            .map(|&i| self.tabs[i].file_reader.clone())
            .collect();
        let parsers: Vec<Option<std::sync::Arc<dyn crate::parser::LogFormatParser>>> =
            source_tab_indices
                .iter()
                .map(|&i| self.tabs[i].display.format.clone())
                .collect();
        let year_maps: Vec<Option<std::sync::Arc<crate::ui::tab_state::year_map::YearMap>>> =
            source_tab_indices
                .iter()
                .map(|&i| self.tabs[i].year_map.clone())
                .collect();
        let continuation_maps: Vec<Option<std::sync::Arc<Vec<usize>>>> = source_tab_indices
            .iter()
            .map(|&i| self.tabs[i].continuation_map.clone())
            .collect();
        let source_labels: Vec<String> = source_tab_indices
            .iter()
            .map(|&i| self.tabs[i].title.clone())
            .collect();

        let entries = build_merged_index(&sources, &parsers, &year_maps, &continuation_maps);
        let source_line_counts: Vec<usize> = sources.iter().map(|s| s.line_count()).collect();
        let label_col_width = source_labels.iter().map(|l| l.len()).max().unwrap_or(0);

        let visible_count = entries.len();
        let entries_arc = std::sync::Arc::new(entries);
        let sources_arc = std::sync::Arc::new(sources);
        let file_reader = crate::ingestion::FileReader::from_merged(entries_arc, sources_arc);

        let title = format!("merged({})", source_tab_indices.len());
        let log_manager = LogManager::new(self.db.clone(), None).await;
        let mut tab = TabState::new(file_reader, log_manager, title);
        tab.display.format = None;
        tab.continuation_map = None;
        tab.filter.visible_indices = VisibleLines::Filtered((0..visible_count).collect());
        tab.display.show_line_numbers = false;
        tab.merged = Some(MergedState {
            source_tab_indices,
            source_parsers: parsers,
            source_labels,
            source_line_counts,
            label_col_width,
            stopped: false,
        });
        self.apply_tab_defaults(&mut tab);

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }
}
