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

pub(super) struct SavedOutput {
    pub path: PathBuf,
    pub temp: Option<tempfile::NamedTempFile>,
    pub warning: Option<String>,
    pub source: Option<String>,
}
impl SavedOutput {
    fn file(path: PathBuf) -> Self {
        Self {
            path,
            temp: None,
            warning: None,
            source: None,
        }
    }
}

pub(super) fn save_or_temporary(
    temp: tempfile::NamedTempFile,
    out: &Path,
    relative: &str,
    control: &Control,
) -> Result<SavedOutput, String> {
    match save_extracted_unique(&temp, out, relative, control) {
        Ok(path) => Ok(SavedOutput::file(path)),
        Err(error) => {
            control.check().map_err(|e| e.to_string())?;
            let destination = out.join(relative);
            Ok(SavedOutput {
                path: temp.path().to_path_buf(),
                temp: Some(temp),
                warning: Some(format!(
                    "Could not save {}: {error}. Opened as a temporary file.",
                    destination.display()
                )),
                source: Some(format!("{} (temporary)", destination.display())),
            })
        }
    }
}

pub(super) fn persist_extracted_unique(
    temp: tempfile::NamedTempFile,
    out: &Path,
    relative: &str,
    control: &Control,
) -> Result<PathBuf, String> {
    save_extracted_unique(&temp, out, relative, control)
}

