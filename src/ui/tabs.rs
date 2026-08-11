use super::App;
use super::TabState;
use super::VisibleLines;
use crate::db::LogManager;
use crate::ingestion::{FileReader, MergeMarkedSource};
use crate::parser::LogFormatParser;
use crate::ui::tab_state::year_map::YearMap;
use std::sync::Arc;

/// Everything [`App::build_merged_tab`] needs to build a merged tab's index,
/// regardless of whether each source came from an already-open tab (see
/// [`App::merge_inputs_from_tabs`], used by `:merge`) or from a
/// freshly-extracted-and-detected archive or directory file (see
/// [`App::merge_inputs_from_extracted`]). `pub(crate)` (and likewise its
/// fields) so a picker-triggered merge's Phase 1 — reading/extracting each
/// source, which lives in `crate::ingestion::loading` (archive) and
/// `crate::ui::input` (directory) — can hand its result straight to
/// [`App::start_merge_build_streaming`].
pub(crate) struct MergeSourceInputs {
    pub(crate) sources: Vec<FileReader>,
    pub(crate) parsers: Vec<Option<Arc<dyn LogFormatParser>>>,
    pub(crate) year_maps: Vec<Option<Arc<YearMap>>>,
    pub(crate) continuation_maps: Vec<Option<Arc<Vec<usize>>>>,
    pub(crate) labels: Vec<String>,
    /// Owned temp copies backing `sources`, kept alive on the resulting tab
    /// (see `TabState::merge_source_temps`). Empty for `:merge`'s
    /// already-open-tab sources, which have no temp copy of their own.
    pub(crate) temp_files: Vec<tempfile::NamedTempFile>,
}

impl App {
    pub(crate) async fn apply_tab_defaults(&self, tab: &mut TabState) {
        tab.interaction.keybindings = self.keybindings.clone();
        tab.display.show_mode_bar = self.display.show_mode_bar;
        tab.display.show_borders = self.display.show_borders_default;
        tab.display.show_line_numbers = self.display.show_line_numbers;
        tab.display.show_sidebar = self.display.show_sidebar;
        tab.display.wrap = self.display.wrap;
        tab.display.sidebar_side = self.display.sidebar_side;
        tab.display.collapse_continuations = self.display.collapse_continuations;
        tab.display.show_groups_panel = self.display.show_groups_panel;
        tab.sync_collapse_mask();
        self.apply_default_filters_if_empty(tab).await;
    }

    /// Whether `tab`'s current format has zero filters and a configured
    /// default path — returns that path if so. Pure/sync: no I/O, so it's
    /// trivially testable without spinning up a real filter file.
    fn pending_default_filter_path(&self, tab: &TabState) -> Option<String> {
        let format_name = tab.display.format.as_deref().map(|f| f.name())?;
        if !tab.log_manager.get_filters().is_empty() {
            return None;
        }
        self.default_filter_files.get(format_name).cloned()
    }

    /// Auto-loads `tab`'s format's default filter file when eligible (see
    /// `pending_default_filter_path`). A load failure is surfaced as a tab
    /// notification rather than swallowed, so a stale configured path
    /// doesn't fail silently.
    pub(crate) async fn apply_default_filters_if_empty(&self, tab: &mut TabState) {
        let Some(path) = self.pending_default_filter_path(tab) else {
            return;
        };
        let expanded = crate::commands::auto_complete::expand_tilde(&path);
        if let Err(e) = tab.log_manager.load_filters(&expanded).await {
            tab.set_notification(format!("default filter file '{path}': {e}"));
        }
    }

    /// Index-taking wrapper around [`Self::apply_default_filters_if_empty`]
    /// for call sites that hold `self.tabs[idx]` rather than a bare
    /// `&mut TabState` (needed since `App` can't be borrowed both
    /// immutably-for-lookup and mutably-for-the-tab at once across a method
    /// call boundary).
    pub(crate) async fn apply_default_filters_if_empty_at(&mut self, tab_idx: usize) {
        let Some(tab) = self.tabs.get(tab_idx) else {
            return;
        };
        let Some(path) = self.pending_default_filter_path(tab) else {
            return;
        };
        let expanded = crate::commands::auto_complete::expand_tilde(&path);
        if let Err(e) = self.tabs[tab_idx].log_manager.load_filters(&expanded).await {
            self.tabs[tab_idx].set_notification(format!("default filter file '{path}': {e}"));
        }
    }

