use crate::commands::Commands;
use crate::db::{Database, LogManager};
use crate::gui::effect::Effect;
use crate::gui::message::Message;
use crate::gui::runtime;
use crate::gui::state::{GuiState, StatusMessage};
use crate::gui::update;
use crate::theme::Theme as TuiTheme;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::combobox::{ComboboxEvent, ComboboxState};
use gpui_kit::component::date_picker::{DatePickerEvent, DatePickerState, DateTime};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::select::{SearchableVec, SelectEvent, SelectState};
use gpui_kit::component::time_field::{TimeFieldEvent, TimeFieldState, TimePrecision};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, ListAlignment, ListState, Window,
    actions, div,
};
use std::path::PathBuf;
use std::sync::Arc;

// GUI actions are independent of terminal modes. Only the launcher has shortcuts.
actions!(logana_gui, [OpenFile, OpenCommands, Quit]);

/// GPUI widget ownership and asynchronous effect execution.
pub struct App {
    pub state: GuiState,
    pub(super) loading: crate::gui::loading::Loading,
    pub(super) persistence: crate::gui::persistence::Persistence,
    pub(super) context_scroll_pending: std::collections::HashSet<crate::shared::TabId>,
    pub(super) mcp: crate::gui::mcp::Server,
    pub(super) default_formats: std::collections::HashMap<crate::shared::TabId, String>,
    pub(super) stream_pending: std::collections::HashSet<crate::shared::TabId>,
    pub text_document: crate::gui::text_selection::SharedDocument,
    pub text_handle: gpui_kit::base::TextSelectionHandle,
    text_context: Option<(crate::shared::TabId, u64, u64, Vec<String>, bool, bool)>,
    pub pending_source_jump: Option<(crate::shared::TabId, usize)>,
    pub loading_details_open: bool,
    pub notes_search: Option<Entity<InputState>>,
    pub editor_inputs: Vec<Entity<InputState>>,
    pub editor_textareas:
        std::collections::HashMap<usize, Entity<gpui_kit::component::input::TextareaState>>,
    pub editor_colors: Vec<Entity<gpui_kit::component::color_picker::ColorPickerState>>,
    pub editor_dates: Vec<Entity<DatePickerState>>,
    pub editor_times: Vec<Entity<TimeFieldState>>,
    pub editor_group_select: Option<Entity<SelectState<SearchableVec<String>>>>,
    pub editor_members: Option<Entity<ComboboxState<SearchableVec<String>>>>,
    pub editor_choice_select: Option<Entity<SelectState<SearchableVec<String>>>>,
    pub editor_field_select: Option<Entity<SelectState<SearchableVec<String>>>>,
    pub(super) field_discovery_key: Option<crate::gui::field_discovery::Key>,
    pub(super) field_discovery_task: Option<gpui_kit::gpui::Task<()>>,
    editor_epoch: u64,
    editor_synced: Vec<String>,
    filter_edits: tokio::sync::mpsc::UnboundedSender<crate::gui::effect::FilterEdit>,
    pub log_scroll: ListState,
    pub horizontal_scroll: gpui_kit::gpui::ScrollHandle,
    /// Keyboard navigation (filter/group mode) moves `sidebar_cursor`
    /// without necessarily scrolling it into view — these keep the
    /// selected row visible the same way `command_scroll` does for the
    /// command palette.
    pub filter_sidebar_scroll: gpui_kit::gpui::ScrollHandle,
    pub group_sidebar_scroll: gpui_kit::gpui::ScrollHandle,
    pub field_widths: Option<crate::gui::column_widths::FieldWidthCache>,
    pub(super) table_preparation_key: Option<crate::gui::table_preparation::Key>,
    pub(super) table_preparation_task: Option<gpui_kit::gpui::Task<()>>,
    pub sidebar_width: f32,
    notification_timer_revision: u64,
    pub table_layout: Option<crate::gui::view::log_pane::LayoutCache>,
    pub menu_bar: Entity<AppMenuBar>,
    pub facet_counts: std::collections::HashMap<crate::shared::TabId, FacetCache>,
    pub facet_search: Option<Entity<InputState>>,
    pub archive_search: Option<Entity<InputState>>,
    pub inspected_row: Option<(crate::shared::TabId, usize)>,
    pub inspector_raw: bool,
    pub schema_preview: Option<Result<crate::gui::schema_preview::Sample, String>>,
    pub export_preview: Option<Result<crate::gui::export_preview::Sample, String>>,
    pub(super) export_preview_key: Option<crate::gui::export_preview::Key>,
    pub(super) export_preview_task: Option<gpui_kit::gpui::Task<()>>,
    pub(super) schema_preview_key: Option<crate::gui::schema_preview::Key>,
    pub(super) schema_preview_task: Option<gpui_kit::gpui::Task<()>>,
    archive_search_operation: Option<u64>,
    pub facet_pages: std::collections::HashMap<String, usize>,
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
        crate::gui::menus::install(cx);
        let persistence = crate::gui::persistence::Persistence::new(db.clone());
        cx.on_app_quit(|app, _| {
            let flushed = app.flush_persistence();
            async move {
                if let Err(error) = flushed.await {
                    eprintln!("Cannot save GUI session: {error}");
                }
            }
        })
        .detach();
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
                        // Polling can restart searches, which spawn Tokio blocking tasks.
                        let runtime_handle = runtime::handle();
                        let _runtime_guard = runtime_handle.enter();
                        app.poll_loading(cx);
                        app.poll_streams(cx);
                        app.poll_merges(cx);
                        app.poll_persistence(cx);
                        app.poll_defaults(cx);
                        app.poll_mcp(cx);
                        let mut filtering = false;
                        for tab in &mut app.state.tabs {
                            if tab.filter.handle.is_some() || tab.filter.prepared.is_some() {
                                tab.advance_filter_computation();
                                if tab.filter.handle.is_none() && tab.filter.prepared.is_none() {
                                    let query = tab.search.query.get_pattern().map(str::to_owned);
                                    if let Some(query) = query {
                                        tab.begin_search(&query, true, false);
                                    }
                                }
                                if tab.stream.tail_mode && !tab.stream.paused {
                                    tab.scroll.scroll_offset =
                                        tab.filter.visible_indices.len().saturating_sub(1);
                                }
                                filtering = true;
                            }
                        }
                        if app.state.pending_selection_anchor.is_some() {
                            crate::gui::update::resolve_pending_selection(&mut app.state);
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
                        app.finish_context_scroll();
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
        let text_document = std::rc::Rc::new(std::cell::RefCell::new(
            crate::gui::text_selection::Document::default(),
        ));
        let text_handle = crate::gui::text_selection::new_handle(text_document.clone(), cx);
        Self {
            persistence,
            loading: crate::gui::loading::Loading::new(cx),
            context_scroll_pending: Default::default(),
            text_document,
            text_handle,
            text_context: None,
            filter_edits,
            state: GuiState::new(db).with_theme(theme),
            mcp: Default::default(),
            default_formats: Default::default(),
            stream_pending: Default::default(),
            pending_source_jump: None,
            loading_details_open: false,
            notes_search: None,
            editor_inputs: Vec::new(),
            editor_textareas: Default::default(),
            editor_colors: Vec::new(),
            editor_dates: Vec::new(),
            editor_times: Vec::new(),
            editor_group_select: None,
            editor_members: None,
            editor_choice_select: None,
            editor_field_select: None,
            field_discovery_key: None,
            field_discovery_task: None,
            editor_epoch: 0,
            editor_synced: Vec::new(),
            horizontal_scroll: gpui_kit::gpui::ScrollHandle::new(),
            filter_sidebar_scroll: gpui_kit::gpui::ScrollHandle::new(),
            group_sidebar_scroll: gpui_kit::gpui::ScrollHandle::new(),
            log_scroll: ListState::new(0, ListAlignment::Top, gpui_kit::gpui::px(60.)),
            table_layout: None,
            notification_timer_revision: 0,
            field_widths: None,
            table_preparation_key: None,
            table_preparation_task: None,
            sidebar_width: crate::gui::view::sidebar::SIDEBAR_WIDTH,
            menu_bar: AppMenuBar::new(cx),
            facet_counts: Default::default(),
            facet_search: None,
            archive_search: None,
            inspected_row: None,
            inspector_raw: false,
            schema_preview: None,
            export_preview: None,
            export_preview_key: None,
            export_preview_task: None,
            schema_preview_key: None,
            schema_preview_task: None,
            archive_search_operation: None,
            facet_pages: Default::default(),
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
        self.request_quit(cx);
    }

    fn sync_archive_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.loading.selected_picker else {
            return;
        };
        if self.archive_search_operation != Some(id) {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search filenames…"));
            cx.subscribe(&input, move |app, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    if let Some(picker) = app
                        .loading
                        .operations
                        .iter_mut()
                        .find(|op| op.id == id)
                        .and_then(|op| op.picker.as_mut())
                    {
                        picker.set_search(query);
                    }
                }
                cx.notify();
            })
            .detach();
            self.archive_search = Some(input);
            self.archive_search_operation = Some(id);
        }
        let query = self
            .archive_search
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        if let Some(picker) = self
            .loading
            .operations
            .iter_mut()
            .find(|op| op.id == id)
            .and_then(|op| op.picker.as_mut())
            && picker.query != query
        {
            picker.set_search(query);
        }
    }

    pub fn dispatch(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        let save_column_widths = matches!(&message, Message::ColumnResizeEnded);
        let prepared_restore_id = match &message {
            Message::TabPrepared(tab, _, _) => Some(tab.id),
            _ => None,
        };
        let restoring_context = matches!(&message, Message::FileLoaded(Ok(loaded)) if loaded.context.is_some())
            || matches!(&message, Message::TabPrepared(tab, _, _) if tab.filter.handle.as_ref().is_some_and(|h| h.scroll_anchor.is_some()));
        if matches!(
            message,
            Message::KeyPressed(..) | Message::SelectLog(..) | Message::SearchNext(..)
        ) && let Some(tab) = self.state.active_tab()
        {
            self.context_scroll_pending.remove(&tab.id);
        }
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
        if matches!(message, Message::EditorApply | Message::EditorConditionAdd)
            && let Some(editor) = &mut self.state.editor
            && editor.epoch == self.editor_epoch
        {
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
                        input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
                    }
                    self.editor_synced[index] = value;
                    editor.error = None;
                }
            }
        }
        let selecting = matches!(message, Message::SelectLog(..));
        let scroll_requested = matches!(
            message,
            Message::KeyPressed(..)
                | Message::SearchNext(_)
                | Message::TabSelected(_)
                | Message::TabClosed(_)
                | Message::FileLoaded(_)
                | Message::GuiAction(crate::gui::actions::Action::Tail)
                | Message::GuiCommand(Commands::Tail, _)
        );
        self.save_session_state();
        let previous_display = self.display_snapshot();
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
        if matches!(&message, Message::SearchPopup(false))
            && self.state.search_open
            && let Some(input) = &self.log_search
        {
            let query = input.read(cx).value().to_string();
            if self
                .state
                .active_tab()
                .and_then(|tab| tab.search.query.get_pattern())
                != Some(query.as_str())
            {
                update::update(&mut self.state, Message::SearchEdited(query));
            }
        }
        let search_popup = match &message {
            Message::SearchPopup(open) => Some(*open),
            _ => None,
        };
        let effect = update::update(&mut self.state, message);
        crate::gui::view::bookmarks::sync(&mut self.state);

        // The auto-filter keybinding opens a prefilled draft; it should
        // prefill from a manually selected text range, never from the
        // whole cursor line (`editor.auto_style` is only set here and by
        // `Message::EditorStyle`/the filter sidebar's auto-color toggle,
        // neither of which opens a *new* editor, so `!had_editor` scopes
        // this to the auto-filter keybinding alone).
        if !had_editor
            && self
                .state
                .editor
                .as_ref()
                .is_some_and(|e| e.action == crate::gui::actions::Action::Filter && e.auto_style)
        {
            let selected = self
                .state
                .active_tab()
                .map(|tab| self.text_document.borrow().copy(tab))
                .unwrap_or_default();
            if let Some(editor) = self.state.editor.as_mut() {
                editor.values[0] = selected;
                editor.regenerate_auto_style();
            }
        }
        if self.state.editor.is_some() || self.state.palette.visible() {
            self.state.search_open = false;
        }
        if restoring_context
            && let Some(tab) = prepared_restore_id
                .and_then(|id| self.state.tabs.iter().find(|tab| tab.id == id))
                .or_else(|| self.state.active_tab())
            && tab.filter.handle.is_some()
        {
            self.context_scroll_pending.insert(tab.id);
        }
        self.save_display_change(previous_display);
        if save_column_widths {
            self.save_preferences(self.state.preferences.clone());
        }
        self.save_session_state();
        if previous_theme != self.state.theme {
            crate::gui::theme::install(&self.state.theme, cx);
            self.state.preferences.apply_font_size(window, cx);
        }
        self.apply_effect(effect, window, cx);
        if let Some(open) = search_popup {
            if open && self.state.search_open {
                if let Some(input) = &self.log_search {
                    input.read(cx).focus_handle(cx).focus(window, cx);
                }
            } else if !open {
                self.root_focus.focus(window, cx);
            }
        }
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

    pub(super) fn sync_notification_timer(&mut self, cx: &mut Context<Self>) {
        let revision = self.state.notification_revision;
        if self.state.status.is_none()
            || matches!(self.state.status, Some(StatusMessage::Error(_)))
            || self.notification_timer_revision == revision
        {
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

    /// Restore the viewport only once the saved source cursor has a final visible position.
    pub(super) fn finish_context_scroll(&mut self) -> bool {
        let ready = self.state.active_tab().is_some_and(|tab| {
            tab.filter.handle.is_none() && self.context_scroll_pending.contains(&tab.id)
        });
        self.context_scroll_pending.retain(|id| {
            self.state
                .tabs
                .iter()
                .any(|tab| tab.id == *id && tab.filter.handle.is_some())
        });
        if ready {
            self.sync_log_scroll();
        }
        ready
    }

    /// Scroll only for explicit navigation, preserving mouse scrolling during edits.
    pub(super) fn sync_log_scroll(&self) {
        if let Some(tab) = self.state.active_tab() {
            if !tab.filter.visible_indices.is_empty()
                && tab.scroll.scroll_offset == tab.filter.visible_indices.len() - 1
            {
                self.log_scroll.scroll_to_end();
            } else {
                self.log_scroll
                    .scroll_to_reveal_item(tab.scroll.scroll_offset);
            }
        }
    }

    fn apply_effect(&mut self, effect: Effect, _window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            Effect::MergeTabs(ids) => self.merge_tabs(ids, cx),
            Effect::ConfigureMcp(port) => self.configure_mcp(port, cx),
            Effect::LoadDefaultMappings { epoch } => self.load_default_mappings(epoch, cx),
            Effect::SavePreferences(prefs) => self.save_preferences(prefs),
            Effect::DefaultMappingDialog { epoch, format } => {
                self.prompt_default_mapping(epoch, format, cx)
            }
            Effect::SaveDefaultMapping { format, path } => {
                self.save_default_mapping(format, path, cx)
            }
            Effect::ListDocker { epoch } => self.list_docker(epoch, cx),
            Effect::ConnectSource(source) => self.connect_source(source, cx),
            Effect::SaveDltDevice { epoch, device } => {
                cx.spawn(async move |this, cx| {
                    let saved = device.clone();
                    let result = runtime::handle()
                        .spawn_blocking(move || crate::config::DltDevice::save(&saved))
                        .await
                        .unwrap_or_else(|error| Err(error.to_string()));
                    this.update_in(cx, |app, window, cx| {
                        app.dispatch(Message::DltDeviceSaved(epoch, device, result), window, cx)
                    })
                    .ok();
                })
                .detach();
            }
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
            Effect::ScrollHorizontal(column) => {
                self.horizontal_scroll.set_offset(gpui_kit::gpui::point(
                    -gpui_kit::gpui::px(column as f32 * 7.2),
                    gpui_kit::gpui::px(0.),
                ));
                cx.notify();
            }
            Effect::Quit => self.request_quit(cx),
            Effect::SaveKeybindings(bindings) => {
                cx.spawn(async move |this, cx| {
                    let saved = bindings.clone();
                    let result = runtime::handle()
                        .spawn_blocking(move || {
                            let path = dirs::config_dir()
                                .ok_or("No configuration directory")?
                                .join("logana/config.json");
                            crate::gui::keyboard::save_bindings(&path, &saved)
                        })
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r);
                    this.update(cx, |app, cx| {
                        match result {
                            Ok(()) => {
                                let kb = std::sync::Arc::new(bindings);
                                for tab in &mut app.state.tabs {
                                    tab.interaction.keybindings = kb.clone();
                                }
                                app.state.notify(crate::gui::state::StatusMessage::Info(
                                    "Keyboard shortcuts saved.".into(),
                                ));
                                crate::gui::menus::install(cx);
                            }
                            Err(error) => app
                                .state
                                .notify(crate::gui::state::StatusMessage::Error(error)),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            Effect::CopyText(text) => {
                cx.write_to_clipboard(gpui_kit::gpui::ClipboardItem::new_string(text))
            }
            Effect::OpenFileDialog => self.spawn_open_file_dialog(false, cx),
            Effect::OpenDirectoryDialog => self.spawn_open_file_dialog(true, cx),
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

    fn spawn_open_file_dialog(&self, directories: bool, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui_kit::gpui::PathPromptOptions {
            files: !directories,
            // Linux portals use directory mode whenever this flag is true,
            // even when files is also true. Keep the two native pickers separate.
            directories,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let paths = prompt.await.ok().and_then(Result::ok).flatten();
            this.update_in(cx, |app, window, cx| match paths {
                Some(paths) if !paths.is_empty() => {
                    for path in paths {
                        app.dispatch(Message::FileDialogResult(Some(path)), window, cx);
                    }
                }
                _ => app.dispatch(Message::FileDialogResult(None), window, cx),
            })
            .ok();
        })
        .detach();
    }

    fn spawn_load_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.queue_file(path, cx);
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
        let counts = cx.background_executor().spawn(async move {
            crate::shared::TabState::field_value_counts_for(&reader, parser.as_deref())
        });
        cx.spawn(async move |this, cx| {
            let result = counts.await;
            this.update(cx, |app, cx| {
                if let Some(entry) = app.facet_counts.get_mut(&id)
                    && entry.key == key
                {
                    entry.counts = Some(result);
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
        if self.inspected_row.is_some_and(|(source, line)| {
            !self
                .state
                .tabs
                .iter()
                .any(|tab| tab.id == source && line < tab.file_reader.line_count())
        }) {
            self.inspected_row = None;
        }
        crate::gui::view::bookmarks::sync(&mut self.state);
        if let Some((source, line)) = self.pending_source_jump {
            if self.state.active_tab().is_none_or(|tab| tab.id != source) {
                self.pending_source_jump = None;
            } else if self
                .state
                .active_tab()
                .is_some_and(|tab| tab.filter.visible_indices.contains(line))
            {
                crate::gui::view::bookmarks::jump(self, line);
                self.pending_source_jump = None;
            } else if self
                .state
                .active_tab()
                .is_some_and(|tab| tab.filter.handle.is_none())
            {
                self.pending_source_jump = None;
                self.state.notify(crate::gui::state::StatusMessage::Info(
                    "This source row is still hidden. Check continuation collapsing in View options.".into()));
            }
        }

        if self.notes_search.is_none()
            && self.state.nav_page == crate::gui::state::NavPage::Annotations
        {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search notes…"));
            cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
            self.notes_search = Some(input);
        }
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
                InputState::new(window, cx).placeholder("Search visible logs (regular expression)")
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
                        app.dispatch(Message::SearchPopup(false), window, cx);
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
            cx.subscribe(&input, |app, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    app.facet_pages.clear();
                }
                cx.notify();
            })
            .detach();
            self.facet_search = Some(input);
        }
        self.sync_archive_search(window, cx);
        if let Some(editor) = &self.state.editor {
            let values = editor.values.clone();
            let editor_action = editor.action;
            let epoch = editor.epoch;
            if self.editor_epoch != epoch || self.editor_inputs.is_empty() {
                self.editor_inputs.clear();
                self.editor_textareas.clear();
                self.editor_field_select = None;
                self.editor_choice_select = None;
                self.editor_group_select = None;
                self.editor_members = None;
                self.editor_dates.clear();
                self.editor_times.clear();
                self.editor_members = None;
                if editor_action == crate::gui::actions::Action::Group {
                    let members = &self.state.editor.as_ref().unwrap().members;
                    let names: Vec<_> = members
                        .iter()
                        .map(|(id, label, _)| format!("{id}: {label}"))
                        .collect();
                    let selected: Vec<_> = members
                        .iter()
                        .zip(&names)
                        .filter(|((_, _, on), _)| *on)
                        .map(|(_, name)| name.clone())
                        .collect();
                    let combo = cx.new(|cx| {
                        let mut combo =
                            ComboboxState::new(SearchableVec::new(names), vec![], window, cx)
                                .multiple(true)
                                .searchable(true);
                        combo.set_selected_values(&selected, window, cx);
                        combo
                    });
                    cx.subscribe_in(
                        &combo,
                        window,
                        move |app, _, event: &ComboboxEvent<SearchableVec<String>>, window, cx| {
                            if app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch) {
                                let values = match event {
                                    ComboboxEvent::Change(values)
                                    | ComboboxEvent::Confirm(values) => values,
                                };
                                let ids = values
                                    .iter()
                                    .filter_map(|v| {
                                        v.split_once(": ").and_then(|(id, _)| id.parse().ok())
                                    })
                                    .collect();
                                app.dispatch(Message::EditorMembers(ids), window, cx);
                            }
                        },
                    )
                    .detach();
                    self.editor_members = Some(combo);
                }
                if matches!(
                    editor_action,
                    crate::gui::actions::Action::Schema | crate::gui::actions::Action::Export
                ) {
                    let mut names = if editor_action == crate::gui::actions::Action::Export {
                        crate::commands::list_templates()
                    } else {
                        let mut names = vec!["none".to_string()];
                        names.extend(crate::parser::builtin_format_names());
                        names.extend(
                            crate::config::custom_schemas()
                                .iter()
                                .map(|s| s.name.clone()),
                        );
                        names
                    };
                    names.sort();
                    names.dedup();
                    let selected = values[0].clone();
                    let select = cx.new(|cx| {
                        let mut state =
                            SelectState::new(SearchableVec::new(names), None, window, cx)
                                .searchable(true);
                        state.set_selected_value(&selected, window, cx);
                        state
                    });
                    cx.subscribe_in(
                        &select,
                        window,
                        move |app, _, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                            if app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch) {
                                let SelectEvent::Confirm(Some(value)) = event else {
                                    return;
                                };
                                app.dispatch(Message::EditorText(0, value.clone()), window, cx);
                            }
                        },
                    )
                    .detach();
                    self.editor_choice_select = Some(select);
                }
                if editor_action == crate::gui::actions::Action::Filter {
                    let editor = self.state.editor.as_ref().unwrap();
                    let mut names = editor.discovery.names.clone();
                    names.extend(editor.conditions.iter().map(|(field, _)| field.clone()));
                    if !values[1].is_empty() {
                        names.push(values[1].clone());
                    }
                    names.sort();
                    names.dedup();
                    let selected = values[1].clone();
                    let select = cx.new(|cx| {
                        let mut state =
                            SelectState::new(SearchableVec::new(names), None, window, cx)
                                .searchable(true);
                        state.set_selected_value(&selected, window, cx);
                        state
                    });
                    cx.subscribe_in(
                        &select,
                        window,
                        move |app, _, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                            if app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch) {
                                let SelectEvent::Confirm(value) = event;
                                app.dispatch(Message::EditorMatchField(value.clone()), window, cx);
                            }
                        },
                    )
                    .detach();
                    self.editor_field_select = Some(select);
                    let mut groups = self
                        .state
                        .active_tab()
                        .map(|tab| tab.log_manager.group_names())
                        .unwrap_or_default();
                    if !values[3].is_empty() {
                        groups.push(values[3].clone());
                    }
                    groups.sort();
                    groups.dedup();
                    let selected = values[3].clone();
                    let select = cx.new(|cx| {
                        let mut state =
                            SelectState::new(SearchableVec::new(groups), None, window, cx)
                                .searchable(true);
                        state.set_selected_value(&selected, window, cx);
                        state
                    });
                    cx.subscribe_in(
                        &select,
                        window,
                        move |app, _, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                            if app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch) {
                                let SelectEvent::Confirm(value) = event;
                                app.dispatch(
                                    Message::EditorText(3, value.clone().unwrap_or_default()),
                                    window,
                                    cx,
                                );
                            }
                        },
                    )
                    .detach();
                    self.editor_group_select = Some(select);
                    for index in 0..2 {
                        let date = cx.new(|cx| {
                            DatePickerState::new(window, cx)
                                .date_format("%Y-%m-%d")
                                .time_precision(TimePrecision::Second)
                        });
                        cx.subscribe_in(
                            &date,
                            window,
                            move |app, _, event: &DatePickerEvent, window, cx| {
                                if app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch) {
                                    let DatePickerEvent::Change(value) = event;
                                    let value = value
                                        .start()
                                        .map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string())
                                        .unwrap_or_default();
                                    app.dispatch(
                                        Message::EditorTimestampPart(index + 1, value),
                                        window,
                                        cx,
                                    );
                                }
                            },
                        )
                        .detach();
                        self.editor_dates.push(date);
                        let time = cx.new(|cx| {
                            TimeFieldState::new(window, cx).precision(TimePrecision::Second)
                        });
                        cx.subscribe_in(
                            &time,
                            window,
                            move |app, _, event: &TimeFieldEvent, window, cx| {
                                if app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch) {
                                    let TimeFieldEvent::Change(value) = event;
                                    app.dispatch(
                                        Message::EditorTimestampPart(
                                            index + 1,
                                            value.format("%H:%M:%S").to_string(),
                                        ),
                                        window,
                                        cx,
                                    );
                                }
                            },
                        )
                        .detach();
                        self.editor_times.push(time);
                    }
                }
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
                            if matches!(event, InputEvent::PressEnter { .. })
                                && app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch)
                            {
                                app.dispatch(Message::EditorApply, window, cx);
                                return;
                            }
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
                self.editor_colors.clear();
                for index in 4..6 {
                    let picker = cx.new(|cx| {
                        gpui_kit::component::color_picker::ColorPickerState::new(window, cx)
                    });
                    cx.subscribe_in(
                        &picker,
                        window,
                        move |app,
                              _,
                              event: &gpui_kit::component::color_picker::ColorPickerEvent,
                              window,
                              cx| {
                            if app.state.editor.as_ref().is_some_and(|e| e.epoch == epoch) {
                                let gpui_kit::component::color_picker::ColorPickerEvent::Change(
                                    value,
                                ) = event;
                                let value = value
                                    .map(|color| {
                                        let color: gpui_kit::gpui::Rgba = color.into();
                                        // Filter colors store opaque RGB. Round channels to avoid
                                        // losing a byte in the picker's HSL round trip.
                                        format!(
                                            "#{:02X}{:02X}{:02X}",
                                            (color.r * 255.).round() as u8,
                                            (color.g * 255.).round() as u8,
                                            (color.b * 255.).round() as u8
                                        )
                                    })
                                    .unwrap_or_default();
                                app.dispatch(Message::EditorText(index, value), window, cx);
                            }
                        },
                    )
                    .detach();
                    self.editor_colors.push(picker);
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
            if let Some(editor) = &self.state.editor {
                if editor.values[0].starts_with(crate::filters::DATE_PREFIX) {
                    let (_, start, end) =
                        crate::gui::time_range::timestamp_parts(&editor.values[0]);
                    for (index, bound) in [start, end].into_iter().enumerate() {
                        let normalized = if bound.len() == 10 {
                            format!("{bound}T00:00:00")
                        } else if bound.len() == 16 {
                            format!("{}:00", bound.replace(' ', "T"))
                        } else {
                            bound.replace(' ', "T")
                        };
                        let value = DateTime::Single(normalized.parse().ok());
                        if let Some(date) = self.editor_dates.get(index)
                            && date.read(cx).date_time() != value
                        {
                            date.update(cx, |date, cx| date.set_date_time(value, window, cx));
                        }
                        let normalized_time = if bound.len() == 5 {
                            format!("{bound}:00")
                        } else {
                            bound.to_owned()
                        };
                        if let Ok(value) = normalized_time.parse()
                            && let Some(time) = self.editor_times.get(index)
                            && time.read(cx).time() != value
                        {
                            time.update(cx, |time, cx| time.set_time(value, window, cx));
                        }
                    }
                }

                if let Some(select) = &self.editor_group_select {
                    let selected = editor.values[3].clone();
                    if select.read(cx).selected_value() != Some(&selected)
                        && !(selected.is_empty() && select.read(cx).selected_value().is_none())
                    {
                        select.update(cx, |select, cx| {
                            select.set_selected_value(&selected, window, cx)
                        });
                    }
                }
                if let Some(select) = &self.editor_field_select {
                    let selected = editor.values[1].clone();
                    if select.read(cx).selected_value() != Some(&selected)
                        && !(selected.is_empty() && select.read(cx).selected_value().is_none())
                    {
                        select.update(cx, |select, cx| {
                            select.set_selected_value(&selected, window, cx)
                        });
                    }
                }
                for (picker, value) in self.editor_colors.iter().zip(&editor.values[4..6]) {
                    let color: Option<gpui_kit::gpui::Hsla> = crate::theme::parse_color(value)
                        .and_then(crate::gui::color::color_to_gpui)
                        .map(Into::into);
                    if picker.read(cx).value() != color {
                        picker.update(cx, |picker, cx| {
                            if let Some(color) = color {
                                picker.set_value(color, window, cx);
                            } else {
                                picker.clear_value(window, cx);
                            }
                        });
                    }
                }
            }
        } else {
            self.editor_field_select = None;
            self.editor_choice_select = None;
            self.editor_group_select = None;
            self.editor_members = None;
            self.editor_dates.clear();
            self.editor_times.clear();
            self.editor_colors.clear();
            self.editor_inputs.clear();
            self.editor_textareas.clear();
            self.editor_synced.clear();
        }
        self.text_document
            .borrow_mut()
            .sync(self.text_handle.snapshot(cx));
        self.sync_table_preparation(cx);
        let text_context = self.state.active_tab().map(|tab| {
            let fields = crate::gui::column_widths::cached_field_names(tab);
            let columns = crate::gui::view::log_pane::visible_columns(tab, &fields);
            (
                tab.id,
                tab.cache.parse_gen,
                tab.cache.render_gen,
                columns,
                tab.display.show_keys,
                tab.display.raw_mode,
            )
        });
        if self.text_context != text_context {
            gpui_kit::base::TextSelection::clear(window, cx);
            let mut document = self.text_document.borrow_mut();
            document.clear();
            document.tab = text_context.as_ref().map(|c| c.0);
            document.columns = text_context
                .as_ref()
                .map(|c| c.3.clone())
                .unwrap_or_default();
            self.text_context = text_context;
        }
        self.sync_field_discovery(cx);
        self.sync_schema_preview(cx);
        self.sync_export_preview(cx);
        let focused = |input: &Option<Entity<InputState>>| {
            input
                .as_ref()
                .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window))
        };
        let editing = focused(&self.log_search)
            || focused(&self.facet_search)
            || focused(&self.notes_search)
            || focused(&self.archive_search);
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
            .text_size(gpui_kit::gpui::rems(1.))
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::on_open_file_action))
            .on_action(cx.listener(Self::on_open_commands_action))
            .on_action(cx.listener(
                |app, action: &crate::gui::menus::InvokeAction, window, cx| {
                    // Editors opened from a menu must return to a surviving
                    // focus handle rather than the dismissed popup's handle.
                    app.root_focus.focus(window, cx);
                    app.dispatch(Message::GuiAction(action.action), window, cx);
                },
            ))
            .on_action(cx.listener(Self::on_quit_action))
            .capture_key_down(cx.listener(move |app, event: &KeyDownEvent, window, cx| {
                let stroke = &event.keystroke;
                if let Some(index) = app.state.binding_capture {
                    cx.stop_propagation();
                    app.state.binding_capture = None;
                    if stroke.key != "escape"
                        && let Some((key, modifiers)) = crate::gui::key::from_gpui_keystroke(stroke)
                    {
                        let value =
                            crate::config::keybindings::KeyBinding(key, modifiers).display();
                        app.dispatch(Message::EditorText(index, value), window, cx);
                    }
                    cx.notify();
                    return;
                }

                if stroke.modifiers.control && !stroke.modifiers.alt && !stroke.modifiers.platform {
                    let delta = match stroke.key.as_str() {
                        "+" | "=" | "plus" => Some(1.),
                        "-" | "minus" => Some(-1.),
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        let size = (f32::from(window.rem_size()) + delta).clamp(10., 32.);
                        let mut preferences = app.state.preferences.clone();
                        preferences.font_size = size;
                        preferences.apply_font_size(window, cx);
                        app.save_preferences(preferences);
                        app.log_scroll.remeasure();
                        app.text_document.borrow_mut().clear();
                        app.text_context = None;
                        gpui_kit::base::TextSelection::clear(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                }

                if let Some(editor) = &app.state.editor
                    && let Some((key, modifiers)) = crate::gui::key::from_gpui_keystroke(stroke)
                {
                    let kb = crate::gui::keyboard::bindings(&app.state);
                    let (confirm, cancel) = match editor.action {
                        crate::gui::actions::Action::Docker => {
                            (&kb.docker_select.confirm, &kb.docker_select.cancel)
                        }
                        crate::gui::actions::Action::Dlt => {
                            (&kb.dlt_select.confirm, &kb.dlt_select.cancel)
                        }
                        crate::gui::actions::Action::ValueColors => {
                            (&kb.value_colors.apply, &kb.value_colors.cancel)
                        }
                        crate::gui::actions::Action::Columns
                        | crate::gui::actions::Action::Merge => {
                            (&kb.select_fields.apply, &kb.select_fields.cancel)
                        }
                        _ => (&kb.filter_edit.confirm, &kb.filter_edit.cancel),
                    };
                    let typing = matches!(key, crate::input::KeyCode::Char(_))
                        && !modifiers.contains(crate::input::KeyModifiers::CONTROL)
                        && !modifiers.contains(crate::input::KeyModifiers::ALT)
                        && (app
                            .editor_inputs
                            .iter()
                            .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
                            || app
                                .editor_textareas
                                .values()
                                .any(|input| input.read(cx).focus_handle(cx).is_focused(window)));
                    // Let native pickers and inputs handle Enter before it bubbles to Apply.
                    if !typing
                        && key != crate::input::KeyCode::Enter
                        && confirm.matches(key, modifiers)
                    {
                        cx.stop_propagation();
                        app.dispatch(Message::EditorApply, window, cx);
                        return;
                    }
                    if !typing && cancel.matches(key, modifiers) {
                        cx.stop_propagation();
                        app.dispatch(Message::EditorCancel, window, cx);
                        return;
                    }
                }
                if app
                    .state
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.action == crate::gui::actions::Action::Annotation)
                    && let Some((key, modifiers)) = crate::gui::key::from_gpui_keystroke(stroke)
                {
                    let kb = crate::gui::keyboard::bindings(&app.state);
                    if kb.comment.save.matches(key, modifiers) {
                        cx.stop_propagation();
                        app.dispatch(Message::EditorApply, window, cx);
                        return;
                    }
                    if kb.comment.cancel.matches(key, modifiers) {
                        cx.stop_propagation();
                        app.dispatch(Message::EditorCancel, window, cx);
                        return;
                    }
                }
                if stroke.key == "escape" && app.state.visual_lines && context == "LoganaLogs" {
                    app.state.visual_lines = false;
                    app.state.log_selection = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if stroke.key == "escape" {
                    if app.state.editor.is_none()
                        && !app.state.palette.visible()
                        && !app.state.time_range_open
                        && !app.state.search_open
                        && app.state.keyboard_sidebar.take().is_some()
                    {
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if app.state.editor.is_some() {
                        cx.stop_propagation();
                        app.dispatch(Message::EditorCancel, window, cx);
                        return;
                    }
                    if app.state.palette.visible() {
                        cx.stop_propagation();
                        app.dispatch(Message::PaletteClose, window, cx);
                        return;
                    }
                    if app.state.time_range_open {
                        cx.stop_propagation();
                        app.state.time_range_open = false;
                        app.root_focus.focus(window, cx);
                        cx.notify();
                        return;
                    }
                }
                if app.state.search_open && stroke.key == "enter" {
                    cx.stop_propagation();
                    app.dispatch(Message::SearchNext(!stroke.modifiers.shift), window, cx);
                    app.dispatch(Message::SearchPopup(false), window, cx);
                    return;
                }
                if app.state.search_open && stroke.key == "escape" {
                    cx.stop_propagation();
                    // Clearing the input first makes `dispatch`'s own
                    // close-time sync (below) see a changed, empty query and
                    // route it through `SearchEdited` into `cancel_search` —
                    // so Escape discards the search instead of just hiding
                    // the popup with results and highlights left behind.
                    if let Some(input) = &app.log_search {
                        input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
                    }
                    app.dispatch(Message::SearchPopup(false), window, cx);
                    return;
                }
                if !editing
                    && context == "LoganaLogs"
                    && stroke.key == "/"
                    && !stroke.modifiers.control
                    && !stroke.modifiers.platform
                    && !stroke.modifiers.alt
                {
                    cx.stop_propagation();
                    app.dispatch(Message::SearchPopup(true), window, cx);
                    return;
                }
                if app.loading.focus.is_focused(window) && app.picker_key(&stroke.key, cx) {
                    cx.stop_propagation();
                    return;
                }
                if !editing
                    && context == "LoganaLogs"
                    && stroke.key == "c"
                    && (stroke.modifiers.control || stroke.modifiers.platform)
                {
                    app.text_document
                        .borrow_mut()
                        .sync(app.text_handle.snapshot(cx));
                    let text = app
                        .state
                        .active_tab()
                        .map(|tab| app.text_document.borrow().copy(tab))
                        .unwrap_or_default();
                    if !text.is_empty() {
                        cx.write_to_clipboard(gpui_kit::gpui::ClipboardItem::new_string(text));
                        cx.stop_propagation();
                        return;
                    }
                }
                if stroke.key == "escape" {
                    if app.state.editor.is_some() {
                        // Let a Select or date picker dismiss its popup first.
                        return;
                    } else if app.state.palette.visible() {
                        cx.stop_propagation();
                        app.dispatch(Message::PaletteClose, window, cx);
                    } else if app.state.time_range_open {
                        app.state.time_range_open = false;
                        cx.notify();
                    } else if !editing && context == "LoganaLogs" {
                        cx.stop_propagation();
                        if let Some(input) = &app.log_search {
                            input
                                .update(cx, |input, cx| input.set_value(String::new(), window, cx));
                        }
                        app.dispatch(Message::SearchEdited(String::new()), window, cx);
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
                    && (stroke.modifiers.control || stroke.modifiers.platform)
                    && stroke.key == "z"
                {
                    cx.stop_propagation();
                    app.dispatch(
                        Message::GuiAction(if stroke.modifiers.shift {
                            crate::gui::actions::Action::Redo
                        } else {
                            crate::gui::actions::Action::Undo
                        }),
                        window,
                        cx,
                    );
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
                if !editing
                    && context == "LoganaLogs"
                    && app.state.keyboard_sidebar.is_some()
                    && let Some((key, modifiers)) = crate::gui::key::from_gpui_keystroke(stroke)
                    && crate::gui::keyboard::sidebar_handled(&app.state, key, modifiers)
                {
                    cx.stop_propagation();
                    app.dispatch(Message::KeyPressed(key, modifiers), window, cx);
                    return;
                }
                if !editing
                    && context == "LoganaLogs"
                    && let Some((key, modifiers)) = crate::gui::key::from_gpui_keystroke(stroke)
                    && crate::gui::keyboard::handled(&app.state, key, modifiers)
                {
                    if let Some(tab) = app.state.active_tab_mut() {
                        tab.scroll.visible_height =
                            ((f32::from(window.viewport_size().height) - 160.) / 28.).max(1.)
                                as usize;
                    }
                    cx.stop_propagation();
                    app.dispatch(Message::KeyPressed(key, modifiers), window, cx);
                    return;
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
            .on_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" && app.state.editor.is_some() {
                    let multiline = app
                        .editor_textareas
                        .values()
                        .any(|input| input.read(cx).focus_handle(cx).is_focused(window));
                    if !multiline
                        || event.keystroke.modifiers.control
                        || event.keystroke.modifiers.platform
                    {
                        cx.stop_propagation();
                        app.dispatch(Message::EditorApply, window, cx);
                        return;
                    }
                }
                if event.keystroke.key == "escape" && app.state.editor.is_some() {
                    cx.stop_propagation();
                    app.dispatch(Message::EditorCancel, window, cx);
                }
            }))
            .child(crate::gui::view::view(self, window, cx))
    }
}
