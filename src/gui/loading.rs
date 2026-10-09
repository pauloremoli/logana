//! GUI ingestion ownership: one heavy worker, independently cancelable requests.
use crate::{
    gui::{
        app::App,
        message::{FileLoaded, Message},
        runtime,
        state::StatusMessage,
    },
    ingestion::{
        archive_job::{Control, Progress},
        archive_tree::{ArchiveTree, CheckState, NodeId, NodeKind},
    },
};
use gpui_kit::gpui::{AsyncApp, Context, WeakEntity};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, watch};

pub(super) struct Operation {
    pub id: u64,
    pub path: PathBuf,
    pub control: Control,
    pub(super) rx: watch::Receiver<Progress>,
    pub progress: Progress,
    pub picker: Option<Picker>,
    pub args: Option<crate::cli::Args>,
    pub preview: Option<Preview>,
}
#[derive(Clone)]
pub(super) struct Preview {
    tab: crate::shared::TabId,
    initial: Option<crate::db::FileContext>,
    restored: Option<crate::db::FileContext>,
}
pub(super) struct Picker {
    pub tree: ArchiveTree,
    pub rows: Vec<NodeId>,
    pub checks: Vec<CheckState>,
    pub cursor: usize,
    pub sizes: std::rc::Rc<Vec<gpui_kit::gpui::Size<gpui_kit::gpui::Pixels>>>,
}
impl Picker {
    pub fn new(tree: ArchiveTree) -> Self {
        let rows = tree.visible_rows();
        let checks = tree.check_states();
        let sizes = std::rc::Rc::new(vec![
            gpui_kit::gpui::size(
                gpui_kit::gpui::px(400.),
                gpui_kit::gpui::px(32.)
            );
            rows.len()
        ]);
        Self {
            tree,
            rows,
            checks,
            sizes,
            cursor: 0,
        }
    }
    fn refresh(&mut self) {
        self.rows = self.tree.visible_rows();
        self.checks = self.tree.check_states();
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.sizes = std::rc::Rc::new(vec![
            gpui_kit::gpui::size(
                gpui_kit::gpui::px(400.),
                gpui_kit::gpui::px(32.)
            );
            self.rows.len()
        ]);
    }
}
pub(super) struct Loading {
    pub operations: Vec<Operation>,
    pub selected_picker: Option<u64>,
    pub scroll: gpui_kit::base::VirtualListScrollHandle,
    pub focus: gpui_kit::gpui::FocusHandle,
    pub(super) next_id: u64,
    pub(super) gate: Arc<Semaphore>,
    last_poll: Instant,
}
impl Loading {
    pub fn new(cx: &mut Context<App>) -> Self {
        Self {
            focus: cx.focus_handle(),
            operations: Vec::new(),
            selected_picker: None,
            next_id: 0,
            gate: Arc::new(Semaphore::new(1)),
            last_poll: Instant::now(),
            scroll: gpui_kit::base::VirtualListScrollHandle::new(),
        }
    }
}
impl Drop for Loading {
    fn drop(&mut self) {
        for op in &self.operations {
            op.control.cancel.store(true, Ordering::Relaxed);
        }
    }
}
pub(super) async fn permit(
    gate: Arc<Semaphore>,
    control: &Control,
) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    let acquire = gate.acquire_owned();
    tokio::pin!(acquire);
    loop {
        control.check().map_err(|e| e.to_string())?;
        tokio::select! {
            permit = &mut acquire => return permit.map_err(|e| e.to_string()),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}
fn current(app: &App, id: Option<u64>, control: &Control) -> bool {
    !control.cancelled() && id.is_none_or(|id| app.loading.operations.iter().any(|op| op.id == id))
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn install_loaded(
    this: &WeakEntity<App>,
    cx: &mut AsyncApp,
    result: Result<FileLoaded, String>,
    control: Control,
    id: Option<u64>,
    activate: bool,
    is_preview: bool,
    temp: Option<tempfile::NamedTempFile>,
    merged: Option<crate::shared::MergedState>,
) -> Result<(), String> {
    control.check().map_err(|e| e.to_string())?;
    let preferences = this
        .update(cx, |app, _| app.state.preferences.clone())
        .map_err(|e| e.to_string())?;
    let active_before_prepare = this
        .update(cx, |app, _| app.state.active_tab().map(|tab| tab.id))
        .map_err(|e| e.to_string())?;
    let mut loaded = result?;
    let preview = this
        .update(cx, |app, _| {
            id.and_then(|id| app.loading.operations.iter().find(|op| op.id == id))
                .and_then(|op| op.preview.clone())
        })
        .map_err(|e| e.to_string())?;
    if let Some(preview) = &preview {
        let snapshot = this
            .update(cx, |app, _| {
                app.state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == preview.tab)
                    .map(|tab| {
                        let current = tab.to_file_context();
                        let context = if current == preview.initial {
                            preview.restored.clone().or(current)
                        } else {
                            current
                        };
                        (tab.log_manager.clone(), context)
                    })
            })
            .map_err(|e| e.to_string())?;
        let Some((manager, context)) = snapshot else {
            return Ok(());
        };
        loaded.log_manager = manager;
        loaded.context = context;
    }
    let options = this
        .update(cx, |app, _| {
            id.and_then(|id| app.loading.operations.iter().find(|op| op.id == id))
                .and_then(|op| op.args.clone())
                .map(|args| {
                    (
                        args,
                        crate::gui::theme::GuiPalette::from_theme(&app.state.theme).background,
                    )
                })
        })
        .map_err(|e| e.to_string())?;
    if let Some((args, background)) = &options
        && preview.is_none()
    {
        loaded.log_manager = runtime::handle()
            .spawn(crate::gui::startup::apply_filters(
                loaded.log_manager,
                args.clone(),
                *background,
            ))
            .await
            .map_err(|e| e.to_string())??;
        if crate::gui::startup::has_filters(args) {
            loaded.context = None;
        }
    }
    let restored = loaded.context.clone();
    let provenance = temp
        .as_ref()
        .and_then(|_| loaded.log_manager.source_file().map(str::to_owned));
    control.phase("Preparing display", loaded.path.display().to_string(), None);
    let worker_control = control.clone();
    let result = runtime::handle()
        .spawn_blocking(move || {
            let handle = runtime::handle();
            let _guard = handle.enter();
            let (mut tab, widths) = crate::gui::update::prepare_tab_with_merge(
                loaded,
                &preferences,
                &worker_control.cancel,
                merged,
            )?;
            if let Some(provenance) = provenance {
                tab.source_path = Some(provenance.clone());
                tab.title = provenance.rsplit('/').next().unwrap_or(&provenance).into();
                tab.stream.watch = None;
            }
            if options.as_ref().is_some_and(|(args, _)| args.tail) {
                tab.stream.tail_mode = true;
                tab.scroll.scroll_offset = tab.filter.visible_indices.len().saturating_sub(1);
                tab.scroll.viewport_offset = tab.scroll.scroll_offset;
            }
            if is_preview {
                tab.stream.watch = None;
            }
            // Ownership keeps completed extracted bytes alive for the tab's lifetime.
            tab.merged_temp = temp;
            worker_control.check().map_err(|e| e.to_string())?;
            Ok::<_, String>((tab, widths))
        })
        .await
        .map_err(|e| e.to_string())??;
    this.update_in(cx, |app, window, cx| {
        if current(app, id, &control) {
            if let Some(preview) = &preview {
                expand_preview(&mut app.state, preview, result.0, result.1);
                if app
                    .state
                    .tabs
                    .iter()
                    .any(|tab| tab.id == preview.tab && tab.filter.handle.is_some())
                {
                    app.context_scroll_pending.insert(preview.tab);
                }
                if let Some(op) =
                    id.and_then(|id| app.loading.operations.iter_mut().find(|op| op.id == id))
                {
                    op.preview = None;
                }
                app.save_session_state();
                cx.notify();
            } else {
                if is_preview
                    && let Some(op) =
                        id.and_then(|id| app.loading.operations.iter_mut().find(|op| op.id == id))
                {
                    op.preview = Some(Preview {
                        tab: result.0.id,
                        initial: result.0.to_file_context(),
                        restored,
                    });
                    op.control.phase(
                        "Loading",
                        op.path.display().to_string(),
                        std::fs::metadata(&op.path).ok().map(|m| m.len()),
                    );
                }
                app.dispatch(
                    Message::TabPrepared(
                        Box::new(result.0),
                        result.1,
                        activate
                            && app.state.active_tab().map(|tab| tab.id) == active_before_prepare,
                    ),
                    window,
                    cx,
                );
            }
        }
    })
    .map_err(|e| e.to_string())?;
    control.check().map_err(|e| e.to_string())
}

fn expand_preview(
    state: &mut crate::gui::state::GuiState,
    preview: &Preview,
    mut full: crate::shared::TabState,
    widths: crate::gui::column_widths::ColumnWidths,
) {
    let Some(index) = state.tabs.iter().position(|tab| tab.id == preview.tab) else {
        return;
    };
    let tab = &mut state.tabs[index];
    let unchanged = tab.to_file_context() == preview.initial;
    let anchor = if unchanged && preview.restored.is_some() {
        preview
            .restored
            .as_ref()
            .map(|context| context.scroll_offset)
    } else {
        tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset)
    };
    let refresh = tab.log_manager.get_filters() != full.log_manager.get_filters()
        || tab.log_manager.get_group_styles() != full.log_manager.get_group_styles()
        || tab.display.raw_mode != full.display.raw_mode
        || tab.filter.enabled != full.filter.enabled
        || tab.filter.show_marks_only
        || tab.filter.highlight_mode;
    if let Some(handle) = tab.filter.handle.take() {
        handle.cancel.store(true, Ordering::Relaxed);
    }
    tab.file_reader = full.file_reader;
    tab.display.format = full.display.format;
    tab.continuation_map = full.continuation_map;
    tab.year_map = full.year_map;
    tab.stream.watch = full.stream.watch;
    tab.invalidate_parse_cache();
    if full.filter.handle.is_none() {
        tab.filter.visible_indices = full.filter.visible_indices;
    }
    tab.filter.manager = full.filter.manager;
    tab.filter.text_styles = full.filter.text_styles;
    tab.filter.date_styles = full.filter.date_styles;
    tab.filter.field_styles = full.filter.field_styles;
    tab.filter.match_counts = full.filter.match_counts;
    tab.filter.pre_collapse_visible = full.filter.pre_collapse_visible;
    tab.filter.cached_scan = None;
    tab.filter.saved_view = None;
    tab.filter.handle = full.filter.handle.take();
    if refresh {
        tab.begin_filter_refresh();
    }
    if let Some(handle) = &mut tab.filter.handle {
        handle.scroll_anchor = anchor;
    } else {
        tab.restore_scroll_to_line(anchor);
    }
    if let Some(query) = tab.search.query.get_pattern().map(str::to_owned) {
        if let Some(handle) = full.search.handle.take() {
            handle.cancel.store(true, Ordering::Relaxed);
        }
        tab.begin_search(&query, true, false);
    }
    if index == state.active_tab {
        state.column_widths = widths;
    }
}
impl App {
    pub(super) fn picker_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.loading.selected_picker else {
            return false;
        };
        let Some(picker) = self
            .loading
            .operations
            .iter_mut()
            .find(|op| op.id == id)
            .and_then(|op| op.picker.as_mut())
        else {
            return false;
        };
        let node = picker.rows.get(picker.cursor).copied();
        match key {
            "up" => picker.cursor = picker.cursor.saturating_sub(1),
            "down" => picker.cursor = (picker.cursor + 1).min(picker.rows.len().saturating_sub(1)),
            "home" => picker.cursor = 0,
            "end" => picker.cursor = picker.rows.len().saturating_sub(1),
            "space" => {
                if let Some(node) = node {
                    self.picker_toggle(id, node, cx);
                }
                return true;
            }
            "right" | "left" => {
                if let Some(node) = node {
                    let item = &picker.tree.nodes[node];
                    let expand = key == "right"
                        && (item.collapsed || matches!(item.kind, NodeKind::LazyContainer { .. }));
                    let collapse = key == "left"
                        && !item.collapsed
                        && matches!(item.kind, NodeKind::Container { .. });
                    if expand || collapse {
                        self.picker_expand(id, node, cx);
                    }
                }
                return true;
            }
            "enter" => {
                self.picker_open(id, cx);
                return true;
            }
            "escape" => {
                self.cancel_loading(id, cx);
                return true;
            }
            _ => return false,
        }
        self.loading
            .scroll
            .scroll_to_item(picker.cursor, gpui_kit::gpui::ScrollStrategy::Nearest);
        cx.notify();
        true
    }
    pub(super) fn poll_loading(&mut self, cx: &mut Context<Self>) {
        if self.loading.last_poll.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.loading.last_poll = Instant::now();
        let mut changed = false;
        for op in &mut self.loading.operations {
            if op
                .preview
                .as_ref()
                .is_some_and(|preview| !self.state.tabs.iter().any(|tab| tab.id == preview.tab))
            {
                op.control.cancel.store(true, Ordering::Relaxed);
            }
            if op.rx.has_changed().unwrap_or(false) {
                op.progress = op.rx.borrow_and_update().clone();
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }
    pub(super) fn finish_loading(
        &mut self,
        id: u64,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.loading.operations.iter().position(|op| op.id == id) {
            let op = self.loading.operations.remove(index);
            // A preview is not completed output; discard it on cancellation/failure.
            if let Some(preview) = op.preview
                && let Some(index) = self.state.tabs.iter().position(|tab| tab.id == preview.tab)
            {
                crate::gui::update::update(&mut self.state, Message::TabClosed(index));
            }
            self.state.notify(if op.control.cancelled() {
                StatusMessage::Info("Operation canceled. Completed files kept.".into())
            } else {
                match result {
                    Ok(()) => StatusMessage::Info("Files opened.".into()),
                    Err(error) => StatusMessage::Error(error),
                }
            });
            if self.loading.selected_picker == Some(id) {
                self.loading.selected_picker = None;
            }
            if self.loading.operations.is_empty() {
                self.persistence.restoring = false;
            }
            self.save_session_state();
            self.sync_notification_timer(cx);
            cx.notify();
        }
    }
    pub(super) fn cancel_loading(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(op) = self.loading.operations.iter_mut().find(|op| op.id == id) {
            op.control.cancel.store(true, Ordering::Relaxed);
            op.progress.phase = "Canceling";
            op.control.phase("Canceling", String::new(), None);
            if op.picker.take().is_some() {
                self.finish_loading(id, Ok(()), cx);
            }
            cx.notify();
        }
    }
    /// Queue a file, archive, or directory for background loading.
    pub fn queue_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if path.is_dir() {
            self.queue_directory(path, None, cx);
        } else {
            self.queue_file_with_args(path, None, cx);
        }
    }
    pub(super) fn queue_file_with_args(
        &mut self,
        path: PathBuf,
        args: Option<crate::cli::Args>,
        cx: &mut Context<Self>,
    ) {
        let preview_enabled = !args.as_ref().is_some_and(|args| args.tail);
        let preview_bytes = self.state.preferences.preview_bytes;
        self.loading.next_id += 1;
        let id = self.loading.next_id;
        let (control, rx) = Control::new();
        self.loading.operations.push(Operation {
            id,
            path: path.clone(),
            control: control.clone(),
            rx,
            progress: Progress::default(),
            picker: None,
            args,
            preview: None,
        });
        let gate = self.loading.gate.clone();
        let db = self.state.db.clone();
        let origin = self.state.active_tab().map(|tab| tab.id);
        cx.spawn(async move |this, cx| {
            let worker_control = control.clone();
            let (preview_tx, preview_rx) = tokio::sync::oneshot::channel();
            let worker = runtime::handle().spawn(async move {
                let _permit = permit(gate, &worker_control).await?;
                worker_control.check().map_err(|e| e.to_string())?;
                if crate::ingestion::detect_archive_type(&path.to_string_lossy()).is_some() {
                    worker_control.phase("Reading archive", path.display().to_string(), None);
                    let control = worker_control.clone();
                    let tree = tokio::task::spawn_blocking(move || {
                        crate::ingestion::archive_tree::list_archive_tree_controlled(
                            &path.to_string_lossy(),
                            control,
                        )
                    })
                    .await
                    .map_err(|e| e.to_string())??;
                    Ok::<_, String>((Some(tree), None, Some(_permit)))
                } else {
                    let preview = if preview_enabled {
                        crate::gui::update::load_file_preview(
                            &path,
                            db.clone(),
                            &worker_control,
                            preview_bytes,
                        )
                        .await
                        .unwrap_or(None)
                    } else {
                        None
                    };
                    let _ = preview_tx.send(preview);
                    let loaded =
                        crate::gui::update::load_file_controlled(path, db, worker_control, None)
                            .await?;
                    Ok((None, Some(loaded), Some(_permit)))
                }
            });
            if let Ok(Some(mut preview)) = preview_rx.await {
                let startup_filters = this
                    .update(cx, |app, _| {
                        app.loading
                            .operations
                            .iter()
                            .find(|op| op.id == id)
                            .and_then(|op| op.args.as_ref())
                            .is_some_and(crate::gui::startup::has_filters)
                    })
                    .unwrap_or(false);
                if startup_filters {
                    preview.context = None;
                }
                let preview = if startup_filters {
                    Ok(preview)
                } else {
                    crate::gui::persistence::prepare_loaded(&this, cx, Ok(preview), &control).await
                };
                if let Ok(preview) = preview {
                    let activate = this
                        .update(cx, |app, _| app.state.active_tab().map(|t| t.id) == origin)
                        .unwrap_or(false);
                    let _ = install_loaded(
                        &this,
                        cx,
                        Ok(preview),
                        control.clone(),
                        Some(id),
                        activate,
                        true,
                        None,
                        None,
                    )
                    .await;
                }
            }
            let result = worker.await.map_err(|e| e.to_string()).and_then(|r| r);
            match result {
                Ok((Some(tree), _, _permit)) => {
                    this.update(cx, |app, cx| {
                        if current(app, Some(id), &control) {
                            if let Some(op) =
                                app.loading.operations.iter_mut().find(|op| op.id == id)
                            {
                                op.picker = Some(Picker::new(tree));
                                op.control.phase("Choose files", String::new(), None);
                                app.loading.selected_picker = Some(id);
                            }
                            cx.notify();
                        } else {
                            app.finish_loading(id, Ok(()), cx);
                        }
                    })
                    .ok();
                }
                Ok((_, Some(loaded), _permit)) => {
                    let startup_filters = this
                        .update(cx, |app, _| {
                            app.loading
                                .operations
                                .iter()
                                .find(|op| op.id == id)
                                .and_then(|op| op.args.as_ref())
                                .is_some_and(crate::gui::startup::has_filters)
                        })
                        .unwrap_or(false);
                    let has_preview = this
                        .update(cx, |app, _| {
                            app.loading
                                .operations
                                .iter()
                                .find(|op| op.id == id)
                                .is_some_and(|op| op.preview.is_some())
                        })
                        .unwrap_or(false);
                    let result = if startup_filters || has_preview {
                        Ok(loaded)
                    } else {
                        crate::gui::persistence::prepare_loaded(&this, cx, Ok(loaded), &control)
                            .await
                    };
                    let activate = this
                        .update(cx, |app, _| {
                            app.state.active_tab().map(|tab| tab.id) == origin
                        })
                        .unwrap_or(false);
                    let result = install_loaded(
                        &this,
                        cx,
                        result,
                        control,
                        Some(id),
                        activate,
                        false,
                        None,
                        None,
                    )
                    .await;
                    this.update(cx, |app, cx| app.finish_loading(id, result, cx))
                        .ok();
                }
                Err(error) => {
                    this.update(cx, |app, cx| app.finish_loading(id, Err(error), cx))
                        .ok();
                }
                _ => {}
            }
        })
        .detach();
        cx.notify();
    }
    pub(super) fn picker_all(&mut self, id: u64, selected: bool, cx: &mut Context<Self>) {
        if let Some(picker) = self
            .loading
            .operations
            .iter_mut()
            .find(|op| op.id == id)
            .and_then(|op| op.picker.as_mut())
        {
            picker.tree.set_all_files_selected(selected);
            picker.refresh();
        }
        cx.notify();
    }
    pub(super) fn picker_toggle(&mut self, id: u64, node: NodeId, cx: &mut Context<Self>) {
        if let Some(picker) = self
            .loading
            .operations
            .iter_mut()
            .find(|op| op.id == id)
            .and_then(|op| op.picker.as_mut())
        {
            picker.tree.toggle_subtree(node);
            picker.refresh();
        }
        cx.notify();
    }
    pub(super) fn picker_expand(&mut self, id: u64, node: NodeId, cx: &mut Context<Self>) {
        let Some(op) = self.loading.operations.iter_mut().find(|op| op.id == id) else {
            return;
        };
        let Some(picker) = op.picker.as_mut() else {
            return;
        };
        if let NodeKind::Container { .. } = &picker.tree.nodes[node].kind {
            picker.tree.nodes[node].collapsed = !picker.tree.nodes[node].collapsed;
            picker.refresh();
            cx.notify();
            return;
        }
        if !matches!(picker.tree.nodes[node].kind, NodeKind::LazyContainer { .. }) {
            return;
        }
        let picker = op.picker.take().unwrap();
        let control = op.control.clone();
        control.phase(
            "Reading nested archive",
            picker.tree.nodes[node].name.clone(),
            None,
        );
        let path = op.path.clone();
        let gate = self.loading.gate.clone();
        cx.spawn(async move |this, cx| {
            let worker_control = control.clone();
            let result = runtime::handle()
                .spawn(async move {
                    let _permit = permit(gate, &worker_control).await?;
                    tokio::task::spawn_blocking(move || {
                        crate::ingestion::archive_job::expand(
                            &path,
                            picker.tree,
                            node,
                            worker_control,
                        )
                    })
                    .await
                    .map_err(|e| e.to_string())?
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            this.update(cx, |app, cx| {
                if current(app, Some(id), &control) {
                    match result {
                        Ok(tree) => {
                            if let Some(op) =
                                app.loading.operations.iter_mut().find(|op| op.id == id)
                            {
                                op.picker = Some(Picker::new(tree));
                                op.control.phase("Choose files", String::new(), None);
                            }
                        }
                        Err(e) => app.finish_loading(id, Err(e), cx),
                    }
                    cx.notify();
                } else {
                    app.finish_loading(id, Ok(()), cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    pub(super) fn picker_open(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(op) = self.loading.operations.iter_mut().find(|op| op.id == id) else {
            return;
        };
        if !op
            .picker
            .as_ref()
            .is_some_and(|p| p.tree.any_file_selected())
        {
            return;
        }
        let picker = op.picker.take().unwrap();
        let args = op.args.clone();
        let control = op.control.clone();
        control.phase("Queued", String::new(), None);
        let path = op.path.clone();
        let db = self.state.db.clone();
        let gate = self.loading.gate.clone();
        cx.spawn(async move |this, cx| {
            let control_for_gate = control.clone();
            let permit = runtime::handle()
                .spawn(async move { permit(gate, &control_for_gate).await })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            let result = async {
                let _permit = permit?;
                let tree = Arc::new(picker.tree);
                let ids: Vec<_> = tree
                    .nodes
                    .iter()
                    .filter(|n| n.selected && matches!(n.kind, NodeKind::File))
                    .map(|n| n.id)
                    .collect();
                control.progress.send_modify(|p| p.total_files = ids.len());
                let (tx, mut rx) = tokio::sync::mpsc::channel(1);
                let worker_tree = tree.clone();
                let worker_control = control.clone();
                let worker_path = path.clone();
                let extraction = runtime::handle().spawn_blocking(move || {
                    crate::ingestion::archive_job::extract_selection(
                        &worker_path,
                        &worker_tree,
                        &worker_control,
                        &mut |entry| {
                            let (ack, accepted) = std::sync::mpsc::sync_channel(1);
                            tx.blocking_send((entry, ack))
                                .map_err(|_| "Operation canceled".to_string())?;
                            loop {
                                worker_control.check().map_err(|e| e.to_string())?;
                                match accepted.recv_timeout(Duration::from_millis(100)) {
                                    Ok(()) => return Ok(()),
                                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                    Err(_) => return Err("Operation canceled".into()),
                                }
                            }
                        },
                    )
                });
                let mut errors = Vec::new();
                while let Some((entry, ack)) = rx.recv().await {
                    let entry_result = async {
                        control.check().map_err(|e| e.to_string())?;
                        let (node, temp) = entry?;
                        let source = provenance(&path, &tree, node);
                        let worker_control = control.clone();
                        let db = db.clone();
                        let (file_path, temp) = if let Some(args) = &args {
                            let out = crate::cli::resolve_out_dir(args.out.as_ref());
                            let relative = crate::ingestion::archive_tree::relative_name(
                                &tree,
                                node,
                                &path.to_string_lossy(),
                            );
                            let save_control = control.clone();
                            let file_path = runtime::handle()
                                .spawn_blocking(move || {
                                    crate::gui::startup::persist_extracted(
                                        temp,
                                        &out,
                                        &relative,
                                        &save_control,
                                    )
                                })
                                .await
                                .map_err(|e| e.to_string())??;
                            (file_path, None)
                        } else {
                            (temp.path().to_path_buf(), Some(temp))
                        };
                        let source = temp.as_ref().map(|_| source);
                        let loaded = runtime::handle()
                            .spawn(async move {
                                crate::gui::update::load_file_controlled(
                                    file_path,
                                    db,
                                    worker_control,
                                    source,
                                )
                                .await
                            })
                            .await
                            .map_err(|e| e.to_string())?;
                        let loaded = if args.as_ref().is_some_and(crate::gui::startup::has_filters)
                        {
                            loaded
                        } else {
                            crate::gui::persistence::prepare_loaded(&this, cx, loaded, &control)
                                .await
                        };
                        install_loaded(
                            &this,
                            cx,
                            loaded,
                            control.clone(),
                            Some(id),
                            false,
                            false,
                            temp,
                            None,
                        )
                        .await
                    }
                    .await;
                    if let Err(error) = entry_result {
                        errors.push(error);
                    }
                    let _ = ack.send(());
                    control.progress.send_modify(|p| p.completed += 1);
                    if control.cancelled() {
                        break;
                    }
                }
                drop(rx);
                let extraction_result = extraction.await.map_err(|e| e.to_string())?;
                control.check().map_err(|e| e.to_string())?;
                if let Err(error) = extraction_result {
                    errors.push(error);
                }
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join("\n"))
                }
            }
            .await;
            this.update(cx, |app, cx| app.finish_loading(id, result, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }
}
pub(super) fn provenance(path: &std::path::Path, tree: &ArchiveTree, id: NodeId) -> String {
    let mut entries = vec![tree.nodes[id].full_path.as_str()];
    let mut parent = tree.nodes[id].parent;
    while let Some(id) = parent {
        entries.push(&tree.nodes[id].full_path);
        parent = tree.nodes[id].parent;
    }
    entries.reverse();
    format!("{}/{}", path.display(), entries.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn first_chunk_is_bounded_complete_and_available_without_full_index() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"first chunk line\n".repeat(128 * 1024)).unwrap();
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let (control, _) = Control::new();
        let start = Instant::now();
        let loaded =
            crate::gui::update::load_file_preview(file.path(), db.clone(), &control, 1024 * 1024)
                .await
                .unwrap()
                .unwrap();
        assert!(loaded.reader.total_len() <= 1024 * 1024);
        assert!(loaded.reader.total_len() < file.as_file().metadata().unwrap().len() as usize);
        assert_eq!(&*loaded.reader.get_line(0), b"first chunk line");
        assert_eq!(
            &*loaded.reader.get_line(loaded.reader.line_count() - 1),
            b"first chunk line"
        );
        eprintln!(
            "first chunk prepared for publication in {:?}",
            start.elapsed()
        );
        control.cancel.store(true, Ordering::Relaxed);
        assert!(
            crate::gui::update::load_file_preview(file.path(), db, &control, 1024 * 1024)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn expanding_preview_keeps_identity_edits_cursor_and_active_tab() {
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let manager = crate::db::LogManager::new(db.clone(), Some("/preview.log".into())).await;
        let mut tab = crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"a\nb\n".to_vec()),
            manager.clone(),
            "preview".into(),
        );
        let preview = Preview {
            tab: tab.id,
            initial: tab.to_file_context(),
            restored: None,
        };
        tab.scroll.scroll_offset = 1;
        tab.mark_manager.mark(1);
        tab.display.wrap = false;
        let mut state = crate::gui::state::GuiState::new(db);
        state.tabs.push(tab);
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"other\n".to_vec()),
            manager.clone(),
            "other".into(),
        ));
        state.active_tab = 1;
        let full = crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"a\nb\nc\n".to_vec()),
            manager,
            "preview".into(),
        );
        expand_preview(&mut state, &preview, full, Default::default());
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_tab, 1);
        assert_eq!(state.tabs[0].id, preview.tab);
        assert_eq!(state.tabs[0].file_reader.line_count(), 3);
        assert_eq!(state.tabs[0].scroll.scroll_offset, 1);
        assert_eq!(state.tabs[0].mark_manager.get_indices(), vec![1]);
        assert!(!state.tabs[0].display.wrap);
        let full = crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"a\nb\nc\n".to_vec()),
            state.tabs[0].log_manager.clone(),
            "preview".into(),
        );
        state.tabs.remove(0);
        state.active_tab = 0;
        expand_preview(&mut state, &preview, full, Default::default());
        assert_eq!(state.tabs.len(), 1, "closed previews must not reappear");
    }

    #[tokio::test]
    async fn filtered_preview_stays_visible_until_full_filter_chunks_arrive() {
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut manager = crate::db::LogManager::new(db.clone(), Some("/preview.log".into())).await;
        manager
            .add_filter_with_color(
                "keep".into(),
                crate::filters::FilterType::Include,
                Default::default(),
            )
            .await;
        let tab = crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"keep\nskip\nkeep\n".to_vec()),
            manager.clone(),
            "preview".into(),
        );
        let preview = Preview {
            tab: tab.id,
            initial: tab.to_file_context(),
            restored: None,
        };
        let mut state = crate::gui::state::GuiState::new(db);
        state.tabs.push(tab);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut full = crate::shared::TabState::new_controlled(
            crate::ingestion::FileReader::from_bytes(b"keep\nskip\nkeep\nskip\nkeep\n".to_vec()),
            manager,
            "preview".into(),
            &cancel,
        )
        .unwrap();
        full.prepare_visible_controlled(&cancel);
        assert!(full.filter.handle.is_some());
        expand_preview(&mut state, &preview, full, Default::default());
        assert_eq!(
            state.tabs[0]
                .filter
                .visible_indices
                .iter()
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while state.tabs[0].filter.handle.is_some() {
                state.tabs[0].advance_filter_computation();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            state.tabs[0]
                .filter
                .visible_indices
                .iter()
                .collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(state.tabs[0].filter.match_counts, vec![3]);
    }

    #[tokio::test]
    async fn saved_cursor_beyond_preview_is_restored_when_full_index_arrives() {
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let manager = crate::db::LogManager::new(db.clone(), Some("/preview.log".into())).await;
        let tab = crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"row\nrow\n".to_vec()),
            manager.clone(),
            "preview".into(),
        );
        let mut restored = tab.to_file_context().unwrap();
        restored.scroll_offset = 8;
        let preview = Preview {
            tab: tab.id,
            initial: tab.to_file_context(),
            restored: Some(restored),
        };
        let mut state = crate::gui::state::GuiState::new(db);
        state.tabs.push(tab);
        let full = crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"row\n".repeat(10)),
            manager,
            "preview".into(),
        );
        expand_preview(&mut state, &preview, full, Default::default());
        assert_eq!(state.tabs[0].scroll.scroll_offset, 8);
    }

    /// Run with LOGANA_PERF_FILE=/path/to/log cargo test --release
    /// --no-default-features --features gui --lib profile_large_file_preparation
    /// -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a local large log; measures real GUI loading without a window"]
    async fn profile_large_file_preparation() {
        let path = PathBuf::from(std::env::var("LOGANA_PERF_FILE").expect("LOGANA_PERF_FILE"));
        for run in 1..=3 {
            let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
            let (control, _) = Control::new();
            let start = Instant::now();
            if let Some(preview) =
                crate::gui::update::load_file_preview(&path, db.clone(), &control, 1024 * 1024)
                    .await
                    .unwrap()
            {
                let (mut tab, widths) = crate::gui::update::prepare_tab(
                    preview,
                    &crate::gui::persistence::Preferences::default(),
                    &control.cancel,
                )
                .unwrap();
                let fields = tab.collect_field_names();
                let state = gpui_kit::gpui::ListState::new(
                    0,
                    gpui_kit::gpui::ListAlignment::Top,
                    gpui_kit::gpui::px(60.),
                );
                crate::gui::view::log_pane::prepare_list(
                    &mut None,
                    &state,
                    &tab,
                    &widths.into(),
                    &fields,
                    800.,
                );
                eprintln!(
                    "run {run}: first chunk ready={:?}, lines={}",
                    start.elapsed(),
                    tab.file_reader.line_count()
                );
            }
            let start = Instant::now();
            let loaded =
                crate::gui::update::load_file_controlled(path.clone(), db, control.clone(), None)
                    .await
                    .unwrap();
            let index = start.elapsed();
            let start = Instant::now();
            let (mut tab, widths) = crate::gui::update::prepare_tab(
                loaded,
                &crate::gui::persistence::Preferences::default(),
                &control.cancel,
            )
            .unwrap();
            eprintln!(
                "run {run}: index={index:?}, prepare={:?}, lines={}, format={:?}",
                start.elapsed(),
                tab.file_reader.line_count(),
                tab.display.format.as_ref().map(|p| p.name())
            );
            let start = Instant::now();
            let fields = tab.collect_field_names();
            let state = gpui_kit::gpui::ListState::new(
                0,
                gpui_kit::gpui::ListAlignment::Top,
                gpui_kit::gpui::px(60.),
            );
            crate::gui::view::log_pane::prepare_list(
                &mut None,
                &state,
                &tab,
                &widths.into(),
                &fields,
                800.,
            );
            eprintln!(
                "run {run}: wrapped layout={:?}, rows={}",
                start.elapsed(),
                state.item_count()
            );
        }
    }
    #[tokio::test]
    async fn queued_operations_can_cancel_without_waiting_for_active_worker() {
        let gate = Arc::new(Semaphore::new(1));
        let held = gate.clone().acquire_owned().await.unwrap();
        let (control, _) = Control::new();
        control.cancel.store(true, Ordering::Relaxed);
        assert!(permit(gate.clone(), &control).await.is_err());
        drop(held);
        let (next, _) = Control::new();
        assert!(permit(gate, &next).await.is_ok());
    }
    #[tokio::test]
    async fn file_progress_and_preparation_cancel_before_publication() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            b"{\"level\":\"INFO\",\"message\":\"sample\"}\n",
        )
        .unwrap();
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let (control, progress) = Control::new();
        let loaded = crate::gui::update::load_file_controlled(
            file.path().to_path_buf(),
            db,
            control.clone(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(progress.borrow().fraction, Some(1.));
        control.cancel.store(true, Ordering::Relaxed);
        assert!(
            crate::gui::update::prepare_tab(loaded, &Default::default(), &control.cancel).is_err()
        );
    }
    #[tokio::test]
    async fn archive_provenance_survives_loading_and_completed_tabs_do_not_steal_focus() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"hello\n").unwrap();
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let (control, _) = Control::new();
        let loaded = crate::gui::update::load_file_controlled(
            file.path().to_path_buf(),
            db.clone(),
            control.clone(),
            Some("bundle.zip/logs/app.log".into()),
        )
        .await
        .unwrap();
        assert_eq!(
            loaded.log_manager.source_file(),
            Some("bundle.zip/logs/app.log")
        );
        let handle = runtime::handle();
        let _guard = handle.enter();
        let (tab, widths) =
            crate::gui::update::prepare_tab(loaded, &Default::default(), &control.cancel).unwrap();
        let mut state = crate::gui::state::GuiState::new(db.clone());
        let manager = crate::db::LogManager::new(db, None).await;
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"existing\n".to_vec()),
            manager,
            "existing".into(),
        ));
        crate::gui::update::update(
            &mut state,
            Message::TabPrepared(Box::new(tab), widths, false),
        );
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_tab, 0);
    }
}

