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
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    Context, Entity, FocusHandle, KeyDownEvent, ScrollStrategy, Window, actions, div,
};
use std::path::PathBuf;
use std::sync::Arc;

// Actions bound once at startup (see `src/bin/logana_gui.rs`) so the File
// menu and global keybindings (`Ctrl+O`, `Ctrl+Q`) both reach the same
// handlers as a normal keypress would, via gpui's own action-dispatch
// system rather than only through the capture-phase handler below. `q`
// quitting Normal mode is handled entirely by the real `Mode`/keybindings
// machinery now (`KeyResult::Ignored` -> `handle_global_key`), same as the
// TUI — these actions are just the GUI-native (File menu, Ctrl+Q) paths.
actions!(logana_gui, [OpenFile, Quit]);

/// The only place in the GUI that touches gpui types directly. Owns the
/// framework-neutral `GuiState` (which now holds the TUI's own
/// `ui::TabState`/`Box<dyn Mode>` per tab — see `gui::state`) and
/// translates between it and gpui: a capture-phase key handler feeds
/// `Message`s into `update()`, and `apply_effect` carries out whatever
/// `Effect` comes back — spawning async work on the shared tokio runtime
/// (see `runtime`) or driving a gpui widget (quit, scroll) directly.
pub struct App {
    pub state: GuiState,
    pub log_scroll: VirtualListScrollHandle,
    pub menu_bar: Entity<AppMenuBar>,
    /// Tracked by the root element (`Render::render`) so every keystroke
    /// has somewhere to land even when nothing else (a button, the menu
    /// bar) is focused. Reclaimed after every dispatch — see
    /// `reclaim_focus_if_idle` — so a transient focus change elsewhere
    /// (clicking a button, closing a native dialog) never leaves the
    /// window unable to receive the next keystroke.
    pub root_focus: FocusHandle,
}

impl App {
    pub fn new(db: Arc<Database>, theme: TuiTheme, cx: &mut Context<Self>) -> Self {
        Self {
            state: GuiState::new(db).with_theme(theme),
            log_scroll: VirtualListScrollHandle::new(),
            menu_bar: AppMenuBar::new(cx),
            root_focus: cx.focus_handle(),
        }
    }

    fn on_open_file_action(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Message::OpenFileDialog, window, cx);
    }

    fn on_quit_action(&mut self, _: &Quit, _window: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    pub fn dispatch(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        let effect = update::update(&mut self.state, message);
        self.apply_effect(effect, window, cx);
        self.sync_log_scroll();
        self.reclaim_focus_if_idle(window, cx);
        cx.notify();
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

    /// The active tab's `Mode`/global-key handling owns `scroll.scroll_offset`
    /// directly (same as the TUI) rather than returning it as an `Effect` —
    /// simpler to just always sync the gpui virtual list to match after
    /// every dispatch than to thread it through every possible `Effect`
    /// variant. Idempotent and cheap when scrolling didn't change.
    fn sync_log_scroll(&self) {
        if let Some(tab) = self.state.active_tab() {
            self.log_scroll
                .scroll_to_item(tab.scroll.scroll_offset, ScrollStrategy::Top);
        }
    }

    fn apply_effect(&mut self, effect: Effect, _window: &mut Window, cx: &mut Context<Self>) {
        match effect {
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
                tab_idx,
                log_manager,
                command,
                theme_bg,
                filter_context,
            } => self.spawn_execute_command(
                tab_idx,
                log_manager,
                command,
                theme_bg,
                filter_context,
                cx,
            ),
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

    fn spawn_remove_filter(
        &self,
        tab_idx: usize,
        mut log_manager: LogManager,
        id: usize,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let log_manager = runtime::handle()
                .spawn(async move {
                    log_manager.remove_filter(id).await;
                    log_manager
                })
                .await
                .expect("remove_filter task panicked");
            this.update_in(cx, |app, window, cx| {
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
        cx.spawn(async move |this, cx| {
            let result = runtime::handle()
                .spawn(async move {
                    update::execute_command(log_manager, command, theme_bg, filter_context).await
                })
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
            .track_focus(&self.root_focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::on_open_file_action))
            .on_action(cx.listener(Self::on_quit_action))
            .capture_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                // Captures every key press application-wide — nothing in
                // this GUI holds gpui widget focus for typing (the
                // command bar included), so every key flows through here
                // and into `Mode::handle_key`, exactly like the TUI's
                // terminal-wide key capture. `track_focus` above plus
                // `reclaim_focus_if_idle` (called after every dispatch)
                // keep the root focused so this keeps firing reliably.
                if let Some((key, modifiers)) = key::from_gpui_keystroke(&event.keystroke) {
                    cx.stop_propagation();
                    app.dispatch(Message::KeyPressed(key, modifiers), window, cx);
                }
            }))
            .child(crate::gui::view::view(self, window, cx))
    }
}
