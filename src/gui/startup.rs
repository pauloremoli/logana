//! CLI startup uses the same grammar as the TUI and the GUI's managed workers.
use crate::{
    cli::Args,
    db::LogManager,
    gui::{
        app::App,
        loading::{Operation, install_loaded, permit},
        persistence::Preferences,
        runtime,
        sources::Source,
    },
    ingestion::archive_job::{Control, Progress},
};
use gpui_kit::gpui::{Context, Window};
use std::{
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) fn has_filters(args: &Args) -> bool {
    args.filters.is_some()
        || !args.include_filters.is_empty()
        || !args.exclude_filters.is_empty()
        || !args.timestamp_filters.is_empty()
}

pub(super) async fn apply_filters(
    mut manager: LogManager,
    args: Args,
    background: (u8, u8, u8),
) -> Result<LogManager, String> {
    use crate::commands::{CommandLine, auto_complete::shell_split};
    use clap::Parser;
    if let Some(path) = args.filters {
        manager
            .load_filters(&path, false)
            .await
            .map_err(|e| e.to_string())?;
    }
    for (prefix, filters) in [
        ("filter", args.include_filters),
        ("exclude", args.exclude_filters),
        ("date-filter", args.timestamp_filters),
    ] {
        for filter in filters {
            if let Some(command) =
                CommandLine::try_parse_from(shell_split(&format!("{prefix} {filter}")))
                    .map_err(|e| e.to_string())?
                    .command
            {
                manager =
                    crate::gui::update::execute_command(manager, command, background, None).await?;
            }
        }
    }
    Ok(manager)
}

