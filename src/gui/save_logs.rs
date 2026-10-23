use crate::gui::{app::App, state::StatusMessage};
use crate::ingestion::FileReader;
use gpui_kit::gpui::Context;
use std::{
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

pub(super) fn same_file(input: &Path, output: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(input), std::fs::metadata(output)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(input), std::fs::canonicalize(output)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

pub(super) fn write_logs(
    reader: &FileReader,
    indices: &[usize],
    sources: &[PathBuf],
    path: &Path,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !sources.iter().any(|source| same_file(source, path)),
        "Output is the same file as an input source"
    );
    let mut writer = BufWriter::new(std::fs::File::create(path)?);
    for &line in indices {
        if line < reader.line_count() {
            writer.write_all(&reader.get_raw_line(line))?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
}

impl App {
    pub(super) fn prompt_save_logs(
        &self,
        tab_id: crate::shared::TabId,
        marked: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.state.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let reader = tab.file_reader.clone();
        let indices = if marked {
            tab.mark_manager.get_indices()
        } else if tab.is_temp_backed() {
            (0..reader.line_count()).collect()
        } else {
            tab.filter.visible_indices.iter().collect()
        };
        let mut sources: Vec<PathBuf> = tab
            .log_manager
            .source_file()
            .map(PathBuf::from)
            .into_iter()
            .collect();
        if let Some(merged) = &tab.merged {
            sources.extend(
                self.state
                    .tabs
                    .iter()
                    .filter(|t| merged.source_tab_ids.contains(&t.id))
                    .filter_map(|t| t.log_manager.source_file().map(PathBuf::from)),
            );
        }
        let prompt = cx.prompt_for_new_path(
            &std::env::current_dir().unwrap_or_default(),
            Some(if marked { "bookmarks.log" } else { "logs.log" }),
        );
        cx.spawn(async move |this, cx| {
            let result = async {
                let Some(path) = prompt.await?? else {
                    return Ok(None);
                };
                crate::gui::runtime::handle()
                    .spawn_blocking(move || {
                        write_logs(&reader, &indices, &sources, &path).map(|_| path)
                    })
                    .await?
                    .map(Some)
            }
            .await;
            this.update_in(cx, |app, _, cx| {
                match result {
                    Ok(Some(path)) => app
                        .state
                        .notify(StatusMessage::Info(format!("Saved {}", path.display()))),
                    Ok(None) => return,
                    Err(error) => app
                        .state
                        .notify(StatusMessage::Error(format!("Cannot save logs: {error}"))),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_original_hidden_data_and_refuses_input_aliases() {
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), b"\x1b[31m{\"hidden\":1}\x1b[0m\r\nother\n").unwrap();
        let reader = FileReader::new(source.path().to_str().unwrap()).unwrap();
        let out = tempfile::NamedTempFile::new().unwrap();
        write_logs(&reader, &[0], &[source.path().into()], out.path()).unwrap();
        assert_eq!(
            std::fs::read(out.path()).unwrap(),
            b"\x1b[31m{\"hidden\":1}\x1b[0m\n"
        );
        assert!(write_logs(&reader, &[0], &[source.path().into()], source.path()).is_err());
        #[cfg(unix)]
        {
            let alias = source.path().with_extension("alias");
            std::fs::hard_link(source.path(), &alias).unwrap();
            assert!(write_logs(&reader, &[0], &[source.path().into()], &alias).is_err());
            std::fs::remove_file(alias).unwrap();
        }
    }
}
