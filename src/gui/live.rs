//! Apply watcher snapshots in the background, retaining tab identity and pause state.
use crate::gui::{app::App, state::StatusMessage};
use crate::ingestion::FileReader;
use gpui_kit::gpui::Context;
use std::path::Path;

struct StreamSnapshot {
    reader: FileReader,
    incremental: bool,
    metadata: Option<crate::ingestion::format_detect::DetectedFormat>,
    hidden_fields: std::collections::HashSet<String>,
}

fn read_prepared_snapshot(
    reader: FileReader,
    path: &Path,
    detect: bool,
) -> std::io::Result<StreamSnapshot> {
    let (reader, incremental) = read_snapshot(reader, path)?;
    let metadata =
        detect.then(|| crate::ingestion::format_detect::detect_format_for_reader(&reader));
    let hidden_fields = metadata
        .as_ref()
        .and_then(|metadata| metadata.format.as_ref())
        .map(|parser| {
            let rows = (0..reader.line_count().min(200))
                .map(|line| reader.get_line(line))
                .collect::<Vec<_>>();
            let sample = rows.iter().map(|row| &**row).collect::<Vec<_>>();
            parser.default_hidden_fields(&sample)
        })
        .unwrap_or_default();
    Ok(StreamSnapshot {
        reader,
        incremental,
        metadata,
        hidden_fields,
    })
}

fn read_snapshot(mut reader: FileReader, path: &Path) -> std::io::Result<(FileReader, bool)> {
    let incremental = reader.try_extend_from_read()?;
    if !incremental {
        reader = FileReader::new(
            path.to_str()
                .ok_or_else(|| std::io::Error::other("Invalid source path"))?,
        )?;
    }
    Ok((reader, incremental))
}

impl App {
    pub(super) fn poll_streams(&mut self, cx: &mut Context<Self>) {
        self.stream_pending
            .retain(|id| self.state.tabs.iter().any(|t| t.id == *id));
        for tab in &mut self.state.tabs {
            if tab.stream.paused || self.stream_pending.contains(&tab.id) {
                continue;
            }
            let Some(watch) = &mut tab.stream.watch else {
                continue;
            };
            let closed = match watch.snapshot_rx.has_changed() {
                Ok(true) => false,
                Ok(false) => continue,
                Err(_) => true,
            };
            watch.snapshot_rx.borrow_and_update();
            let path = watch.reader_path.clone();
            let reader = tab.file_reader.clone();
            let id = tab.id;
            let detect = tab.display.format.is_none();
            self.stream_pending.insert(id);
            cx.spawn(async move |this, cx| {
                let read_path = path.clone();
                let result = crate::gui::runtime::handle()
                    .spawn_blocking(move || read_prepared_snapshot(reader, &read_path, detect))
                    .await;
                this.update_in(cx, |app, _, cx| {
                    let handle = crate::gui::runtime::handle();
                    let _guard = handle.enter();
                    app.stream_pending.remove(&id);
                    let Some(index) = app.state.tabs.iter().position(|t| t.id == id) else {
                        return;
                    };
                    let tab = &mut app.state.tabs[index];
                    if !tab
                        .stream
                        .watch
                        .as_ref()
                        .is_some_and(|w| w.reader_path == path)
                    {
                        return;
                    }
                    match result {
                        Ok(Ok(StreamSnapshot { reader, incremental, metadata, hidden_fields })) => {
                            let replaced =
                                !incremental && reader.line_count() < tab.file_reader.line_count();
                            tab.file_reader = reader;
                            if replaced {
                                tab.mark_manager.clear();
                                tab.comment_manager.clear();
                                app.state.log_selection = None;
                            }
                            if tab.display.format.is_none() && let Some(metadata) = metadata {
                                tab.display.format = metadata.format;
                                tab.continuation_map = metadata.continuation_map;
                                tab.year_map = metadata.year_map;
                                if tab.display.hidden_fields.is_empty() && !hidden_fields.is_empty() {
                                    tab.display.hidden_fields = hidden_fields;
                                    tab.set_notification("Some fields are hidden. Use 'select-fields' to choose fields or 'show-all-fields' to show all.");
                                }
                                tab.invalidate_parse_cache();
                            }
                            // Full background filtering also rechecks a previously incomplete last line.
                            tab.begin_filter_refresh_background();
                            let follow = tab.stream.tail_mode && !tab.stream.paused;
                            if follow {
                                tab.scroll.scroll_offset =
                                    tab.filter.visible_indices.len().saturating_sub(1);
                            }
                            if index == app.state.active_tab && follow {
                                app.log_scroll
                                    .scroll_to_reveal_item(tab.scroll.scroll_offset);
                            }
                            if closed {
                                tab.stream.watch = None;
                            }
                            cx.notify();
                        }
                        Ok(Err(error)) => app.state.notify(StatusMessage::Error(format!(
                            "Cannot refresh source: {error}"
                        ))),
                        Err(error) => app.state.notify(StatusMessage::Error(format!(
                            "Source refresh task failed: {error}"
                        ))),
                    }
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
    use std::io::Write;
    #[test]
    fn first_stream_snapshot_prepares_format_metadata() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
        file.write_all(b"{\"level\":\"INFO\",\"msg\":\"value1\"}\n")
            .unwrap();
        let snapshot = read_prepared_snapshot(reader, file.path(), true).unwrap();
        assert!(snapshot.incremental);
        let metadata = snapshot.metadata.unwrap();
        assert!(metadata.format.is_some());
        assert_eq!(
            metadata.continuation_map.unwrap().len(),
            snapshot.reader.line_count()
        );
        let snapshot = read_prepared_snapshot(snapshot.reader, file.path(), false).unwrap();
        assert!(snapshot.metadata.is_none());
    }

    #[test]
    fn growth_partial_lines_and_truncation_are_consumed() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"first\npart").unwrap();
        let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
        file.write_all(b"ial\nnext\n").unwrap();
        let (reader, incremental) = read_snapshot(reader, file.path()).unwrap();
        assert!(incremental);
        assert_eq!(reader.get_line(1), b"partial");
        assert_eq!(reader.line_count(), 3);
        std::fs::write(file.path(), b"replacement\n").unwrap();
        let (reader, incremental) = read_snapshot(reader, file.path()).unwrap();
        assert!(!incremental);
        assert_eq!(reader.get_line(0), b"replacement");
    }
}
