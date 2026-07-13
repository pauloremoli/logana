use super::App;
use super::TabState;
use super::VisibleLines;
use crate::db::LogManager;
use crate::ingestion::{FileReader, MergeMarkedSource};
use crate::parser::LogFormatParser;
use crate::ui::tab_state::year_map::YearMap;
use std::sync::Arc;

/// Everything [`build_merged_tab`] needs to build a merged tab's index,
/// regardless of whether each source came from an already-open tab (see
/// [`App::merge_inputs_from_tabs`], used by `:merge`) or from a
/// freshly-extracted-and-detected archive file (see
/// [`App::merge_inputs_from_extracted`]).
struct MergeSourceInputs {
    sources: Vec<FileReader>,
    parsers: Vec<Option<Arc<dyn LogFormatParser>>>,
    year_maps: Vec<Option<Arc<YearMap>>>,
    continuation_maps: Vec<Option<Arc<Vec<usize>>>>,
    labels: Vec<String>,
}

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

    /// Gathers `MergeSourceInputs` from already-open tabs — the source
    /// shape `:merge` has always used.
    fn merge_inputs_from_tabs(&self, source_tab_indices: &[usize]) -> MergeSourceInputs {
        MergeSourceInputs {
            sources: source_tab_indices
                .iter()
                .map(|&i| self.tabs[i].file_reader.clone())
                .collect(),
            parsers: source_tab_indices
                .iter()
                .map(|&i| self.tabs[i].display.format.clone())
                .collect(),
            year_maps: source_tab_indices
                .iter()
                .map(|&i| self.tabs[i].year_map.clone())
                .collect(),
            continuation_maps: source_tab_indices
                .iter()
                .map(|&i| self.tabs[i].continuation_map.clone())
                .collect(),
            labels: source_tab_indices
                .iter()
                .map(|&i| self.tabs[i].title.clone())
                .collect(),
        }
    }

    /// Gathers `MergeSourceInputs` from freshly-extracted-and-detected
    /// archive files — no tab lookups, and (deliberately) no throwaway
    /// `TabState`/`LogManager`/DB row per source, since only the final
    /// merged tab needs one of those.
    fn merge_inputs_from_extracted(sources: Vec<MergeMarkedSource>) -> MergeSourceInputs {
        let mut inputs = MergeSourceInputs {
            sources: Vec::with_capacity(sources.len()),
            parsers: Vec::with_capacity(sources.len()),
            year_maps: Vec::with_capacity(sources.len()),
            continuation_maps: Vec::with_capacity(sources.len()),
            labels: Vec::with_capacity(sources.len()),
        };
        for s in sources {
            inputs.labels.push(s.label);
            inputs.parsers.push(s.detected.format);
            inputs.year_maps.push(s.detected.year_map);
            inputs.continuation_maps.push(s.detected.continuation_map);
            inputs.sources.push(s.reader);
        }
        inputs
    }

    /// Builds and pushes one merged tab from `inputs`, sorted by timestamp
    /// across every source. `source_tab_indices` drives live-update
    /// polling (`App::advance_merged_tabs`) — pass an empty `Vec` for
    /// extraction-sourced merges, which have no open tab to poll for
    /// growth (an empty `source_tab_indices` is already a verified no-op
    /// there).
    async fn build_merged_tab(
        &mut self,
        inputs: MergeSourceInputs,
        title: String,
        source_tab_indices: Vec<usize>,
    ) {
        use crate::ui::tab_state::merged::{MergedState, build_merged_index};

        let entries = build_merged_index(
            &inputs.sources,
            &inputs.parsers,
            &inputs.year_maps,
            &inputs.continuation_maps,
        );
        let source_line_counts: Vec<usize> =
            inputs.sources.iter().map(|s| s.line_count()).collect();
        let label_col_width = inputs.labels.iter().map(|l| l.len()).max().unwrap_or(0);

        let visible_count = entries.len();
        let entries_arc = Arc::new(entries);
        let sources_arc = Arc::new(inputs.sources);
        let file_reader = FileReader::from_merged(entries_arc, sources_arc);

        let log_manager = LogManager::new(self.db.clone(), None).await;
        let mut tab = TabState::new(file_reader, log_manager, title);
        tab.display.format = None;
        tab.continuation_map = None;
        tab.filter.visible_indices = VisibleLines::Filtered((0..visible_count).collect());
        tab.display.show_line_numbers = false;
        tab.merged = Some(MergedState {
            source_tab_indices,
            source_parsers: inputs.parsers,
            source_labels: inputs.labels,
            source_line_counts,
            label_col_width,
            stopped: false,
        });
        self.apply_tab_defaults(&mut tab);

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    pub(crate) async fn open_merge_tab(&mut self, source_tab_indices: Vec<usize>) {
        let inputs = self.merge_inputs_from_tabs(&source_tab_indices);
        let title = format!("merged({})", source_tab_indices.len());
        self.build_merged_tab(inputs, title, source_tab_indices)
            .await;
    }

    /// Builds one merged tab from files extracted from an archive, marked
    /// with 'm' in the archive picker — the individual sources never get
    /// their own separate tab, only the merged result does.
    pub(crate) async fn open_merged_tab_from_extraction(
        &mut self,
        sources: Vec<MergeMarkedSource>,
    ) {
        let title = format!("merged({})", sources.len());
        let inputs = Self::merge_inputs_from_extracted(sources);
        self.build_merged_tab(inputs, title, Vec::new()).await;
    }
}