    /// [`Self::pending_default_filter_path`]'s counterpart for a merged tab:
    /// a merged tab deliberately never sets `display.format` (see
    /// `open_merge_tab`'s doc comment — it combines possibly-heterogeneous
    /// sources, so there's no single format to inherit), so the
    /// `tab.display.format`-keyed lookup `pending_default_filter_path` does
    /// can never find anything for one. This instead derives the same
    /// "format name" from `parsers` — the per-source parsers a merge's
    /// sources were detected with — only when every source agrees on one
    /// recognized format; a merge of genuinely mixed formats has no single
    /// default filter file that would make sense to apply.
    fn pending_default_filter_path_for_merge(
        &self,
        tab: &TabState,
        parsers: &[Option<Arc<dyn LogFormatParser>>],
    ) -> Option<String> {
        if !tab.log_manager.get_filters().is_empty() {
            return None;
        }
        let mut names = parsers.iter().map(|p| p.as_deref().map(|p| p.name()));
        let format_name = names.next().flatten()?;
        if !names.all(|n| n == Some(format_name)) {
            return None;
        }
        self.default_filter_files.get(format_name).cloned()
    }

    /// Auto-loads a merged tab's default filter file when eligible (see
    /// [`Self::pending_default_filter_path_for_merge`]) — the merge
    /// counterpart of [`Self::apply_default_filters_if_empty`], for
    /// [`Self::build_merged_tab`] to call on its local `tab` before pushing
    /// it, once `inputs.parsers` (the merge's sources' detected formats) is
    /// known.
    async fn apply_default_filters_if_empty_for_merge(
        &self,
        tab: &mut TabState,
        parsers: &[Option<Arc<dyn LogFormatParser>>],
    ) {
        let Some(path) = self.pending_default_filter_path_for_merge(tab, parsers) else {
            return;
        };
        let expanded = crate::commands::auto_complete::expand_tilde(&path);
        if let Err(e) = tab.log_manager.load_filters(&expanded).await {
            tab.set_notification(format!("default filter file '{path}': {e}"));
        }
    }

    /// Index-taking wrapper around
    /// [`Self::apply_default_filters_if_empty_for_merge`] for
    /// [`Self::start_merge_build_streaming`], whose tab already lives in
    /// `self.tabs` by the time its sources' parsers become known.
    async fn apply_default_filters_if_empty_for_merge_at(
        &mut self,
        tab_idx: usize,
        parsers: &[Option<Arc<dyn LogFormatParser>>],
    ) {
        let Some(tab) = self.tabs.get(tab_idx) else {
            return;
        };
        let Some(path) = self.pending_default_filter_path_for_merge(tab, parsers) else {
            return;
        };
        let expanded = crate::commands::auto_complete::expand_tilde(&path);
        if let Err(e) = self.tabs[tab_idx].log_manager.load_filters(&expanded).await {
            self.tabs[tab_idx].set_notification(format!("default filter file '{path}': {e}"));
        }
    }

    pub fn tab(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }

    pub fn tab_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    /// Removes the tab at `idx` and fixes up every in-flight background
    /// operation that tracks a tab index by position
    /// (`pending_merge_builds`, `pending_archive`'s merge tab,
    /// `pending_directory_merge`, and every remaining tab's own
    /// `load_state.on_complete`) so they keep pointing at the right tab.
    /// Removing any tab shifts every later index down by one; without this,
    /// a background merge build or file load racing with an unrelated tab
    /// close/removal (a placeholder cleanup, `:close-tab`, another merge
    /// finishing) would silently start writing its results into the wrong
    /// tab.
    ///
    /// This is the only place that should call `self.tabs.remove` —
    /// removing a tab any other way risks exactly that desync.
    pub(crate) fn remove_tab_at(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.active_tab > idx {
            self.active_tab -= 1;
        }
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));

        for state in &mut self.pending_merge_builds {
            if state.tab_idx > idx {
                state.tab_idx -= 1;
            }
        }
        if let Some(state) = self.pending_archive.as_mut()
            && let Some(merge_idx) = state.merge_tab_idx.as_mut()
            && *merge_idx > idx
        {
            *merge_idx -= 1;
        }
        if let Some(state) = self.pending_directory_merge.as_mut()
            && state.tab_idx > idx
        {
            state.tab_idx -= 1;
        }
        // Every remaining tab's own in-progress background load (if any)
        // embeds the tab_idx it was started with, so it can find its way
        // back to the right tab once loading finishes — that index must
        // shift the same way every other pending-state tracker above does,
        // or a load started before this removal lands on whatever tab now
        // occupies its old position instead (see `handle_open_files`,
        // whose `remove_empty_placeholder` runs after opening files while
        // their loads are still in flight).
        for tab in &mut self.tabs {
            let Some(ls) = tab.load_state.as_mut() else {
                continue;
            };
            match &mut ls.on_complete {
                crate::ui::LoadContext::ReplaceTab { tab_idx } if *tab_idx > idx => {
                    *tab_idx -= 1;
                }
                crate::ui::LoadContext::SessionRestoreTab {
                    tab_idx,
                    initial_tab_idx,
                    ..
                } => {
                    if *tab_idx > idx {
                        *tab_idx -= 1;
                    }
                    if *initial_tab_idx > idx {
                        *initial_tab_idx -= 1;
                    }
                }
                _ => {}
            }
        }
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
        self.remove_tab_at(self.active_tab);
        false
    }

    pub(super) fn handle_open_merge_select(&mut self) {
        use crate::mode::merge_select_mode::MergeSelectMode;
        let (tabs, tab_indices): (Vec<(String, bool)>, Vec<usize>) = self
            .tabs
            .iter()
            .enumerate()
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

    /// Opens the `Ctrl+P` quick-open popup, snapshotting every open tab's
    /// title against its `self.tabs` index.
    pub(super) fn handle_open_file_switcher(&mut self) {
        use crate::mode::file_switcher_mode::FileSwitcherMode;
        let entries: Vec<(usize, String)> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| (i, t.title.clone()))
            .collect();
        self.tabs[self.active_tab].interaction.mode =
            Box::new(FileSwitcherMode::new(entries, self.active_tab));
    }

    /// A source tab's parser for a nested `merge_inputs_from_tabs` build. A
    /// plain tab uses its own detected `display.format`. A merged tab
    /// always has `display.format == None` (it may combine heterogeneous
    /// sources — see `pending_default_filter_path_for_merge`'s doc comment),
    /// so using that directly would silently contribute zero lines to the
    /// new merge (`append_source_entries` skips every line when `parser` is
    /// `None`). Falling back to its `MergedState::uniform_parser` instead
    /// lets a previously-merged tab (e.g. from the archive picker) be
    /// merged again when its own sources agreed on one format.
    fn merge_source_parser(&self, tab_idx: usize) -> Option<Arc<dyn LogFormatParser>> {
        let tab = &self.tabs[tab_idx];
        match &tab.merged {
            Some(merged) => merged.uniform_parser(),
            None => tab.display.format.clone(),
        }
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
                .map(|&i| self.merge_source_parser(i))
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
            temp_files: Vec::new(),
        }
    }

    /// Gathers `MergeSourceInputs` from freshly-extracted-and-detected
    /// archive or directory files — no tab lookups, and (deliberately) no
    /// throwaway `TabState`/`LogManager`/DB row per source, since only the
    /// final merged tab needs one of those.
    pub(crate) fn merge_inputs_from_extracted(
        sources: Vec<MergeMarkedSource>,
    ) -> MergeSourceInputs {
        let mut inputs = MergeSourceInputs {
            sources: Vec::with_capacity(sources.len()),
            parsers: Vec::with_capacity(sources.len()),
            year_maps: Vec::with_capacity(sources.len()),
            continuation_maps: Vec::with_capacity(sources.len()),
            labels: Vec::with_capacity(sources.len()),
            temp_files: Vec::with_capacity(sources.len()),
        };
        for s in sources {
            inputs.labels.push(s.label);
            inputs.parsers.push(s.detected.format);
            inputs.year_maps.push(s.detected.year_map);
            inputs.continuation_maps.push(s.detected.continuation_map);
            inputs.sources.push(s.reader);
            inputs.temp_files.push(s.temp_file);
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
        self.apply_tab_defaults(&mut tab).await;
        self.apply_default_filters_if_empty_for_merge(&mut tab, &inputs.parsers)
            .await;
        tab.merged = Some(MergedState {
            source_tab_indices,
            source_parsers: inputs.parsers,
            source_labels: inputs.labels,
            source_line_counts,
            label_col_width,
            stopped: false,
            building: None,
        });

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    pub(crate) async fn open_merge_tab(&mut self, source_tab_indices: Vec<usize>) {
        let inputs = self.merge_inputs_from_tabs(&source_tab_indices);
        let title = format!("merged({})", source_tab_indices.len());
        self.build_merged_tab(inputs, title, source_tab_indices)
            .await;
    }

    /// Creates an empty, "pending" merged tab immediately and makes it
    /// active — before either phase of a picker-triggered merge (archive
    /// entry extraction / directory file copy, or the index build once
    /// sources are ready) has produced anything. For a merge of big files,
    /// reading/extracting the sources is the slow part; without this, the
    /// destination tab wouldn't appear at all until that finished, leaving
    /// the user with no visible sign the merge is happening. `labels` (the
    /// eventual source filenames) are known upfront from the archive
    /// tree/directory listing alone, with no reading needed, so the tab can
    /// show real source names immediately.
    ///
    /// The caller is responsible for reporting Phase 1 progress on this
    /// tab (e.g. via `TabState::set_notification`) and, once sources are
    /// ready, calling [`Self::start_merge_build_streaming`] to fill it in —
    /// or removing the tab if Phase 1 fails before ever producing sources.
    pub(crate) async fn create_pending_merged_tab(&mut self, labels: Vec<String>) -> usize {
        use crate::ui::tab_state::merged::MergedState;

        let sources_total = labels.len();
        let title = format!("merged({sources_total})");
        let label_col_width = labels.iter().map(|l| l.len()).max().unwrap_or(0);
        let file_reader = FileReader::from_merged(Arc::new(Vec::new()), Arc::new(Vec::new()));
        let log_manager = LogManager::new(self.db.clone(), None).await;
        let mut tab = TabState::new(file_reader, log_manager, title);
        tab.display.format = None;
        tab.continuation_map = None;
        tab.filter.visible_indices = VisibleLines::Filtered(Vec::new());
        tab.display.show_line_numbers = false;
        tab.merged = Some(MergedState {
            source_tab_indices: Vec::new(),
            source_parsers: Vec::new(),
            source_labels: labels,
            source_line_counts: Vec::new(),
            label_col_width,
            stopped: false,
            building: Some((0, sources_total)),
        });
        self.apply_tab_defaults(&mut tab).await;
        self.tabs.push(tab);
        let tab_idx = self.tabs.len() - 1;
        self.active_tab = tab_idx;
        tab_idx
    }

    /// Fills in the tab created by [`Self::create_pending_merged_tab`] once
    /// its sources are ready: spawns a background thread that folds sources
    /// in one at a time and reports each intermediate result, applied to
    /// the live tab by [`Self::poll_merge_builds`] — same "renders
    /// progressively instead of freezing" reasoning as `create_pending_merged_tab`,
    /// just for the index-build phase instead of the read/extract phase.
    /// A no-op if `tab_idx` no longer exists (its tab was closed while
    /// Phase 1 was still running).
    pub(crate) async fn start_merge_build_streaming(
        &mut self,
        tab_idx: usize,
        inputs: MergeSourceInputs,
    ) {
        use crate::ui::tab_state::merged::build_merged_index_streaming;

        if tab_idx >= self.tabs.len() {
            return;
        }

        self.apply_default_filters_if_empty_for_merge_at(tab_idx, &inputs.parsers)
            .await;

        let source_line_counts: Vec<usize> =
            inputs.sources.iter().map(|s| s.line_count()).collect();
        let label_col_width = inputs.labels.iter().map(|l| l.len()).max().unwrap_or(0);
        let sources_total = inputs.sources.len();
        let sources_arc = Arc::new(inputs.sources.clone());

        self.tabs[tab_idx].file_reader =
            FileReader::from_merged(Arc::new(Vec::new()), sources_arc.clone());
        self.tabs[tab_idx].merge_source_temps = inputs.temp_files;
        if let Some(merged) = self.tabs[tab_idx].merged.as_mut() {
            merged.source_parsers = inputs.parsers.clone();
            merged.source_labels = inputs.labels;
            merged.source_line_counts = source_line_counts;
            merged.label_col_width = label_col_width;
            merged.building = Some((0, sources_total));
        }
        self.active_tab = tab_idx;

        let (update_tx, update_rx) = std::sync::mpsc::channel();
        let sources = inputs.sources;
        let parsers = inputs.parsers;
        let year_maps = inputs.year_maps;
        let continuation_maps = inputs.continuation_maps;
        tokio::task::spawn_blocking(move || {
            build_merged_index_streaming(
                &sources,
                &parsers,
                &year_maps,
                &continuation_maps,
                &update_tx,
            );
        });

        self.pending_merge_builds.push(crate::ui::MergeBuildState {
            tab_idx,
            sources_arc,
            update_rx,
        });
    }

    /// Removes the tab created by [`Self::create_pending_merged_tab`] when
    /// Phase 1 fails before producing any sources to merge (e.g. every
    /// merge-marked file had an unrecognized format) — there's nothing left
    /// to fill it in, so it would otherwise sit around forever showing
    /// "building" progress that will never move.
    pub(crate) fn remove_pending_merged_tab(&mut self, tab_idx: usize) {
        self.remove_tab_at(tab_idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::{Database, LogManager};
    use crate::theme::Theme;

    async fn make_app() -> App {
        let file_reader = FileReader::from_bytes(b"initial line\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await
    }

    async fn push_tab_with_format(app: &mut App, content: &str, format_name: &str) -> usize {
        let fr = FileReader::from_bytes(content.as_bytes().to_vec());
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut tab = TabState::new(fr, lm, "src".to_string());
        tab.display.format = crate::parser::find_builtin_parser(format_name).map(Arc::from);
        app.tabs.push(tab);
        app.tabs.len() - 1
    }

    fn write_include_error_filter(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(
            &path,
            r#"[{"id":1,"pattern":"error","filter_type":"Include","enabled":true,"color_config":null,"use_regex":false,"ignore_case":false,"group":null}]"#,
        )
        .unwrap();
        path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn test_open_merge_tab_applies_default_filter_when_all_sources_share_a_format() {
        let mut app = make_app().await;
        let dir = tempfile::tempdir().unwrap();
        let path = write_include_error_filter(&dir, "f.json");
        app.default_filter_files.insert("syslog".to_string(), path);
        let a = push_tab_with_format(&mut app, "2024-01-01T00:00:00Z a\n", "syslog").await;
        let b = push_tab_with_format(&mut app, "2024-01-01T00:00:01Z b\n", "syslog").await;

        app.open_merge_tab(vec![a, b]).await;

        let merged_tab = app.tabs.last().unwrap();
        assert!(
            merged_tab.merged.is_some(),
            "sanity check: last tab should be the merged one"
        );
        assert_eq!(merged_tab.log_manager.get_filters().len(), 1);
        assert_eq!(merged_tab.log_manager.get_filters()[0].pattern, "error");
    }

    #[tokio::test]
    async fn test_open_merge_tab_does_not_apply_default_filter_when_sources_have_mixed_formats() {
        let mut app = make_app().await;
        let dir = tempfile::tempdir().unwrap();
        let path = write_include_error_filter(&dir, "f.json");
        app.default_filter_files.insert("syslog".to_string(), path);
        let a = push_tab_with_format(&mut app, "2024-01-01T00:00:00Z a\n", "syslog").await;
        let b = push_tab_with_format(&mut app, "{\"msg\":\"b\"}\n", "logfmt").await;

        app.open_merge_tab(vec![a, b]).await;

        let merged_tab = app.tabs.last().unwrap();
        assert!(
            merged_tab.log_manager.get_filters().is_empty(),
            "no single default filter file applies to a mix of formats"
        );
    }

    #[tokio::test]
    async fn test_open_merge_tab_noop_when_no_mapping_for_the_shared_format() {
        let mut app = make_app().await;
        let a = push_tab_with_format(&mut app, "2024-01-01T00:00:00Z a\n", "syslog").await;
        let b = push_tab_with_format(&mut app, "2024-01-01T00:00:01Z b\n", "syslog").await;

        app.open_merge_tab(vec![a, b]).await;

        let merged_tab = app.tabs.last().unwrap();
        assert!(merged_tab.log_manager.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_handle_open_merge_select_includes_a_previously_merged_tab() {
        let mut app = make_app().await;
        let a = push_tab_with_format(&mut app, "2024-01-01T00:00:00Z a\n", "syslog").await;
        let b = push_tab_with_format(&mut app, "2024-01-01T00:00:01Z b\n", "syslog").await;
        app.open_merge_tab(vec![a, b]).await;

        app.handle_open_merge_select();

        let debug = format!("{:?}", app.tabs[app.active_tab].interaction.mode);
        assert!(
            debug.contains("merged(2)"),
            "a tab that is itself the result of a merge (e.g. from the archive \
             picker) must still be selectable as a source for a further merge; \
             got mode: {debug}"
        );
    }

    #[tokio::test]
    async fn test_open_merge_tab_includes_lines_from_a_merged_tab_source() {
        let mut app = make_app().await;
        let a = push_tab_with_format(&mut app, "2024-01-01T00:00:00Z a\n", "syslog").await;
        let b = push_tab_with_format(&mut app, "2024-01-01T00:00:01Z b\n", "syslog").await;
        app.open_merge_tab(vec![a, b]).await;
        let inner_merged = app.tabs.len() - 1;

        let c = push_tab_with_format(&mut app, "2024-01-01T00:00:02Z c\n", "syslog").await;
        app.open_merge_tab(vec![inner_merged, c]).await;

        let outer_merged = app.tabs.last().unwrap();
        assert_eq!(
            outer_merged.filter.visible_indices.len(),
            3,
            "nested merge must include both lines from the inner merged tab \
             instead of silently dropping them for lacking a `display.format`"
        );
    }
}