#[cfg(all(test, feature = "gui-test"))]
mod ui_tests {
    use super::*;
    use gpui_kit::{
        gpui::{AppContext, TestAppContext},
        test::TestWindowExt,
    };
    #[gpui_kit::test]
    fn pending_file_load_allows_tab_switching_and_actions(cx: &mut TestAppContext) {
        let db = Arc::new(
            runtime::handle()
                .block_on(crate::db::Database::in_memory())
                .unwrap(),
        );
        let manager = runtime::handle().block_on(crate::db::LogManager::new(db.clone(), None));
        let theme = crate::theme::Theme::default();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::gui::theme::install(&theme, cx);
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| App::new(db.clone(), theme.clone(), cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                for title in ["first", "second"] {
                    app.state.tabs.push(crate::shared::TabState::new(
                        crate::ingestion::FileReader::from_bytes(b"existing\n".to_vec()),
                        manager.clone(),
                        title.into(),
                    ));
                }
                let path = PathBuf::from("new.log");
                assert!(matches!(
                    crate::gui::update::update(
                        &mut app.state,
                        Message::FileDialogResult(Some(path.clone()))
                    ),
                    crate::gui::effect::Effect::LoadFile(_)
                ));
                let (control, rx) = Control::new();
                app.loading.operations.push(Operation {
                    id: 1,
                    path,
                    control,
                    rx,
                    progress: Progress::default(),
                    picker: None,
                    args: None,
                    preview: None,
                });
                app.dispatch(Message::TabSelected(1), window, cx);
                app.dispatch(
                    Message::GuiAction(crate::gui::actions::Action::ShowBookmarks),
                    window,
                    cx,
                );
                assert_eq!(app.state.active_tab, 1);
                assert_eq!(app.state.nav_page, crate::gui::state::NavPage::Bookmarks);
                assert!(!app.state.palette.pending());
                assert_eq!(app.loading.operations.len(), 1);
            });
            window.render_frame(cx);
            window.click("open-commands", cx);
            assert!(app.read(cx).state.palette.open);
            app.update(cx, |app, cx| {
                let tab = crate::shared::TabState::new(
                    crate::ingestion::FileReader::from_bytes(b"new file\n".to_vec()),
                    manager.clone(),
                    "new.log".into(),
                );
                app.dispatch(
                    Message::TabPrepared(Box::new(tab), Default::default(), false),
                    window,
                    cx,
                );
                app.finish_loading(1, Ok(()), cx);
            });
        })
        .unwrap();
        app.read_with(cx, |app, _| {
            assert_eq!(app.state.tabs.len(), 3);
            assert_eq!(app.state.active_tab, 1);
            assert_eq!(app.state.nav_page, crate::gui::state::NavPage::Bookmarks);
            assert!(app.state.palette.open);
        });
    }
    #[gpui_kit::test]
    fn progress_cancel_and_archive_picker_controls_are_native(cx: &mut TestAppContext) {
        let db = Arc::new(
            runtime::handle()
                .block_on(crate::db::Database::in_memory())
                .unwrap(),
        );
        let (control, rx) = Control::new();
        control.phase("Loading", "large.log".into(), Some(1_000_000));
        control.bytes(500_000);
        let manager = runtime::handle().block_on(crate::db::LogManager::new(db.clone(), None));
        let theme = crate::theme::Theme::default();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::gui::theme::install(&theme, cx);
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| App::new(db.clone(), theme.clone(), cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                let progress = rx.borrow().clone();
                app.state.tabs.push(crate::shared::TabState::new(
                    crate::ingestion::FileReader::from_bytes(b"completed\n".to_vec()),
                    manager.clone(),
                    "completed".into(),
                ));
                let tab = crate::shared::TabState::new(
                    crate::ingestion::FileReader::from_bytes(b"first chunk\n".to_vec()),
                    manager,
                    "large.log".into(),
                );
                let preview = Preview {
                    tab: tab.id,
                    initial: tab.to_file_context(),
                    restored: None,
                };
                app.state.tabs.push(tab);
                app.state.active_tab = 1;
                app.loading.operations.push(Operation {
                    id: 7,
                    path: "large.log".into(),
                    progress,
                    rx,
                    control: control.clone(),
                    picker: None,
                    args: None,
                    preview: Some(preview),
                });
                cx.notify();
            });
            window.render_frame(cx);
            window.click("loading-details-toggle", cx);
            window.render_frame(cx);
            let _ = window.find(("load-progress", 7_u64));
            assert_eq!(
                app.read(cx).loading.operations[0].progress.fraction,
                Some(0.5)
            );
            window.click(("load-cancel", 7_u64), cx);
            assert!(control.cancelled());
            app.update(cx, |app, cx| {
                app.finish_loading(7, Ok(()), cx);
                assert_eq!(app.state.tabs.len(), 1);
                assert_eq!(app.state.tabs[0].title, "completed");
                let root = crate::ingestion::archive::test_helpers::make_zip(&[
                    ("one.log", b"one\n"),
                    ("two.log", b"two\n"),
                ]);
                let file = tempfile::Builder::new().suffix(".zip").tempfile().unwrap();
                std::fs::copy(root.path(), file.path()).unwrap();
                let (control, rx) = Control::new();
                let tree = crate::ingestion::archive_tree::list_archive_tree_controlled(
                    &file.path().to_string_lossy(),
                    control.clone(),
                )
                .unwrap();
                app.loading.operations.push(Operation {
                    id: 8,
                    path: "bundle.zip".into(),
                    control,
                    rx,
                    progress: Progress::default(),
                    picker: Some(Picker::new(tree)),
                    args: None,
                    preview: None,
                });
                app.loading.selected_picker = Some(8);
                cx.notify();
            });
            window.render_frame(cx);
            window.click("archive-select-all", cx);
            assert!(
                app.read(cx).loading.operations[0]
                    .picker
                    .as_ref()
                    .unwrap()
                    .tree
                    .nodes
                    .iter()
                    .all(|n| n.selected)
            );
            window.click("archive-clear", cx);
            assert!(
                !app.read(cx).loading.operations[0]
                    .picker
                    .as_ref()
                    .unwrap()
                    .tree
                    .any_file_selected()
            );
            app.update(cx, |app, cx| app.loading.focus.focus(window, cx));
            window.press("space", cx);
            assert!(
                app.read(cx).loading.operations[0]
                    .picker
                    .as_ref()
                    .unwrap()
                    .tree
                    .nodes[0]
                    .selected
            );
            window.press("down", cx);
            window.press("space", cx);
            assert!(
                app.read(cx).loading.operations[0]
                    .picker
                    .as_ref()
                    .unwrap()
                    .tree
                    .nodes[1]
                    .selected
            );
            window.click("archive-cancel", cx);
            assert!(app.read(cx).loading.operations.is_empty());
        })
        .unwrap();
    }
}