fn save_extracted_unique(
    temp: &tempfile::NamedTempFile,
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
    let original = destination.clone();
    let mut number = 1;
    loop {
        control.check().map_err(|e| e.to_string())?;
        match staged.persist_noclobber(&destination) {
            Ok(_) => return Ok(destination),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                staged = error.file;
                number += 1;
                let mut name = original.file_stem().unwrap_or_default().to_os_string();
                name.push(format!(" ({number})"));
                if let Some(extension) = original.extension() {
                    name.push(".");
                    name.push(extension);
                }
                destination = original.with_file_name(name);
            }
            Err(error) => return Err(error.to_string()),
        }
    }
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
        preferences.apply_font_size(window, cx);
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
            extraction_confirmed: false,
            args,
            preview: None,
        });
        (id, control)
    }

    fn directory_remaining(
        &mut self,
        children: &std::collections::HashSet<u64>,
        canceled: bool,
        cx: &mut Context<Self>,
    ) -> usize {
        if canceled {
            let pending: Vec<_> = self
                .loading
                .operations
                .iter()
                .filter(|op| children.contains(&op.id))
                .map(|op| op.id)
                .collect();
            for child in pending {
                self.cancel_loading(child, cx);
            }
        }
        self.loading
            .operations
            .iter()
            .filter(|op| children.contains(&op.id))
            .count()
    }

    pub(super) fn queue_rotation_selection(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let (id, control) = self.cli_operation(PathBuf::from("Selected files"), None);
        let gate = self.loading.gate.clone();
        cx.spawn(async move |this, cx| {
            let worker = control.clone();
            let result = runtime::handle()
                .spawn(async move {
                    let _permit = permit(gate, &worker).await?;
                    tokio::task::spawn_blocking(move || prepare_rotation_selection(paths, &worker))
                        .await
                        .map_err(|e| e.to_string())?
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            let result = async {
                let outputs = result?;
                let mut warnings = Vec::new();
                let mut children = std::collections::HashSet::new();
                for output in outputs {
                    control.check().map_err(|e| e.to_string())?;
                    if let Some(warning) = output.warning {
                        warnings.push(warning);
                    }
                    if output.temp.is_some() {
                        let db = this
                            .update(cx, |app, _| app.state.db.clone())
                            .map_err(|e| e.to_string())?;
                        let worker = control.clone();
                        let loaded = runtime::handle()
                            .spawn(async move {
                                crate::gui::update::load_file_controlled(
                                    output.path,
                                    db,
                                    worker,
                                    output.source,
                                )
                                .await
                            })
                            .await
                            .map_err(|e| e.to_string())?;
                        install_loaded(
                            &this,
                            cx,
                            loaded,
                            control.clone(),
                            Some(id),
                            false,
                            false,
                            output.temp,
                            None,
                        )
                        .await?;
                    } else {
                        let child = this
                            .update(cx, |app, cx| {
                                app.queue_file_with_args(output.path, None, cx);
                                app.loading.next_id
                            })
                            .map_err(|e| e.to_string())?;
                        children.insert(child);
                    }
                }
                loop {
                    let remaining = this
                        .update(cx, |app, cx| {
                            app.directory_remaining(&children, control.cancelled(), cx)
                        })
                        .map_err(|e| e.to_string())?;
                    if remaining == 0 {
                        break;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(100))
                        .await;
                }
                if warnings.is_empty() {
                    Ok(())
                } else {
                    Err(warnings.join("\n"))
                }
            }
            .await;
            this.update(cx, |app, cx| app.finish_loading(id, result, cx))
                .ok();
        })
        .detach();
        cx.notify();
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
                        let errors = collect_directory(&path, &worker, &mut paths)?;
                        if paths.is_empty() && errors.is_empty() {
                            return Err(format!("'{}' contains no files.", path.display()));
                        }
                        Ok((paths, errors))
                    })
                    .await
                    .map_err(|e| e.to_string())?
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            match result {
                Ok((paths, errors)) => {
                    control.phase("Opening directory", String::new(), None);
                    control
                        .progress
                        .send_modify(|p| p.total_files = paths.len());
                    let mut children = std::collections::HashSet::new();
                    for batch in paths.chunks(32) {
                        if control.cancelled() {
                            break;
                        }
                        let queued = this.update(cx, |app, cx| {
                            let mut queued = Vec::new();
                            for path in batch {
                                if control.cancelled() {
                                    break;
                                }
                                app.queue_file_with_args(path.clone(), args.clone(), cx);
                                queued.push(app.loading.next_id);
                            }
                            queued
                        });
                        let Ok(queued) = queued else { return };
                        children.extend(queued);
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(10))
                            .await;
                    }
                    // Keep the directory's cancel button until all its requests finish.
                    loop {
                        let remaining = this.update(cx, |app, cx| {
                            app.directory_remaining(&children, control.cancelled(), cx)
                        });
                        let Ok(remaining) = remaining else { return };
                        control
                            .progress
                            .send_modify(|p| p.completed = children.len() - remaining);
                        if remaining == 0 {
                            break;
                        }
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(100))
                            .await;
                    }
                    let result = if errors.is_empty() {
                        Ok(())
                    } else {
                        Err(errors.join("\n"))
                    };
                    this.update(cx, |app, cx| app.finish_loading(id, result, cx))
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
) -> Result<Vec<String>, String> {
    control.check().map_err(|e| e.to_string())?;
    let mut errors = Vec::new();
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        control.check().map_err(|e| e.to_string())?;
        match entry {
            Ok(entry) => entries.push(entry),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        control.check().map_err(|e| e.to_string())?;
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                errors.push(format!("{}: {error}", entry.path().display()));
                continue;
            }
        };
        if kind.is_dir() {
            match collect_directory(&entry.path(), control, files) {
                Ok(nested_errors) => errors.extend(nested_errors),
                Err(error) => {
                    control.check().map_err(|e| e.to_string())?;
                    errors.push(error);
                }
            }
        } else if kind.is_file() {
            files.push(entry.path());
            control.bytes(files.len() as u64);
        }
    }
    Ok(errors)
}

