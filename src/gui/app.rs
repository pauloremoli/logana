use crate::commands::Commands;
use crate::db::{Database, LogManager};
use crate::gui::effect::Effect;
use crate::gui::key;
use crate::gui::message::Message;
use crate::gui::runtime;
use crate::gui::state::GuiState;
use crate::gui::update;
use crate::theme::Theme as TuiTheme;
use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, FocusHandle, KeyDownEvent, ScrollStrategy, Window, div, px};
use std::path::PathBuf;
use std::sync::Arc;

/// The only place in the GUI that touches gpui types directly. Owns the
/// framework-neutral `GuiState` and translates between it and gpui: a
/// capture-phase key handler feeds `Message`s into `update()`, and
/// `apply_effect` carries out whatever `Effect` comes back — spawning
/// async work on the shared tokio runtime (see `runtime`) or driving a
/// gpui widget (focus, quit, scroll) directly.
pub struct App {
    pub state: GuiState,
    pub command_bar_focus: FocusHandle,
    pub log_scroll: VirtualListScrollHandle,
}

impl App {
    pub fn new(db: Arc<Database>, theme: TuiTheme, cx: &mut Context<Self>) -> Self {
        Self {
            state: GuiState::new(db).with_theme(theme),
            command_bar_focus: cx.focus_handle(),
            log_scroll: VirtualListScrollHandle::new(),
        }
    }

    pub fn dispatch(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        let effect = update::update(&mut self.state, message);
        self.apply_effect(effect, window, cx);
        cx.notify();
    }

    fn apply_effect(&mut self, effect: Effect, window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            Effect::None => {}
            Effect::Quit => cx.quit(),
            Effect::FocusCommandBar => self.command_bar_focus.focus(window, cx),
            Effect::Scroll(line) => {
                self.log_scroll.scroll_to_item(line, ScrollStrategy::Top);
            }
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
            Effect::ExecuteCommand {
                tab_idx,
                log_manager,
                command,
            } => self.spawn_execute_command(tab_idx, log_manager, command, cx),
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
        cx.spawn(async move |this, cx| {
            let log_manager = runtime::handle()
                .spawn(async move {
                    log_manager.remove_group(&name).await;
                    log_manager
                })
                .await
                .expect("remove_group task panicked");
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::GroupsMutated(tab_idx, log_manager), window, cx);
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
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = runtime::handle()
                .spawn(async move { update::execute_command(log_manager, command).await })
                .await
                .expect("execute_command task panicked");
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::CommandExecuted(tab_idx, result), window, cx);
            })
            .ok();
        })
        .detach();
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(8.))
            .capture_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                let (key, modifiers) = key::from_gpui_keystroke(&event.keystroke);
                if update::should_capture(&app.state.mode, &key) {
                    cx.stop_propagation();
                    app.dispatch(Message::KeyPressed(key, modifiers), window, cx);
                }
            }))
            .child(crate::gui::view::view(self, window, cx))
    }
}