pub(super) fn persist_extracted(
    temp: tempfile::NamedTempFile,
    out: &Path,
    relative: &str,
    control: &Control,
) -> Result<PathBuf, String> {
    let first = Path::new(relative).components().next();
    let base = match first {
        Some(std::path::Component::Normal(part))
            if Path::new(relative).components().count() > 1 && out.join(part).is_file() =>
        {
            out.join(".logana-extracted")
        }
        _ => out.to_path_buf(),
    };
    let mut destination = base;
    for component in Path::new(relative).components() {
        match component {
            std::path::Component::Normal(part) => destination.push(part),
            std::path::Component::CurDir => {}
            _ => return Err(format!("Unsafe archive entry: {relative}")),
        }
    }
    if destination == out {
        return Err("Empty archive entry path".into());
    }
    let parent = destination.parent().ok_or("Missing output directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // Stage on the destination filesystem so publication is atomic across mounts.
    control.phase(
        "Saving extracted file",
        destination.display().to_string(),
        Some(temp.as_file().metadata().map_err(|e| e.to_string())?.len()),
    );
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let mut input = crate::ingestion::archive_job::CheckedReader::new(
        temp.reopen().map_err(|e| e.to_string())?,
        Some(control.clone()),
    );
    std::io::copy(&mut input, &mut staged).map_err(|e| e.to_string())?;
    control.check().map_err(|e| e.to_string())?;
    staged.persist(&destination).map_err(|e| e.to_string())?;
    Ok(destination)
}

fn has_stdin_input() -> bool {
    if std::io::stdin().is_terminal() {
        return false;
    }
    // Desktop launchers commonly attach /dev/null; that should still restore a session.
    #[cfg(unix)]
    {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(libc::STDIN_FILENO, stat.as_mut_ptr()) } != 0 {
            return false;
        }
        let kind = unsafe { stat.assume_init() }.st_mode & libc::S_IFMT;
        matches!(kind, libc::S_IFREG | libc::S_IFIFO | libc::S_IFSOCK)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

impl App {
    pub fn start_cli(
        &mut self,
        args: Args,
        preferences: Preferences,
        session: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.preferences = preferences.clone();
        let config = crate::config::Config::load().unwrap_or_default();
        if let Some(port) = args.mcp {
            self.configure_mcp(Some(config.mcp_port.unwrap_or(port)), cx);
        }
        if let Some(command) = &args.run {
            let mut words = command.split_whitespace();
            if let Some(program) = words.next() {
                self.connect_source_with_args(
                    Source::Run {
                        program: program.into(),
                        args: words.map(str::to_owned).collect(),
                    },
                    Some(args.clone()),
                    cx,
                );
            }
        } else if args.merge {
            self.queue_cli_merge(args, cx);
        } else if args.files.is_empty() && has_stdin_input() {
            self.connect_source_with_args(Source::Stdin, Some(args), cx);
        } else if args.files.is_empty() {
            self.restore_session_with_args(preferences, session, Some(args), window, cx);
        } else {
            for path in &args.files {
                if Path::new(path).is_dir() {
                    self.queue_directory(PathBuf::from(path), Some(args.clone()), cx);
                } else {
                    self.queue_file_with_args(PathBuf::from(path), Some(args.clone()), cx);
                }
            }
        }
    }

    fn cli_operation(&mut self, path: PathBuf, args: Option<Args>) -> (u64, Control) {
        self.loading.next_id += 1;
        let id = self.loading.next_id;
        let (control, rx) = Control::new();
        self.loading.operations.push(Operation {
            id,
            path,
            control: control.clone(),
            rx,
            progress: Progress::default(),
            picker: None,
            args,
            preview: None,
        });
        (id, control)
    }

    pub(super) fn queue_directory(
        &mut self,
        path: PathBuf,
        args: Option<Args>,
        cx: &mut Context<Self>,
    ) {
        let (id, control) = self.cli_operation(path.clone(), args.clone());
        let gate = self.loading.gate.clone();
        cx.spawn(async move |this, cx| {
            let worker = control.clone();
            let result = runtime::handle()
                .spawn(async move {
                    let _permit = permit(gate, &worker).await?;
                    tokio::task::spawn_blocking(move || {
                        worker.phase("Reading directory", path.display().to_string(), None);
                        let mut paths = Vec::new();
                        collect_directory(&path, &worker, &mut paths)?;
                        if paths.is_empty() {
                            return Err(format!("'{}' contains no files.", path.display()));
                        }
                        Ok(paths)
                    })
                    .await
                    .map_err(|e| e.to_string())?
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            match result {
                Ok(paths) => {
                    for batch in paths.chunks(32) {
                        if control.cancelled() {
                            break;
                        }
                        this.update(cx, |app, cx| {
                            for path in batch {
                                app.queue_file_with_args(path.clone(), args.clone(), cx);
                            }
                        })
                        .ok();
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(10))
                            .await;
                    }
                    this.update(cx, |app, cx| app.finish_loading(id, Ok(()), cx))
                        .ok();
                }
                Err(error) => {
                    this.update(cx, |app, cx| app.finish_loading(id, Err(error), cx))
                        .ok();
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn queue_cli_merge(&mut self, args: Args, cx: &mut Context<Self>) {
        let (id, control) = self.cli_operation(PathBuf::from("Merged logs"), Some(args.clone()));
        let gate = self.loading.gate.clone();
        let db = self.state.db.clone();
        cx.spawn(async move |this, cx| {
            let worker = control.clone();
            let result = runtime::handle()
                .spawn(async move {
                    let permit = permit(gate, &worker).await?;
                    let result = tokio::task::spawn_blocking(move || {
                        let handle = runtime::handle();
                        let _entered = handle.enter();
                        build_merge(args, db, worker)
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                    Ok::<_, String>((result?, permit))
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            let result = match result {
                Ok(((loaded, temp, merged), _permit)) => {
                    install_loaded(
                        &this,
                        cx,
                        Ok(loaded),
                        control,
                        Some(id),
                        true,
                        false,
                        temp,
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
}

fn collect_directory(
    path: &Path,
    control: &Control,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        control.check().map_err(|e| e.to_string())?;
        entries.push(entry.map_err(|e| e.to_string())?);
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        control.check().map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            collect_directory(&entry.path(), control, files)?;
        } else if kind.is_file() {
            files.push(entry.path());
            control.bytes(files.len() as u64);
        }
    }
    Ok(())
}

fn build_merge(
    args: Args,
    db: Arc<crate::db::Database>,
    control: Control,
) -> Result<
    (
        crate::gui::message::FileLoaded,
        Option<tempfile::NamedTempFile>,
        crate::shared::MergedState,
    ),
    String,
> {
    let out = crate::cli::resolve_out_dir(args.out.as_ref());
    let mut paths = Vec::new();
    for path in &args.files {
        control.check().map_err(|e| e.to_string())?;
        if crate::ingestion::detect_archive_type(path).is_some() {
            let mut tree = crate::ingestion::archive_tree::list_archive_tree_controlled(
                path,
                control.clone(),
            )?;
            tree.set_all_files_selected(true);
            crate::ingestion::archive_job::extract_selection(
                Path::new(path),
                &tree,
                &control,
                &mut |entry| {
                    let (node, temp) = entry?;
                    let relative = crate::ingestion::archive_tree::relative_name(&tree, node, path);
                    paths.push(persist_extracted(
                        temp,
                        &out.join(".logana-merge"),
                        &relative,
                        &control,
                    )?);
                    Ok(())
                },
            )?;
        } else {
            paths.push(PathBuf::from(path));
        }
    }
    if let Some(output) = &args.output {
        for path in &paths {
            if crate::headless::same_file(&path.to_string_lossy(), output) {
                return Err("Merge output is the same as an input file".into());
            }
        }
    }
    let mut labels = Vec::new();
    let mut sources = Vec::new();
    let mut parsers = Vec::new();
    let mut years = Vec::new();
    let mut continuations = Vec::new();
    for path in paths {
        let loaded = runtime::handle().block_on(crate::gui::update::load_file_controlled(
            path.clone(),
            db.clone(),
            control.clone(),
            None,
        ))?;
        let title = path.display().to_string();
        let tab = crate::shared::TabState::new_controlled(
            loaded.reader,
            loaded.log_manager,
            title.clone(),
            &control.cancel,
        )
        .ok_or("Operation canceled")?;
        labels.push(title);
        if tab.display.format.is_none() {
            eprintln!(
                "Warning: '{}' has no detected timestamp format; its lines will not appear in the merged output.",
                tab.title
            );
        }
        sources.push(tab.file_reader);
        parsers.push(tab.display.format);
        years.push(tab.year_map);
        continuations.push(tab.continuation_map);
    }
    let entries = crate::shared::tab_state::merged::build_merged_index_controlled(
        &sources,
        &parsers,
        &years,
        &continuations,
        Some(&control),
    )
    .map_err(|e| e.to_string())?;
    let mut temp = if let Some(output) = &args.output {
        for path in &args.files {
            if crate::headless::same_file(path, output) {
                return Err("Merge output is the same as an input file".into());
            }
        }
        tempfile::NamedTempFile::new_in(
            output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
    } else {
        tempfile::NamedTempFile::new()
    }
    .map_err(|e| e.to_string())?;
    control.phase(
        "Writing merged logs",
        String::new(),
        Some(entries.len() as u64),
    );
    {
        let mut writer = std::io::BufWriter::new(temp.as_file_mut());
        for (i, entry) in entries.iter().enumerate() {
            if i % 1024 == 0 {
                control.check().map_err(|e| e.to_string())?;
                control.bytes(i as u64);
            }
            writer
                .write_all(&sources[entry.source_idx].get_line(entry.line_idx))
                .map_err(|e| e.to_string())?;
            writer.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
    }
    control.check().map_err(|e| e.to_string())?;
    let (path, temp) = if let Some(output) = args.output {
        temp.persist(&output).map_err(|e| e.to_string())?;
        (output, None)
    } else {
        (temp.path().to_path_buf(), Some(temp))
    };
    let source_line_counts = sources.iter().map(|reader| reader.line_count()).collect();
    let label_col_width = labels.iter().map(String::len).max().unwrap_or(0);
    let merged = crate::shared::MergedState {
        source_tab_ids: Vec::new(),
        source_parsers: parsers,
        source_labels: labels,
        source_line_counts,
        label_col_width,
        stopped: true,
        building: None,
    };
    let provenance = if temp.is_some() {
        "Merged logs".to_owned()
    } else {
        std::fs::canonicalize(&path)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned()
    };
    let manager = runtime::handle().block_on(LogManager::new(db, Some(provenance)));
    let (_, rx) = tokio::sync::watch::channel(());
    let loaded = crate::gui::message::FileLoaded {
        context: None,
        path,
        reader: crate::ingestion::FileReader::from_merged(Arc::new(entries), Arc::new(sources)),
        log_manager: manager,
        watch: crate::shared::watch_state_from_file(rx, String::new()),
    };
    Ok((loaded, temp, merged))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[tokio::test]
    async fn startup_filters_keep_colors_fields_groups_and_dates() {
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let manager = LogManager::new(db, None).await;
        let args = Args::try_parse_from([
            "logana-gui",
            "-i",
            "--bg Red --field level=ERROR --group failures",
            "-o",
            "--ignore-case debug",
            "-t",
            "> 2024-02-21",
        ])
        .unwrap();
        let manager = apply_filters(manager, args, (0, 0, 0)).await.unwrap();
        let filters = manager.get_filters();
        assert_eq!(filters.len(), 3);
        assert!(filters[0].pattern.starts_with(crate::filters::FIELD_PREFIX));
        assert_eq!(filters[0].group.as_deref(), Some("failures"));
        assert_eq!(
            filters[0].color_config.as_ref().unwrap().bg,
            Some(crate::style::Color::Red)
        );
        assert!(filters[1].ignore_case);
        assert!(filters[2].pattern.starts_with(crate::filters::DATE_PREFIX));
    }

    #[test]
    fn interactive_merge_orders_lines_and_keeps_temporary_output_alive() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.log");
        let b = dir.path().join("b.log");
        std::fs::write(
            &a,
            "Jan  1 00:00:01 host tag: a1\nJan  1 00:00:03 host tag: a3\n",
        )
        .unwrap();
        std::fs::write(&b, "Jan  1 00:00:02 host tag: b2\n").unwrap();
        let args = Args {
            files: vec![a.to_string_lossy().into(), b.to_string_lossy().into()],
            merge: true,
            ..Args::default()
        };
        let handle = runtime::handle();
        let _entered = handle.enter();
        let db = Arc::new(handle.block_on(crate::db::Database::in_memory()).unwrap());
        let (control, _) = Control::new();
        let (loaded, temp, _) = build_merge(args.clone(), db.clone(), control).unwrap();
        assert!(temp.as_ref().unwrap().path().exists());
        assert_eq!(loaded.reader.line_count(), 3);
        assert!(String::from_utf8_lossy(&loaded.reader.get_line(1)).contains("b2"));
        assert!(String::from_utf8_lossy(&loaded.reader.get_line(2)).contains("a3"));
        let output = dir.path().join("merged.log");
        let (control, _) = Control::new();
        let (loaded, temp, _) = build_merge(
            Args {
                output: Some(output.clone()),
                ..args.clone()
            },
            db.clone(),
            control,
        )
        .unwrap();
        assert!(temp.is_none());
        assert_eq!(loaded.path, output);
        assert!(std::fs::read_to_string(output).unwrap().contains("b2"));
        let (control, _) = Control::new();
        assert!(
            build_merge(
                Args {
                    output: Some(a.clone()),
                    ..args
                },
                db,
                control
            )
            .err()
            .unwrap()
            .contains("same as an input")
        );
        assert!(std::fs::read_to_string(a).unwrap().contains("a3"));
    }

    #[test]
    fn merge_extracts_nested_compression_into_requested_directory() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("bundle.zip");
        let plain = dir.path().join("plain.log");
        std::fs::write(&plain, b"Jan  1 00:00:02 host tag: plain\n").unwrap();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(b"Jan  1 00:00:01 host tag: nested\n")
            .unwrap();
        let compressed = gzip.finish().unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        zip.start_file("nested.log.gz", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&compressed).unwrap();
        zip.start_file("other.log", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"Jan  1 00:00:03 host tag: other\n").unwrap();
        zip.finish().unwrap();
        let handle = runtime::handle();
        let _entered = handle.enter();
        let db = Arc::new(handle.block_on(crate::db::Database::in_memory()).unwrap());
        let out = dir.path().join("extracted");
        let args = Args {
            files: vec![
                archive.to_string_lossy().into(),
                plain.to_string_lossy().into(),
            ],
            merge: true,
            out: Some(out.clone()),
            ..Args::default()
        };
        let (control, _) = Control::new();
        let (loaded, _temp, merged) = build_merge(args, db, control).unwrap();
        assert_eq!(loaded.reader.line_count(), 3);
        assert_eq!(merged.source_labels.len(), 3);
        assert!(String::from_utf8_lossy(&loaded.reader.get_line(0)).contains("nested"));
        assert!(String::from_utf8_lossy(&loaded.reader.get_line(1)).contains("plain"));
        assert_eq!(
            std::fs::read(out.join(".logana-merge/bundle.zip/nested.log")).unwrap(),
            b"Jan  1 00:00:01 host tag: nested\n"
        );
        assert!(!out.join(".logana-merge/bundle.zip/nested.log.gz").exists());
        assert!(zip::ZipArchive::new(std::fs::File::open(archive).unwrap()).is_ok());
    }

    #[test]
    fn directory_scan_and_extraction_obey_cancel_and_destination() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b.log"), "b\n").unwrap();
        std::fs::write(dir.path().join("a.log"), "a\n").unwrap();
        let (control, _) = Control::new();
        let mut paths = Vec::new();
        collect_directory(dir.path(), &control, &mut paths).unwrap();
        assert_eq!(
            paths,
            vec![dir.path().join("a.log"), dir.path().join("sub/b.log")]
        );
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        temp.write_all(b"extracted\n").unwrap();
        let path = persist_extracted(temp, dir.path(), "nested/entry.log", &control).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"extracted\n");
        assert!(
            persist_extracted(
                tempfile::NamedTempFile::new().unwrap(),
                dir.path(),
                "../escape",
                &control
            )
            .is_err()
        );
        control
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(collect_directory(dir.path(), &control, &mut Vec::new()).is_err());
        assert!(
            persist_extracted(
                tempfile::NamedTempFile::new().unwrap(),
                dir.path(),
                "canceled.log",
                &control
            )
            .is_err()
        );
        assert!(!dir.path().join("canceled.log").exists());
    }
}