fn prepare_rotation_selection(
    selected: Vec<PathBuf>,
    control: &Control,
) -> Result<Vec<SavedOutput>, String> {
    use std::collections::{BTreeMap, HashSet};
    use std::io::Read;
    let mut paths = Vec::new();
    control.phase("Reading selected files", String::new(), None);
    for path in selected {
        control.check().map_err(|e| e.to_string())?;
        if path.is_dir() {
            let errors = collect_directory(&path, control, &mut paths)?;
            if !errors.is_empty() {
                return Err(errors.join("\n"));
            }
        } else {
            paths.push(path);
        }
    }
    let mut seen = HashSet::new();
    let mut groups = BTreeMap::<PathBuf, Vec<(u64, PathBuf)>>::new();
    let mut outputs = Vec::new();
    for path in paths {
        let path = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if !seen.insert(path.clone()) {
            continue;
        }
        let kind = crate::ingestion::detect_archive_type(&path.to_string_lossy());
        if kind.as_ref().is_some_and(|kind| {
            !matches!(
                kind,
                crate::ingestion::ArchiveType::Gz
                    | crate::ingestion::ArchiveType::Bz2
                    | crate::ingestion::ArchiveType::Xz
            )
        }) {
            outputs.push(SavedOutput::file(path));
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy();
        if let Some((base, rank)) = crate::shared::archive_selection::rotation_name(&name) {
            groups
                .entry(path.parent().unwrap().join(base))
                .or_default()
                .push((rank, path));
        } else {
            outputs.push(SavedOutput::file(path));
        }
    }
    for (base, mut members) in groups {
        if members.len() < 2 || !members.iter().any(|(rank, _)| *rank > 0) {
            outputs.extend(members.into_iter().map(|(_, path)| SavedOutput::file(path)));
            continue;
        }
        members.sort_by_key(|(rank, path)| (std::cmp::Reverse(*rank), path.clone()));
        control.phase("Merging rotated logs", base.display().to_string(), None);
        let mut merged = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let mut last = None;
        for (_, path) in members {
            control.check().map_err(|e| e.to_string())?;
            let decoded = if let Some(kind) =
                crate::ingestion::detect_archive_type(&path.to_string_lossy())
            {
                Some(crate::ingestion::archive_job::unpack(
                    &path, &kind, "", control,
                )?)
            } else {
                None
            };
            let mut input =
                std::fs::File::open(decoded.as_ref().map(|f| f.path()).unwrap_or(&path))
                    .map_err(|e| e.to_string())?;
            let mut buffer = [0; 64 * 1024];
            let mut first = true;
            loop {
                control.check().map_err(|e| e.to_string())?;
                let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                if first && last.is_some_and(|b| b != b'\n') {
                    merged.write_all(b"\n").map_err(|e| e.to_string())?;
                }
                merged
                    .write_all(&buffer[..count])
                    .map_err(|e| e.to_string())?;
                last = Some(buffer[count - 1]);
                first = false;
            }
        }
        let mut name = base.file_stem().unwrap().to_os_string();
        name.push("-merged");
        if let Some(ext) = base.extension() {
            name.push(".");
            name.push(ext);
        }
        outputs.push(save_or_temporary(
            merged,
            base.parent().unwrap(),
            &name.to_string_lossy(),
            control,
        )?);
    }
    Ok(outputs)
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
    let mut requested = Vec::new();
    for path in &args.files {
        let path = Path::new(path);
        if path.is_dir() {
            let errors = collect_directory(path, &control, &mut requested)?;
            if !errors.is_empty() {
                return Err(errors.join("\n"));
            }
        } else {
            requested.push(path.to_path_buf());
        }
    }
    requested.retain(|path| {
        !path
            .components()
            .any(|part| part.as_os_str() == ".logana-merge")
    });
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    for path in requested {
        control.check().map_err(|e| e.to_string())?;
        let canonical =
            std::fs::canonicalize(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !seen.insert(canonical) {
            continue;
        }
        if crate::ingestion::detect_archive_type(&path.to_string_lossy()).is_some() {
            let tree = crate::ingestion::archive_tree::list_archive_tree_controlled(
                &path.to_string_lossy(),
                control.clone(),
            )?;
            let mut tree = crate::ingestion::archive_job::expand_all(&path, tree, control.clone())?;
            tree.set_all_files_selected(true);
            crate::ingestion::archive_job::extract_selection(
                &path,
                &tree,
                &control,
                &mut |entry| {
                    let (node, temp) = entry?;
                    let relative = crate::gui::loading::path_for_archive_output(
                        &path,
                        &crate::shared::archive_selection::output_entry_path(&tree, node),
                    );
                    paths.push(persist_extracted_unique(
                        temp,
                        &out.join(".logana-merge"),
                        &relative,
                        &control,
                    )?);
                    Ok(())
                },
            )?;
        } else {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return Err("The selection contains no files to merge.".into());
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
    fn failed_save_preserves_temporary_bytes_and_reports_destination() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"keep").unwrap();
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        temp.write_all(b"dummy contents\n").unwrap();
        let (control, _) = Control::new();
        let saved = save_or_temporary(temp, &blocked, "runtime-merged.log", &control).unwrap();
        assert!(saved.temp.is_some());
        assert_eq!(std::fs::read(&saved.path).unwrap(), b"dummy contents\n");
        assert!(
            saved
                .warning
                .as_ref()
                .unwrap()
                .contains("runtime-merged.log")
        );
        assert!(
            saved
                .source
                .as_ref()
                .unwrap()
                .ends_with("runtime-merged.log (temporary)")
        );
        assert_eq!(std::fs::read(&blocked).unwrap(), b"keep");
        let path = saved.path.clone();
        drop(saved);
        assert!(!path.exists());
        control
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            save_or_temporary(
                tempfile::NamedTempFile::new().unwrap(),
                &blocked,
                "cancel.log",
                &control
            )
            .is_err()
        );
    }

    #[test]
    #[cfg(unix)]
    fn rotation_output_that_cannot_be_saved_remains_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("{}.log", "x".repeat(245));
        std::fs::write(dir.path().join(&name), b"current\n").unwrap();
        std::fs::write(dir.path().join(format!("{name}1")), b"previous\n").unwrap();
        let (control, _) = Control::new();
        let outputs = prepare_rotation_selection(vec![dir.path().into()], &control).unwrap();
        assert_eq!(outputs.len(), 1);
        assert!(outputs[0].temp.is_some());
        assert!(
            outputs[0]
                .warning
                .as_ref()
                .unwrap()
                .contains("Could not save")
        );
        assert_eq!(
            std::fs::read(&outputs[0].path).unwrap(),
            b"previous\ncurrent\n"
        );
    }

    #[test]
    fn selection_merges_only_numbered_families_in_the_same_folder() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        for (name, content) in [
            ("runtime.log", "current\n"),
            ("runtime.log2", "older"),
            ("startup.log", "startup\n"),
            ("startup.log1", "previous\n"),
            ("other.log", "unrelated\n"),
            ("nested/runtime.log", "separate\n"),
        ] {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(b"oldest\n").unwrap();
        std::fs::write(dir.path().join("runtime.log10.gz"), gzip.finish().unwrap()).unwrap();
        let (control, _) = Control::new();
        let outputs = prepare_rotation_selection(
            vec![dir.path().into(), dir.path().join("runtime.log")],
            &control,
        )
        .unwrap();
        assert_eq!(outputs.len(), 4);
        assert_eq!(
            std::fs::read(dir.path().join("runtime-merged.log")).unwrap(),
            b"oldest\nolder\ncurrent\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("startup-merged.log")).unwrap(),
            b"previous\nstartup\n"
        );
        assert!(outputs.iter().any(|o| o.path == nested.join("runtime.log")));
        assert!(
            outputs
                .iter()
                .any(|o| o.path == dir.path().join("other.log"))
        );
        assert!(
            !outputs
                .iter()
                .any(|o| o.path == dir.path().join("runtime.log"))
        );
        assert_eq!(
            std::fs::read(dir.path().join("runtime.log2")).unwrap(),
            b"older"
        );
        control
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(prepare_rotation_selection(vec![dir.path().into()], &control).is_err());
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
                dir.path().to_string_lossy().into(),
                plain.to_string_lossy().into(),
            ],
            merge: true,
            out: Some(out.clone()),
            ..Args::default()
        };
        let (control, _) = Control::new();
        let (loaded, _temp, merged) = build_merge(args.clone(), db.clone(), control).unwrap();
        let (control, _) = Control::new();
        let (repeated, _, _) = build_merge(args, db, control).unwrap();
        assert_eq!(repeated.reader.line_count(), 3);
        assert_eq!(loaded.reader.line_count(), 3);
        assert_eq!(merged.source_labels.len(), 3);
        assert!(String::from_utf8_lossy(&loaded.reader.get_line(0)).contains("nested"));
        assert!(String::from_utf8_lossy(&loaded.reader.get_line(1)).contains("plain"));
        assert_eq!(
            std::fs::read(out.join(".logana-merge/bundle/nested.log")).unwrap(),
            b"Jan  1 00:00:01 host tag: nested\n"
        );
        assert!(!out.join(".logana-merge/bundle/nested.log.gz").exists());
        assert!(zip::ZipArchive::new(std::fs::File::open(archive).unwrap()).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn directory_scan_continues_after_unreadable_subdirectory() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let unreadable = dir.path().join("a-unreadable");
        std::fs::create_dir(&unreadable).unwrap();
        std::fs::write(unreadable.join("hidden.log"), "hidden\n").unwrap();
        let good = dir.path().join("z-good.log");
        std::fs::write(&good, "good\n").unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o0)).unwrap();
        let (control, _) = Control::new();
        let mut paths = Vec::new();
        let result = collect_directory(dir.path(), &control, &mut paths);
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let errors = result.unwrap();
        assert_eq!(paths, vec![good]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("a-unreadable"));
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
        let path =
            persist_extracted_unique(temp, dir.path(), "nested/entry.log", &control).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"extracted\n");
        assert!(
            persist_extracted_unique(
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
            persist_extracted_unique(
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

#[cfg(all(test, feature = "gui-test"))]
mod directory_ui_tests {
    use super::*;
    use gpui_kit::gpui::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn directory_cancel_stops_children_and_keeps_unrelated_operation(cx: &mut TestAppContext) {
        let db = Arc::new(
            runtime::handle()
                .block_on(crate::db::Database::in_memory())
                .unwrap(),
        );
        let theme = crate::theme::Theme::default();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::gui::theme::install(&theme, cx);
        });
        let app = cx.new(|cx| App::new(db, theme, cx));
        app.update(cx, |app, cx| {
            let (parent, _) = app.cli_operation(PathBuf::from("directory"), None);
            let (first, first_control) = app.cli_operation(PathBuf::from("a.log"), None);
            let (second, second_control) = app.cli_operation(PathBuf::from("b.zip"), None);
            let (_, unrelated) = app.cli_operation(PathBuf::from("unrelated.log"), None);
            let children = [first, second].into_iter().collect();
            assert_eq!(app.directory_remaining(&children, false, cx), 2);
            assert!(!first_control.cancelled());
            app.cancel_loading(parent, cx);
            assert_eq!(app.directory_remaining(&children, true, cx), 2);
            assert!(first_control.cancelled());
            assert!(second_control.cancelled());
            assert!(!unrelated.cancelled());
            app.finish_loading(first, Ok(()), cx);
            app.finish_loading(second, Ok(()), cx);
            assert_eq!(app.directory_remaining(&children, true, cx), 0);
            app.finish_loading(parent, Ok(()), cx);
            assert_eq!(app.loading.operations.len(), 1);
        });
    }
}
