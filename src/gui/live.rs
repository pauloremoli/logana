//! Apply watcher snapshots in the background, retaining tab identity and pause state.
use crate::gui::{app::App, state::StatusMessage};
use crate::ingestion::FileReader;
use gpui_kit::gpui::Context;
use std::path::Path;

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
            self.stream_pending.insert(id);
            cx.spawn(async move |this, cx| {
                let read_path = path.clone();
                let result = crate::gui::runtime::handle()
                    .spawn_blocking(move || read_snapshot(reader, &read_path))
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
                        Ok(Ok((reader, incremental))) => {
                            let replaced =
                                !incremental && reader.line_count() < tab.file_reader.line_count();
                            tab.file_reader = reader;
                            if replaced {
                                tab.mark_manager.clear();
                                tab.comment_manager.clear();
                                app.state.log_selection = None;
                            }
                            if tab.display.format.is_none() && tab.file_reader.line_count() > 0 {
                                tab.detect_and_apply_format();
                            }
                            // Full background filtering also rechecks a previously incomplete last line.
                            tab.begin_filter_refresh();
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
