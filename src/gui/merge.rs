//! Timestamp merges use immutable source snapshots and the existing cancelable loader.
use crate::{
    gui::{
        app::App,
        loading::{Operation, install_loaded, permit},
        message::FileLoaded,
        runtime,
        state::GuiState,
    },
    ingestion::{
        FileReader,
        archive_job::{Control, Progress},
    },
    parser::LogFormatParser,
    shared::{MergedState, TabId, TabState},
};
use gpui_kit::gpui::Context;
use std::{path::PathBuf, sync::Arc};

pub(super) fn source_parser(tab: &TabState) -> Option<Arc<dyn LogFormatParser>> {
    tab.display
        .format
        .clone()
        .or_else(|| tab.merged.as_ref().and_then(MergedState::uniform_parser))
}

struct Sources {
    readers: Vec<FileReader>,
    parsers: Vec<Option<Arc<dyn LogFormatParser>>>,
    years: Vec<Option<Arc<crate::shared::tab_state::year_map::YearMap>>>,
    continuations: Vec<Option<Arc<Vec<usize>>>>,
    ids: Vec<TabId>,
    labels: Vec<String>,
    retained: Option<Arc<Vec<crate::ingestion::MergedEntry>>>,
    changed: Vec<bool>,
}
impl Sources {
    fn open(state: &GuiState, ids: &[TabId]) -> Result<Self, String> {
        let tabs = ids
            .iter()
            .map(|id| {
                state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == *id)
                    .ok_or("A merge source was closed.".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            readers: tabs.iter().map(|t| t.file_reader.clone()).collect(),
            parsers: tabs.iter().map(|t| source_parser(t)).collect(),
            years: tabs.iter().map(|t| t.year_map.clone()).collect(),
            continuations: tabs.iter().map(|t| t.continuation_map.clone()).collect(),
            ids: ids.to_vec(),
            labels: tabs.iter().map(|t| t.title.clone()).collect(),
            retained: None,
            changed: vec![true; ids.len()],
        })
    }
    fn refresh(state: &GuiState, tab: &TabState) -> Option<Self> {
        let merged = tab.merged.as_ref()?;
        if merged.stopped || tab.stream.paused || merged.source_tab_ids.is_empty() {
            return None;
        }
        let frozen = tab.file_reader.merged_sources()?;
        let mut readers = Vec::new();
        let mut parsers = Vec::new();
        let mut years = Vec::new();
        let mut continuations = Vec::new();
        let mut changed = Vec::new();
        for (index, id) in merged.source_tab_ids.iter().enumerate() {
            if let Some(source) = state.tabs.iter().find(|t| t.id == *id) {
                let parser = source_parser(source);
                changed.push(
                    !source.file_reader.same_snapshot(frozen.get(index)?)
                        || parser.as_ref().map(|p| p.name())
                            != merged.source_parsers[index].as_ref().map(|p| p.name()),
                );
                parsers.push(parser);
                readers.push(source.file_reader.clone());
                years.push(source.year_map.clone());
                continuations.push(source.continuation_map.clone());
            } else {
                readers.push(frozen.get(index)?.clone());
                changed.push(false);
                parsers.push(merged.source_parsers[index].clone());
                // Unchanged sources retain their original entries, including continuation timestamps.
                years.push(None);
                continuations.push(None);
            }
        }
        changed.iter().any(|on| *on).then(|| Self {
            readers,
            parsers,
            years,
            continuations,
            ids: merged.source_tab_ids.clone(),
            labels: merged.source_labels.clone(),
            retained: tab.file_reader.merged_entries().cloned(),
            changed,
        })
    }
    fn build(self, control: &Control) -> Result<(FileReader, MergedState), String> {
        control.phase("Merging logs", String::new(), None);
        let mut entries = Vec::new();
        for index in 0..self.readers.len() {
            control.check().map_err(|e| e.to_string())?;
            if !self.changed[index] {
                if let Some(retained) = &self.retained {
                    entries.extend(retained.iter().filter(|e| e.source_idx == index).cloned());
                }
                continue;
            }
            // ponytail: rebuild each changed source; use incremental extension if live-merge profiling warrants it.
            let source_entries = crate::shared::tab_state::merged::build_merged_index_controlled(
                &self.readers[index..index + 1],
                &self.parsers[index..index + 1],
                &self.years[index..index + 1],
                &self.continuations[index..index + 1],
                Some(control),
            )
            .map_err(|e| e.to_string())?;
            entries.extend(source_entries.into_iter().map(|mut entry| {
                entry.source_idx = index;
                entry
            }));
        }
        entries.sort_by_key(|entry| entry.sort_key);
        control.check().map_err(|e| e.to_string())?;
        let merged = MergedState {
            source_line_counts: self.readers.iter().map(FileReader::line_count).collect(),
            source_tab_ids: self.ids,
            source_parsers: self.parsers,
            label_col_width: self.labels.iter().map(String::len).max().unwrap_or(0),
            source_labels: self.labels,
            stopped: false,
            building: None,
        };
        Ok((
            FileReader::from_merged(Arc::new(entries), Arc::new(self.readers)),
            merged,
        ))
    }
}

