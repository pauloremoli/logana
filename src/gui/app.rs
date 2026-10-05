use crate::commands::Commands;
use crate::db::{Database, LogManager};
use crate::gui::effect::Effect;
use crate::gui::message::Message;
use crate::gui::runtime;
use crate::gui::state::GuiState;
use crate::gui::update;
use crate::theme::Theme as TuiTheme;
use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, ScrollStrategy, Window, actions, div,
};
use std::path::PathBuf;
use std::sync::Arc;

// GUI actions are independent of terminal modes. Only the launcher has shortcuts.
actions!(logana_gui, [OpenFile, OpenCommands, Quit]);

/// GPUI widget ownership and asynchronous effect execution.
pub struct App {
    pub state: GuiState,
    pub(super) stream_pending: std::collections::HashSet<crate::shared::TabId>,
    pub editor_inputs: Vec<Entity<InputState>>,
    pub editor_textareas:
        std::collections::HashMap<usize, Entity<gpui_kit::component::input::TextareaState>>,
    editor_epoch: u64,
    editor_synced: Vec<String>,
    filter_edits: tokio::sync::mpsc::UnboundedSender<crate::gui::effect::FilterEdit>,
    pub log_scroll: VirtualListScrollHandle,
    pub field_widths: Option<crate::gui::column_widths::FieldWidthCache>,
    notification_timer_revision: u64,
    pub table_layout: Option<crate::gui::view::log_pane::LayoutCache>,
    pub menu_bar: Entity<AppMenuBar>,
    pub facet_counts: std::collections::HashMap<crate::shared::TabId, FacetCache>,
    pub facet_search: Option<Entity<InputState>>,
    pub log_search: Option<Entity<InputState>>,
    pub command_input: Option<Entity<InputState>>,
    pub command_scroll: gpui_kit::gpui::ScrollHandle,
    command_focus_epoch: u64,
    command_synced_text: String,
    search_tab: Option<usize>,
    /// Tracked by the root element (`Render::render`) so every keystroke
    /// has somewhere to land even when nothing else (a button, the menu
    /// bar) is focused. Reclaimed after every dispatch — see
    /// `reclaim_focus_if_idle` — so a transient focus change elsewhere
    /// (clicking a button, closing a native dialog) never leaves the
    /// window unable to receive the next keystroke.
    pub root_focus: FocusHandle,
    pub interface_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

pub struct FacetCache {
    pub(super) key: (usize, String),
    pub counts: Option<crate::shared::FieldValueCounts>,
}

impl App {
    pub fn new(db: Arc<Database>, theme: TuiTheme, cx: &mut Context<Self>) -> Self {
        let (filter_edits, mut edits) =
            tokio::sync::mpsc::unbounded_channel::<crate::gui::effect::FilterEdit>();
        cx.spawn(async move |this, cx| {
            while let Some(edit) = edits.recv().await {
                let result = runtime::handle().spawn(edit.persist()).await;
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error.to_string()),
                    Err(error) => Some(error.to_string()),
                };
                if let Some(error) = error
                    && this
                        .update_in(cx, |app, window, cx| {
                            app.dispatch(Message::FilterPersistenceFailed(error), window, cx);
                        })
                        .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                if this
                    .update_in(cx, |app, _window, cx| {
                        app.poll_streams(cx);
                        let mut filtering = false;
                        for tab in &mut app.state.tabs {
                            if tab.filter.handle.is_some() {
                                tab.advance_filter_computation();
                                if tab.stream.tail_mode && !tab.stream.paused {
                                    tab.scroll.scroll_offset =
                                        tab.filter.visible_indices.len().saturating_sub(1);
                                }
                                filtering = true;
                            }
                        }
                        for tab in &mut app.state.tabs {
                            if let Some(handle) = &mut tab.search.handle {
                                let mut done = false;
                                for _ in 0..4 {
                                    match handle.result_rx.try_recv() {
                                        Ok(batch) => {
                                            tab.search.query.extend_results(batch);
                                            tab.cache.search_result_gen =
                                                tab.cache.search_result_gen.wrapping_add(1);
                                            filtering = true;
                                        }
                                        Err(
                                            tokio::sync::mpsc::error::TryRecvError::Disconnected,
                                        ) => {
                                            done = true;
                                            break;
                                        }
                                        Err(_) => break,
                                    }
                                }
                                if done {
                                    tab.search.handle = None;
                                }
                            }
                        }
                        if filtering {
                            if app
                                .state
                                .active_tab()
                                .is_some_and(|tab| tab.stream.tail_mode && !tab.stream.paused)
                            {
                                app.sync_log_scroll();
                            }
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            filter_edits,
            state: GuiState::new(db).with_theme(theme),
            stream_pending: Default::default(),
            editor_inputs: Vec::new(),
            editor_textareas: Default::default(),
            editor_epoch: 0,
            editor_synced: Vec::new(),
            log_scroll: VirtualListScrollHandle::new(),
            table_layout: None,
            notification_timer_revision: 0,
            field_widths: None,
            menu_bar: AppMenuBar::new(cx),
            facet_counts: Default::default(),
            facet_search: None,
            log_search: None,
            command_input: None,
            command_scroll: gpui_kit::gpui::ScrollHandle::new(),
            command_focus_epoch: 0,
            command_synced_text: String::new(),
            search_tab: None,
            root_focus: cx.focus_handle(),
            interface_focus: cx.focus_handle(),
            return_focus: None,
        }
    }

    fn on_open_file_action(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Message::OpenFileDialog, window, cx);
    }

    fn on_open_commands_action(
        &mut self,
        _: &OpenCommands,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(Message::OpenCommands, window, cx);
    }

    fn on_quit_action(&mut self, _: &Quit, _window: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    pub fn dispatch(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.editor.is_none()
            && !self.state.palette.visible()
            && matches!(
                message,
                Message::OpenCommands
                    | Message::GuiAction(_)
                    | Message::LogRowAction(..)
                    | Message::EditFilter(_)
                    | Message::EditGroup(_)
            )
        {
            self.return_focus = window.focused(cx);
        }
        if matches!(message, Message::EditorApply | Message::EditorConditionAdd) {
            if let Some(editor) = &mut self.state.editor {
                if editor.epoch == self.editor_epoch {
                    for (index, input) in self.editor_inputs.iter().enumerate() {
                        let value = self
                            .editor_textareas
                            .get(&index)
                            .map(|input| input.read(cx).value().to_string())
                            .unwrap_or_else(|| input.read(cx).value().to_string());
                        if self.editor_synced.get(index) == Some(&editor.values[index])
                            && value != editor.values[index]
                        {
                            editor.values[index] = value.clone();
                            if let Some(input) = self.editor_textareas.get(&index) {
                                input.update(cx, |input, cx| {
                                    input.set_value(value.clone(), window, cx)
                                });
                            }
                            self.editor_synced[index] = value;
                            editor.error = None;
                        }
                    }
                }
            }
        }
        let selecting = matches!(message, Message::SelectLog(..));
        let scroll_requested = matches!(
            message,
            Message::SearchNext(_)
                | Message::TabSelected(_)
                | Message::TabClosed(_)
                | Message::FileLoaded(_)
                | Message::GuiAction(crate::gui::actions::Action::Tail)
                | Message::GuiCommand(Commands::Tail, _)
        );
        let had_editor = self.state.editor.is_some();
        let closing_palette = matches!(
            message,
            Message::PaletteInsert(_)
                | Message::GuiAction(_)
                | Message::EditorCancel
                | Message::EditorApply
                | Message::PaletteClose
                | Message::PaletteKey(crate::input::KeyCode::Esc | crate::input::KeyCode::Enter, _)
        );
        let previous_theme = self.state.theme.clone();
        let runtime_handle = runtime::handle();
        let _runtime_guard = runtime_handle.enter();
        let effect = update::update(&mut self.state, message);
        if previous_theme != self.state.theme {
            crate::gui::theme::install(&self.state.theme, cx);
        }
        self.apply_effect(effect, window, cx);
        if (closing_palette || had_editor)
            && self.state.editor.is_none()
            && !self.state.palette.visible()
        {
            self.return_focus
                .as_ref()
                .unwrap_or(&self.root_focus)
                .focus(window, cx);
        }
        self.command_scroll
            .scroll_to_item(self.state.palette.selected);
        if scroll_requested {
            self.sync_log_scroll();
        }
        if selecting {
            self.root_focus.focus(window, cx);
        }
        self.sync_notification_timer(cx);
        self.reclaim_focus_if_idle(window, cx);
        cx.notify();
    }

    fn sync_notification_timer(&mut self, cx: &mut Context<Self>) {
        let revision = self.state.notification_revision;
        if self.state.status.is_none() || self.notification_timer_revision == revision {
            return;
        }
        self.notification_timer_revision = revision;
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_secs(5));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::DismissNotification(revision), window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Refocuses the root element whenever nothing else holds focus, so
    /// key capture keeps working no matter what happened in between —
    /// a button click, a native file-dialog round trip, anything. Only
    /// acts when focus is genuinely idle (`window.focused(cx).is_none()`),
    /// so it never fights a widget that's deliberately holding focus.
    fn reclaim_focus_if_idle(&self, window: &mut Window, cx: &mut Context<Self>) {
        if window.focused(cx).is_none() {
            self.root_focus.focus(window, cx);
        }
    }

    /// Scroll only for explicit navigation, preserving mouse scrolling during edits.
    fn sync_log_scroll(&self) {
        if let Some(tab) = self.state.active_tab() {
            self.log_scroll
                .scroll_to_item(tab.scroll.scroll_offset, ScrollStrategy::Top);
        }
    }

    fn apply_effect(&mut self, effect: Effect, _window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            Effect::ConnectSource(source) => self.connect_source(source, cx),
            Effect::ExportDialog {
                tab_id,
                template,
                fields,
            } => self.prompt_export(tab_id, template, fields, cx),
            Effect::SaveLogsDialog { tab_id, marked } => self.prompt_save_logs(tab_id, marked, cx),
            Effect::FilterFileDialog { action, tab_id } => {
                self.prompt_filter_file(action, tab_id, cx)
            }
            Effect::ClearGuiDates {
                request_id,
                tab_id,
                log_manager,
            } => {
                cx.spawn(async move |this, cx| {
                    let result = runtime::handle()
                        .spawn(crate::gui::actions::clear_dates(log_manager))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    this.update_in(cx, |app, window, cx| {
                        let mut result = result;
                        if let Ok(manager) = &mut result {
                            app.preserve_filter_edits(tab_id, manager);
                        }
                        app.dispatch(
                            Message::PaletteExecuted(request_id, tab_id, result),
                            window,
                            cx,
                        );
                    })
                    .ok();
                })
                .detach();
            }
            Effect::SaveGuiTheme {
                request_id,
                name,
                db,
            } => {
                cx.spawn(async move |this, cx| {
                    use crate::db::{AppSettingsStore, SettingsKey};
                    let result = runtime::handle()
                        .spawn(async move {
                            db.save_app_setting(SettingsKey::Theme, &name)
                                .await
                                .map(|_| format!("Theme changed to {name}."))
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    this.update_in(cx, |app, window, cx| {
                        app.dispatch(Message::GuiFinished(request_id, result), window, cx)
                    })
                    .ok();
                })
                .detach();
            }
            Effect::SaveGuiFilter {
                request_id,
                tab_id,
                log_manager,
                id,
                pattern,
                kind,
                options,
            } => {
                cx.spawn(async move |this, cx| {
                    let result = runtime::handle()
                        .spawn(async move {
                            crate::gui::actions::save_filter(
                                log_manager,
                                id,
                                pattern,
                                kind,
                                options,
                            )
                            .await
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    this.update_in(cx, |app, window, cx| {
                        let mut result = result;
                        if let Ok(manager) = &mut result {
                            app.preserve_filter_edits(tab_id, manager);
                        }
                        app.dispatch(
                            Message::PaletteExecuted(request_id, tab_id, result),
                            window,
                            cx,
                        )
                    })
                    .ok();
                })
                .detach();
            }
            Effect::SaveGuiGroup {
                request_id,
                tab_id,
                log_manager,
                name,
                fg,
                bg,
                whole_line,
                members,
            } => {
                cx.spawn(async move |this, cx| {
                    let result = runtime::handle()
                        .spawn(crate::gui::actions::save_group(
                            log_manager,
                            name,
                            fg,
                            bg,
                            whole_line,
                            members,
                        ))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    this.update_in(cx, |app, window, cx| {
                        let mut result = result;
                        if let Ok(manager) = &mut result {
                            app.preserve_filter_edits(tab_id, manager);
                        }
                        app.dispatch(
                            Message::PaletteExecuted(request_id, tab_id, result),
                            window,
                            cx,
                        );
                    })
                    .ok();
                })
                .detach();
            }
            Effect::PersistFilterEdit(edit) => {
                if self.filter_edits.send(edit).is_err() {
                    self.state.notify(crate::gui::state::StatusMessage::Error(
                        "Filter persistence worker stopped".into(),
                    ));
                }
            }

            Effect::ExecutePalette {
                request_id,
                tab_id,
                log_manager,
                command,
                theme_bg,
                filter_context,
            } => {
                cx.spawn(async move |this, cx| {
                    let result = runtime::handle()
                        .spawn(async move {
                            let manager = if matches!(command, Commands::DateFilter { .. }) {
                                crate::gui::actions::clear_dates(log_manager).await?
                            } else {
                                log_manager
                            };
                            update::execute_command(manager, command, theme_bg, filter_context)
                                .await
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("Command task failed: {error}")));
                    this.update_in(cx, |app, window, cx| {
                        let mut result = result;
                        if let Ok(manager) = &mut result {
                            app.preserve_filter_edits(tab_id, manager);
                        }
                        app.dispatch(
                            Message::PaletteExecuted(request_id, tab_id, result),
                            window,
                            cx,
                        )
                    })
                    .ok();
                })
                .detach();
            }
            Effect::None => {}
            Effect::Quit => cx.quit(),
            Effect::OpenFileDialog => self.spawn_open_file_dialog(cx),
            Effect::LoadFile(path) => self.spawn_load_file(path, cx),
            Effect::ToggleFilter {
                tab_idx,
                log_manager,
                id,
            } => self.spawn_toggle_filter(tab_idx, log_manager, id, cx),
            Effect::ToggleGroup {
                tab_idx,
                log_manager,
                name,
            } => self.spawn_toggle_group(tab_idx, log_manager, name, cx),
            Effect::RemoveGroup {
                tab_idx,
                log_manager,
                name,
            } => self.spawn_remove_group(tab_idx, log_manager, name, cx),
            Effect::RemoveFilter {
                tab_idx,
                log_manager,
                id,
            } => self.spawn_remove_filter(tab_idx, log_manager, id, cx),
            Effect::ExecuteCommand {
                request,
                tab_idx,
                log_manager,
                command,
                theme_bg,
                filter_context,
            } => {
                if let Some((request_id, tab_id)) = request {
                    self.apply_effect(
                        Effect::ExecutePalette {
                            request_id,
                            tab_id,
                            log_manager,
                            command,
                            theme_bg,
                            filter_context,
                        },
                        _window,
                        cx,
                    );
                } else {
                    self.spawn_execute_command(
                        tab_idx,
                        log_manager,
                        command,
                        theme_bg,
                        filter_context,
                        cx,
                    );
                }
            }
        }
    }

    fn spawn_open_file_dialog(&self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui_kit::gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let path = prompt
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|mut paths| paths.pop());
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::FileDialogResult(path), window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn spawn_load_file(&self, path: PathBuf, cx: &mut Context<Self>) {
        let db = Arc::clone(&self.state.db);
        cx.spawn(async move |this, cx| {
            let result = runtime::handle()
                .spawn(async move { update::load_file(path, db).await })
                .await
                .expect("load_file task panicked");
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::FileLoaded(result), window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn refresh_facet_counts(&mut self, cx: &mut Context<Self>) {
        self.facet_counts
            .retain(|id, _| self.state.tabs.iter().any(|tab| tab.id == *id));
        let Some(tab) = self.state.active_tab() else {
            return;
        };
        let id = tab.id;
        let key = (
            tab.file_reader.line_count(),
            tab.display
                .format
                .as_ref()
                .map(|p| p.name().to_string())
                .unwrap_or_default(),
        );
        if self
            .facet_counts
            .get(&id)
            .is_some_and(|entry| entry.key == key)
        {
            return;
        }
        let reader = tab.file_reader.clone();
        let parser = tab.display.format.clone();
        self.facet_counts.insert(
            id,
            FacetCache {
                key: key.clone(),
                counts: None,
            },
        );
        cx.spawn(async move |this, cx| {
            let result = runtime::handle()
                .spawn_blocking(move || {
                    crate::shared::TabState::field_value_counts_for(&reader, parser.as_deref())
                })
                .await;
            this.update(cx, |app, cx| {
                if let Some(entry) = app.facet_counts.get_mut(&id)
                    && entry.key == key
                {
                    entry.counts = Some(result.unwrap_or_default());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn preserve_filter_edits(&mut self, tab_id: crate::shared::TabId, manager: &mut LogManager) {
        let Some(tab) = self.state.tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        let changes = manager.merge_newer_ui_edits(&tab.log_manager);
        let db = manager.db.clone();
        for (id, enabled) in changes.enabled {
            let _ = self
                .filter_edits
                .send(crate::gui::effect::FilterEdit::Toggle {
                    db: db.clone(),
                    id,
                    enabled,
                });
        }
        for (name, enabled) in changes.groups {
            let _ = self
                .filter_edits
                .send(crate::gui::effect::FilterEdit::Group {
                    db: db.clone(),
                    source: manager.source_file().unwrap_or("").into(),
                    name,
                    enabled,
                });
        }
        if let Some(order) = changes.order {
            let _ = self
                .filter_edits
                .send(crate::gui::effect::FilterEdit::Reorder { db, order });
        }
    }

    fn spawn_toggle_filter(
        &self,
        tab_idx: usize,
        mut log_manager: LogManager,
        id: usize,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let log_manager = runtime::handle()
                .spawn(async move {
                    log_manager.toggle_filter(id).await;
                    log_manager
                })
                .await
                .expect("toggle_filter task panicked");
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::FiltersMutated(tab_idx, log_manager), window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn spawn_toggle_group(
        &self,
        tab_idx: usize,
        mut log_manager: LogManager,
        name: String,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let log_manager = runtime::handle()
                .spawn(async move {
                    update::toggle_group_checkbox(&mut log_manager, &name).await;
                    log_manager
                })
                .await
                .expect("toggle_group task panicked");
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::GroupsMutated(tab_idx, log_manager), window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn spawn_remove_group(
        &self,
        tab_idx: usize,
        mut log_manager: LogManager,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.state.tabs.get(tab_idx).map(|tab| tab.id) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let log_manager = runtime::handle()
                .spawn(async move {
                    log_manager.remove_group(&name).await;
                    log_manager
                })
                .await
                .expect("remove_group task panicked");
            this.update_in(cx, |app, window, cx| {
                let Some(tab_idx) = app.state.tabs.iter().position(|tab| tab.id == tab_id) else {
                    return;
                };
                let mut log_manager = log_manager;
                app.preserve_filter_edits(tab_id, &mut log_manager);
                app.dispatch(Message::GroupsMutated(tab_idx, log_manager), window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn spawn_remove_filter(
        &self,
        tab_idx: usize,
        mut log_manager: LogManager,
        id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.state.tabs.get(tab_idx).map(|tab| tab.id) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let log_manager = runtime::handle()
                .spawn(async move {
                    log_manager.remove_filter(id).await;
                    log_manager
                })
                .await
                .expect("remove_filter task panicked");
            this.update_in(cx, |app, window, cx| {
                let Some(tab_idx) = app.state.tabs.iter().position(|tab| tab.id == tab_id) else {
                    return;
                };
                let mut log_manager = log_manager;
                app.preserve_filter_edits(tab_id, &mut log_manager);
                app.dispatch(Message::FiltersMutated(tab_idx, log_manager), window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn spawn_execute_command(
        &self,
        tab_idx: usize,
        log_manager: LogManager,
        command: Commands,
        theme_bg: (u8, u8, u8),
        filter_context: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.state.tabs.get(tab_idx).map(|tab| tab.id) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = runtime::handle()
                .spawn(async move {
                    update::execute_command(log_manager, command, theme_bg, filter_context).await
                })
                .await
                .expect("execute_command task panicked");
            this.update_in(cx, |app, window, cx| {
                let Some(tab_idx) = app.state.tabs.iter().position(|tab| tab.id == tab_id) else {
                    return;
                };
                let mut result = result;
                if let Ok(manager) = &mut result {
                    app.preserve_filter_edits(tab_id, manager);
                }
                app.dispatch(Message::CommandExecuted(tab_idx, result), window, cx);
            })
            .ok();
        })
        .detach();
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_notification_timer(cx);
        self.refresh_facet_counts(cx);
        if self.command_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search actions…"));
            cx.subscribe_in(
                &input,
                window,
                |app, input, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change)
                        && app.command_synced_text == app.state.palette.input
                        && input.read(cx).value().as_ref() != app.state.palette.input.as_str()
                    {
                        app.dispatch(
                            Message::PaletteEdited(input.read(cx).value().to_string()),
                            window,
                            cx,
                        );
                    }
                },
            )
            .detach();
            self.command_input = Some(input);
        }
        if self.state.palette.visible()
            && let Some(input) = self.command_input.clone()
        {
            let value = input.read(cx).value().to_string();
            if self.state.palette.input != self.command_synced_text {
                input.update(cx, |input, cx| {
                    input.set_value(self.state.palette.input.clone(), window, cx)
                });
                self.command_synced_text = self.state.palette.input.clone();
            } else if value != self.command_synced_text {
                // Native edits may precede their deferred Change event. Never
                // overwrite them with the previous reducer snapshot.
                let effect = update::update(&mut self.state, Message::PaletteEdited(value.clone()));
                self.command_synced_text = value;
                self.apply_effect(effect, window, cx);
            }
            if self.command_focus_epoch != self.state.palette.focus_epoch {
                input.update(cx, |input, cx| input.focus(window, cx));
                self.command_focus_epoch = self.state.palette.focus_epoch;
            }
        }
        if self.log_search.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Search logs... (regex, field:value, or free text)")
            });
            cx.subscribe_in(
                &input,
                window,
                |app, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change | InputEvent::Focus => {
                        app.dispatch(
                            Message::SearchEdited(input.read(cx).value().to_string()),
                            window,
                            cx,
                        );
                    }
                    InputEvent::PressEnter { .. } => {
                        app.dispatch(Message::SearchNext(true), window, cx);
                    }
                    _ => {}
                },
            )
            .detach();
            self.log_search = Some(input);
        }
        if self.search_tab != Some(self.state.active_tab) {
            let value = self
                .state
                .active_tab()
                .and_then(|tab| tab.search.query.get_pattern())
                .unwrap_or_default()
                .to_string();
            self.log_search
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.set_value(value, window, cx));
            self.search_tab = Some(self.state.active_tab);
        }
        if self.facet_search.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search filters..."));
            cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                if matches!(
                    event,
                    InputEvent::Change | InputEvent::Focus | InputEvent::Blur
                ) {
                    cx.notify();
                }
            })
            .detach();
            self.facet_search = Some(input);
        }
        if let Some(editor) = &self.state.editor {
            let values = editor.values.clone();
            let editor_action = editor.action;
            let epoch = editor.epoch;
            if self.editor_epoch != epoch || self.editor_inputs.is_empty() {
                self.editor_inputs.clear();
                self.editor_textareas.clear();
                self.editor_synced = values.to_vec();
                for (index, value) in values.into_iter().enumerate() {
                    let input = cx.new(|cx| {
                        let mut input = InputState::new(window, cx);
                        input.set_value(value, window, cx);
                        input
                    });
                    cx.subscribe_in(
                        &input,
                        window,
                        move |app, input, event: &InputEvent, window, cx| {
                            if matches!(event, InputEvent::Change)
                                && app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch)
                            {
                                let value = input.read(cx).value().to_string();
                                if app.editor_synced.get(index)
                                    == app.state.editor.as_ref().map(|e| &e.values[index])
                                {
                                    app.editor_synced[index] = value.clone();
                                    app.dispatch(Message::EditorText(index, value), window, cx);
                                }
                            }
                        },
                    )
                    .detach();
                    self.editor_inputs.push(input);
                    if (editor_action == crate::gui::actions::Action::Annotation && index == 0)
                        || (editor_action == crate::gui::actions::Action::Run && index == 1)
                        || (editor_action == crate::gui::actions::Action::Export && index >= 6)
                    {
                        let value = self.state.editor.as_ref().unwrap().values[index].clone();
                        let textarea = cx.new(|cx| {
                            let mut input =
                                gpui_kit::component::input::TextareaState::new(window, cx).rows(4);
                            input.set_value(value, window, cx);
                            input
                        });
                        cx.subscribe_in(
                            &textarea,
                            window,
                            move |app, input, event: &InputEvent, window, cx| {
                                if matches!(event, InputEvent::Change)
                                    && app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch)
                                {
                                    let value = input.read(cx).value().to_string();
                                    app.editor_synced[index] = value.clone();
                                    app.dispatch(Message::EditorText(index, value), window, cx);
                                }
                            },
                        )
                        .detach();
                        self.editor_textareas.insert(index, textarea);
                    }
                }
                self.editor_epoch = epoch;
                let action = self.state.editor.as_ref().unwrap().action;
                match action {
                    crate::gui::actions::Action::Appearance => {
                        self.editor_inputs[1].update(cx, |input, cx| input.focus(window, cx))
                    }
                    crate::gui::actions::Action::FileInfo
                    | crate::gui::actions::Action::View
                    | crate::gui::actions::Action::ValueColors
                    | crate::gui::actions::Action::LevelColors => {
                        self.interface_focus.focus(window, cx)
                    }
                    crate::gui::actions::Action::Annotation => {
                        self.editor_textareas[&0].update(cx, |input, cx| input.focus(window, cx))
                    }
                    _ => self.editor_inputs[0].update(cx, |input, cx| input.focus(window, cx)),
                };
            } else {
                for (index, value) in values.into_iter().enumerate() {
                    if value != self.editor_synced[index] {
                        self.editor_inputs[index]
                            .update(cx, |input, cx| input.set_value(value.clone(), window, cx));
                        self.editor_synced[index] = value;
                    }
                }
            }
        } else {
            self.editor_inputs.clear();
            self.editor_textareas.clear();
            self.editor_synced.clear();
        }
        let focused = |input: &Option<Entity<InputState>>| {
            input
                .as_ref()
                .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window))
        };
        let editing = focused(&self.log_search) || focused(&self.facet_search);
        let context = if self.state.editor.is_some() || self.state.time_range_open {
            "LoganaDialog"
        } else if self.state.palette.visible() {
            "LoganaActions"
        } else if editing {
            "LoganaInput"
        } else {
            "LoganaLogs"
        };
        div()
            .key_context(context)
            .size_full()
            .track_focus(&self.root_focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(gpui_kit::gpui::px(13.))
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::on_open_file_action))
            .on_action(cx.listener(Self::on_open_commands_action))
            .on_action(cx.listener(Self::on_quit_action))
            .capture_key_down(cx.listener(move |app, event: &KeyDownEvent, window, cx| {
                let stroke = &event.keystroke;
                if stroke.key == "escape" {
                    if app.state.editor.is_some() {
                        cx.stop_propagation();
                        app.dispatch(Message::EditorCancel, window, cx);
                    } else if app.state.palette.visible() {
                        cx.stop_propagation();
                        app.dispatch(Message::PaletteClose, window, cx);
                    } else if app.state.time_range_open {
                        app.state.time_range_open = false;
                        cx.notify();
                    }
                    return;
                }
                if context == "LoganaActions" {
                    let key = match stroke.key.as_str() {
                        "enter" => Some(crate::input::KeyCode::Enter),
                        "up" => Some(crate::input::KeyCode::Up),
                        "down" => Some(crate::input::KeyCode::Down),
                        _ => None,
                    };
                    if let Some(key) = key {
                        cx.stop_propagation();
                        app.dispatch(Message::PaletteKey(key, Default::default()), window, cx);
                    }
                    return;
                }
                if !editing
                    && context == "LoganaLogs"
                    && stroke.modifiers.control
                    && !stroke.modifiers.alt
                {
                    let action = match (stroke.key.as_str(), stroke.modifiers.shift) {
                        ("b", false) => Some(crate::gui::actions::Action::Bookmark),
                        ("b", true) => Some(crate::gui::actions::Action::ToggleBookmarks),
                        ("a", true) => Some(crate::gui::actions::Action::Annotation),
                        _ => None,
                    };
                    if let Some(action) = action {
                        cx.stop_propagation();
                        app.dispatch(Message::GuiAction(action), window, cx);
                        return;
                    }
                }
                if context == "LoganaDialog" || context == "LoganaActions" {
                    return;
                }
                let colon = matches!(stroke.key.as_str(), ":" | "colon")
                    || (stroke.key == ";" && stroke.modifiers.shift);
                let shortcut =
                    stroke.key == "p" && stroke.modifiers.control && stroke.modifiers.shift;
                if shortcut || (colon && !editing) {
                    cx.stop_propagation();
                    app.dispatch(Message::OpenCommands, window, cx);
                }
            }))
            .child(crate::gui::view::view(self, window, cx))
    }
}