fn apply_snapshot(tab: &mut TabState, reader: FileReader, merged: MergedState) {
    let positions: std::collections::HashMap<_, _> = reader
        .merged_entries()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, entry)| ((entry.source_idx, entry.line_idx), index))
        .collect();
    let remap = |line: usize| {
        let old = tab.file_reader.merged_entries()?.get(line)?;
        let index = *positions.get(&(old.source_idx, old.line_idx))?;
        (tab.file_reader.get_line(line) == reader.get_line(index)).then_some(index)
    };
    let marks = tab
        .mark_manager
        .get_indices()
        .into_iter()
        .filter_map(remap)
        .collect();
    let mut comments = tab.comment_manager.get().to_vec();
    for comment in &mut comments {
        comment.line_indices = comment
            .line_indices
            .iter()
            .copied()
            .filter_map(remap)
            .collect();
    }
    comments.retain(|comment| !comment.line_indices.is_empty());
    let anchor = tab
        .filter
        .visible_indices
        .get_opt(tab.scroll.scroll_offset)
        .and_then(remap);
    tab.mark_manager.set(marks);
    tab.comment_manager.set(comments);
    tab.file_reader = reader;
    tab.merged = Some(merged);
    tab.invalidate_parse_cache();
    tab.filter.cached_scan = None;
    tab.filter.saved_view = None;
    tab.begin_filter_refresh();
    if let Some(query) = tab.search.query.get_pattern().map(str::to_owned) {
        tab.begin_search(&query, true, false);
    }
    if let Some(handle) = &mut tab.filter.handle {
        handle.scroll_anchor = anchor;
    } else {
        tab.restore_scroll_to_line(anchor);
    }
}

impl App {
    pub(super) fn merge_tabs(&mut self, ids: Vec<TabId>, cx: &mut Context<Self>) {
        let sources = match Sources::open(&self.state, &ids) {
            Ok(sources) => sources,
            Err(error) => {
                self.state
                    .notify(crate::gui::state::StatusMessage::Error(error));
                return;
            }
        };
        if sources.readers.len() < 2 {
            return;
        }
        let title = format!("merged({})", ids.len());
        self.loading.next_id += 1;
        let id = self.loading.next_id;
        let (control, rx) = Control::new();
        self.loading.operations.push(Operation {
            id,
            path: PathBuf::from(&title),
            control: control.clone(),
            rx,
            progress: Progress::default(),
            picker: None,
            extraction_confirmed: false,
            args: None,
            preview: None,
        });
        let gate = self.loading.gate.clone();
        let db = self.state.db.clone();
        cx.spawn(async move |this, cx| {
            let worker = control.clone();
            let result = runtime::handle()
                .spawn(async move {
                    let permit = permit(gate, &worker).await?;
                    let (reader, merged) =
                        tokio::task::spawn_blocking(move || sources.build(&worker))
                            .await
                            .map_err(|e| e.to_string())??;
                    let manager = crate::db::LogManager::new(db, Some(title.clone())).await;
                    let (_, rx) = tokio::sync::watch::channel(());
                    let loaded = FileLoaded {
                        context: None,
                        path: title.into(),
                        reader,
                        log_manager: manager,
                        watch: crate::shared::watch_state_from_file(rx, String::new()),
                    };
                    Ok::<_, String>((loaded, merged, permit))
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            let result = match result {
                Ok((loaded, merged, _permit)) => {
                    install_loaded(
                        &this,
                        cx,
                        Ok(loaded),
                        control,
                        Some(id),
                        true,
                        false,
                        None,
                        Some(merged),
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            this.update(cx, |app, cx| app.finish_loading(id, result, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn merge_archive(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(op) = self.loading.operations.iter_mut().find(|op| op.id == id) else {
            return;
        };
        let Some(picker) = op.picker.as_ref() else {
            return;
        };
        let count = picker
            .tree
            .nodes
            .iter()
            .filter(|node| {
                node.selected && matches!(node.kind, crate::ingestion::archive_tree::NodeKind::File)
            })
            .count();
        if count < 2 {
            return;
        }
        op.extraction_confirmed = true;
        let picker = op.picker.take().unwrap();
        let out = picker.output_dir;
        let tree = picker.tree;
        let path = op.path.clone();
        let control = op.control.clone();
        control.phase("Queued", String::new(), None);
        let gate = self.loading.gate.clone();
        let db = self.state.db.clone();
        cx.spawn(async move |this, cx| {
            let worker = control.clone();
            let result = runtime::handle()
                .spawn(async move {
                    let permit = permit(gate, &worker).await?;
                    tokio::task::spawn_blocking(move || {
                        let handle = runtime::handle();
                        let _guard = handle.enter();
                        let mut tabs = Vec::new();
                        let mut temps = Vec::new();
                        crate::ingestion::archive_job::extract_selection(
                            &path,
                            &tree,
                            &worker,
                            &mut |entry| {
                                let (node, temp) = entry?;
                                let label = crate::gui::loading::provenance(&path, &tree, node);
                                let loaded =
                                    handle.block_on(crate::gui::update::load_file_controlled(
                                        temp.path().to_path_buf(),
                                        db.clone(),
                                        worker.clone(),
                                        Some(label.clone()),
                                    ))?;
                                let tab = TabState::new_controlled(
                                    loaded.reader,
                                    loaded.log_manager,
                                    label,
                                    &worker.cancel,
                                )
                                .ok_or("Operation canceled")?;
                                tabs.push(tab);
                                temps.push(temp);
                                Ok(())
                            },
                        )?;
                        let count = tabs.len();
                        let sources = Sources {
                            readers: tabs.iter().map(|t| t.file_reader.clone()).collect(),
                            parsers: tabs.iter().map(source_parser).collect(),
                            years: tabs.iter().map(|t| t.year_map.clone()).collect(),
                            continuations: tabs
                                .iter()
                                .map(|t| t.continuation_map.clone())
                                .collect(),
                            ids: Vec::new(),
                            labels: tabs.iter().map(|t| t.title.clone()).collect(),
                            retained: None,
                            changed: vec![true; count],
                        };
                        let (reader, mut merged) = sources.build(&worker)?;
                        merged.stopped = true;
                        let mut output =
                            tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
                        for line in 0..reader.line_count() {
                            worker.check().map_err(|error| error.to_string())?;
                            std::io::Write::write_all(&mut output, &reader.get_line(line))
                                .map_err(|error| error.to_string())?;
                            std::io::Write::write_all(&mut output, b"\n")
                                .map_err(|error| error.to_string())?;
                        }
                        let relative =
                            crate::gui::loading::path_for_archive_output(&path, "merged.log");
                        let saved = crate::gui::startup::persist_extracted_unique(
                            output, &out, &relative, &worker,
                        )?;
                        let manager = handle.block_on(crate::db::LogManager::new(
                            db,
                            Some(saved.to_string_lossy().into_owned()),
                        ));
                        let (_, rx) = tokio::sync::watch::channel(());
                        let loaded = FileLoaded {
                            context: None,
                            path: saved,
                            reader,
                            log_manager: manager,
                            watch: crate::shared::watch_state_from_file(rx, String::new()),
                        };
                        // Keep extracted files alive through display preparation as well.
                        Ok::<_, String>((loaded, merged, temps, permit))
                    })
                    .await
                    .map_err(|e| e.to_string())?
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            let result = match result {
                Ok((loaded, merged, _temps, _permit)) => {
                    install_loaded(
                        &this,
                        cx,
                        Ok(loaded),
                        control,
                        Some(id),
                        true,
                        false,
                        None,
                        Some(merged),
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            this.update(cx, |app, cx| app.finish_loading(id, result, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn poll_merges(&mut self, cx: &mut Context<Self>) {
        for index in 0..self.state.tabs.len() {
            let id = self.state.tabs[index].id;
            if self.stream_pending.contains(&id) {
                continue;
            }
            let Some(sources) = Sources::refresh(&self.state, &self.state.tabs[index]) else {
                continue;
            };
            self.stream_pending.insert(id);
            cx.spawn(async move |this, cx| {
                let (control, _) = Control::new();
                let result = runtime::handle()
                    .spawn_blocking(move || sources.build(&control))
                    .await;
                this.update(cx, |app, cx| {
                    app.stream_pending.remove(&id);
                    let Some(tab) = app.state.tabs.iter_mut().find(|t| t.id == id) else {
                        return;
                    };
                    if tab.stream.paused || tab.merged.as_ref().is_none_or(|m| m.stopped) {
                        return;
                    }
                    match result {
                        Ok(Ok((reader, merged))) => {
                            let handle = runtime::handle();
                            let _guard = handle.enter();
                            apply_snapshot(tab, reader, merged);
                            if app
                                .state
                                .log_selection
                                .is_some_and(|(source, _, _)| source == id)
                            {
                                app.state.log_selection = None;
                            }
                            let tab = app.state.tabs.iter_mut().find(|t| t.id == id).unwrap();
                            if tab.stream.tail_mode {
                                tab.scroll.scroll_offset =
                                    tab.filter.visible_indices.len().saturating_sub(1);
                                if app.state.active_tab().is_some_and(|t| t.id == id) {
                                    app.sync_log_scroll();
                                }
                            }
                        }
                        Ok(Err(error)) => app
                            .state
                            .notify(crate::gui::state::StatusMessage::Error(error)),
                        Err(error) => app
                            .state
                            .notify(crate::gui::state::StatusMessage::Error(error.to_string())),
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::{actions::Action, effect::Effect, message::Message, update::update};

    fn state() -> GuiState {
        let handle = runtime::handle();
        let db = Arc::new(handle.block_on(crate::db::Database::in_memory()).unwrap());
        let mut state = GuiState::new(db.clone());
        for (title, lines) in [
            (
                "a.log",
                "Jan  1 00:00:01 host tag: a1\nJan  1 00:00:03 host tag: a3\n",
            ),
            ("b.log", "Jan  1 00:00:02 host tag: b2\n"),
        ] {
            let manager = handle.block_on(crate::db::LogManager::new(db.clone(), None));
            state.tabs.push(TabState::new(
                FileReader::from_bytes(lines.as_bytes().to_vec()),
                manager,
                title.into(),
            ));
        }
        state
    }
    fn build(sources: Sources) -> (FileReader, MergedState) {
        let (control, _) = Control::new();
        sources.build(&control).unwrap()
    }

    #[test]
    fn picker_validates_selection_and_uses_stable_ids_after_tab_closure() {
        let handle = runtime::handle();
        let _guard = handle.enter();
        let mut state = state();
        let ids: Vec<_> = state.tabs.iter().map(|tab| tab.id).collect();
        update(&mut state, Message::GuiAction(Action::Merge));
        assert!(matches!(
            update(&mut state, Message::EditorApply),
            Effect::None
        ));
        assert!(state.editor.as_ref().unwrap().error.is_some());
        update(&mut state, Message::EditorAll(true));
        assert!(
            matches!(update(&mut state, Message::EditorApply), Effect::MergeTabs(selected) if selected == ids)
        );
        update(&mut state, Message::GuiAction(Action::Merge));
        update(&mut state, Message::EditorAll(true));
        state.tabs.remove(0);
        assert!(matches!(
            update(&mut state, Message::EditorApply),
            Effect::None
        ));
        assert!(
            state
                .editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("closed")
        );
        update(&mut state, Message::EditorCancel);
        assert!(state.editor.is_none());
    }

    #[test]
    fn same_count_replacement_and_newly_detected_formats_refresh() {
        let handle = runtime::handle();
        let _guard = handle.enter();
        let mut state = state();
        state.tabs[1].file_reader = FileReader::from_bytes(b"unrecognized\n".to_vec());
        state.tabs[1].display.format = None;
        let ids: Vec<_> = state.tabs.iter().map(|t| t.id).collect();
        let (reader, merged) = build(Sources::open(&state, &ids).unwrap());
        let manager = handle.block_on(crate::db::LogManager::new(state.db.clone(), None));
        let mut tab = TabState::new(reader, manager, "merged".into());
        tab.display.format = None;
        tab.merged = Some(merged);
        state.tabs.push(tab);
        state.tabs[1].file_reader =
            FileReader::from_bytes(b"Jan  1 00:00:02 host tag: detected\n".to_vec());
        state.tabs[1].detect_and_apply_format();
        let (reader, merged) = build(Sources::refresh(&state, &state.tabs[2]).unwrap());
        assert_eq!(reader.line_count(), 3);
        assert!(String::from_utf8_lossy(&reader.get_line(1)).contains("detected"));
        apply_snapshot(&mut state.tabs[2], reader, merged);
        state.tabs[1].file_reader =
            FileReader::from_bytes(b"Jan  1 00:00:04 host tag: replaced\n".to_vec());
        let (reader, merged) = build(Sources::refresh(&state, &state.tabs[2]).unwrap());
        state.tabs[2].begin_search("replaced", true, false);
        let old_search = state.tabs[2].search.handle.as_ref().unwrap().cancel.clone();
        apply_snapshot(&mut state.tabs[2], reader, merged);
        assert!(old_search.load(std::sync::atomic::Ordering::Relaxed));
        assert!(state.tabs[2].search.handle.is_some());
        assert!(
            String::from_utf8_lossy(&state.tabs[2].file_reader.get_line(2)).contains("replaced")
        );
        assert!(Sources::refresh(&state, &state.tabs[2]).is_none());
    }

    #[test]
    fn paged_source_growth_with_and_without_newlines_refreshes_merge() {
        use std::io::Write;
        let handle = runtime::handle();
        let _guard = handle.enter();
        let mut state = state();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"Jan  1 00:00:02 host tag: part").unwrap();
        state.tabs[1].file_reader = FileReader::try_new_paged(
            file.path().to_str().unwrap(),
            Arc::new(file.path().to_path_buf()),
        )
        .unwrap()
        .unwrap();
        state.tabs[1].detect_and_apply_format();
        let ids: Vec<_> = state.tabs.iter().map(|t| t.id).collect();
        let (reader, merged) = build(Sources::open(&state, &ids).unwrap());
        let manager = handle.block_on(crate::db::LogManager::new(state.db.clone(), None));
        let mut tab = TabState::new(reader, manager, "merged".into());
        tab.display.format = None;
        tab.merged = Some(merged);
        state.tabs.push(tab);
        for appended in ["ial", "\nJan  1 00:00:04 host tag: next\n"] {
            file.write_all(appended.as_bytes()).unwrap();
            assert!(state.tabs[1].file_reader.try_extend_from_read().unwrap());
            let (reader, merged) = build(Sources::refresh(&state, &state.tabs[2]).unwrap());
            apply_snapshot(&mut state.tabs[2], reader, merged);
            assert!(Sources::refresh(&state, &state.tabs[2]).is_none());
        }
        assert_eq!(state.tabs[2].file_reader.line_count(), 4);
        assert!(
            String::from_utf8_lossy(&state.tabs[2].file_reader.get_line(1)).contains("partial")
        );
    }

    #[test]
    fn live_merge_retains_closed_sources_and_remaps_notes_and_bookmarks() {
        let handle = runtime::handle();
        let _guard = handle.enter();
        let mut state = state();
        let ids: Vec<_> = state.tabs.iter().map(|tab| tab.id).collect();
        let (reader, merged) = build(Sources::open(&state, &ids).unwrap());
        assert_eq!(reader.line_count(), 3);
        assert!(String::from_utf8_lossy(&reader.get_line(1)).contains("b2"));
        let manager = handle.block_on(crate::db::LogManager::new(state.db.clone(), None));
        let mut tab = TabState::new(reader, manager, "merged".into());
        tab.display.format = None;
        tab.merged = Some(merged);
        tab.mark_manager.mark(2);
        tab.comment_manager.add("a3 note".into(), vec![2]);
        state.tabs.push(tab);
        // Close the first source: the remaining source must keep slot 1.
        state.tabs.remove(0);
        state.tabs[0].file_reader = FileReader::from_bytes(
            b"Jan  1 00:00:02 host tag: b2\nJan  1 00:00:02 host tag: b2again\n".to_vec(),
        );
        state.tabs[1].stream.paused = true;
        assert!(Sources::refresh(&state, &state.tabs[1]).is_none());
        state.tabs[1].stream.paused = false;
        let sources = Sources::refresh(&state, &state.tabs[1]).unwrap();
        let (reader, merged) = build(sources);
        assert_eq!(reader.line_count(), 4);
        assert!(String::from_utf8_lossy(&reader.get_line(0)).contains("a1"));
        assert!(String::from_utf8_lossy(&reader.get_line(2)).contains("b2again"));
        apply_snapshot(&mut state.tabs[1], reader, merged);
        assert_eq!(state.tabs[1].mark_manager.get_indices(), vec![3]);
        assert_eq!(state.tabs[1].comment_manager.get()[0].line_indices, vec![3]);
        assert!(Sources::refresh(&state, &state.tabs[1]).is_none());
        // Truncation replaces only the changed source's entries.
        state.tabs[0].file_reader =
            FileReader::from_bytes(b"Jan  1 00:00:04 host tag: replacement\n".to_vec());
        let (reader, merged) = build(Sources::refresh(&state, &state.tabs[1]).unwrap());
        assert_eq!(reader.line_count(), 3);
        apply_snapshot(&mut state.tabs[1], reader, merged);
        state.active_tab = 1;
        update(
            &mut state,
            Message::GuiCommand(crate::commands::Commands::Stop, "Stop".into()),
        );
        assert!(state.tabs[1].merged.as_ref().unwrap().stopped);
        assert!(Sources::refresh(&state, &state.tabs[1]).is_none());
        assert!(!crate::gui::update::is_tab_live(&state.tabs[1]));
    }
}
