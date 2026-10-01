use crate::commands::Commands;
use crate::db::{Database, LogManager};
use crate::gui::effect::Effect;
use crate::gui::key;
use crate::gui::message::Message;
use crate::gui::runtime;
use crate::gui::state::GuiState;
use crate::gui::update;
use crate::gui::update::{NormalAction, ScrollTarget};
use crate::theme::Theme as TuiTheme;
use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::menu::{AppMenuBar, ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    App as GpuiApp, ClickEvent, Context, Entity, FocusHandle, KeyDownEvent, ScrollStrategy, Window,
    actions, div, px,
};
use std::path::PathBuf;
use std::sync::Arc;

// Actions bound once at startup (see `src/bin/logana_gui.rs`) so the File
// menu and global keybindings (`q`, `Ctrl+O`) both reach the same handlers
// as a normal keypress would, via gpui's own action-dispatch system rather
// than only through the capture-phase handler below.
actions!(logana_gui, [OpenFile, Quit]);

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
    pub menu_bar: Entity<AppMenuBar>,
}

impl App {
    pub fn new(db: Arc<Database>, theme: TuiTheme, cx: &mut Context<Self>) -> Self {
        Self {
            state: GuiState::new(db).with_theme(theme),
            command_bar_focus: cx.focus_handle(),
            log_scroll: VirtualListScrollHandle::new(),
            menu_bar: AppMenuBar::new(cx),
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

/// One context-menu item that runs a `NormalAction` — the label shows the
/// same keybinding hint the mode bar does, so the menu and the mode bar
/// never describe the bindings differently.
fn normal_action_item(
    label: &'static str,
    action: NormalAction,
    my_app: &Entity<App>,
) -> PopupMenuItem {
    let my_app = my_app.clone();
    PopupMenuItem::new(label).on_click(
        move |_: &ClickEvent, window: &mut Window, cx: &mut GpuiApp| {
            my_app.update(cx, |app, cx| {
                app.dispatch(Message::RunNormalAction(action), window, cx);
            });
        },
    )
}

/// Builds the right-click context menu: the same Normal-mode actions the
/// mode bar advertises (`gg`/`G`/Tab/Shift-Tab/`:`), plus Open/Quit —
/// mirroring the TUI's keybindings rather than inventing GUI-only ones.
fn build_context_menu(
    my_app: Entity<App>,
    menu: PopupMenu,
    _window: &mut Window,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    menu.item(normal_action_item(
        "Scroll to top (gg)",
        NormalAction::Scroll(ScrollTarget::Top),
        &my_app,
    ))
    .item(normal_action_item(
        "Scroll to bottom (G)",
        NormalAction::Scroll(ScrollTarget::Bottom),
        &my_app,
    ))
    .separator()
    .item(normal_action_item(
        "Next tab (Tab)",
        NormalAction::NextTab,
        &my_app,
    ))
    .item(normal_action_item(
        "Previous tab (Shift-Tab)",
        NormalAction::PrevTab,
        &my_app,
    ))
    .separator()
    .item(normal_action_item(
        "Command mode (:)",
        NormalAction::EnterCommandMode,
        &my_app,
    ))
    .separator()
    .menu("Open...", Box::new(OpenFile))
    .menu("Quit (q)", Box::new(Quit))
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let my_app = cx.entity();
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::on_open_file_action))
            .on_action(cx.listener(Self::on_quit_action))
            .capture_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                let (key, modifiers) = key::from_gpui_keystroke(&event.keystroke);
                if update::should_capture(&app.state.mode, &key) {
                    cx.stop_propagation();
                    app.dispatch(Message::KeyPressed(key, modifiers), window, cx);
                }
            }))
            .context_menu(move |menu, window, cx| {
                build_context_menu(my_app.clone(), menu, window, cx)
            })
            .child(div().h(px(28.)).child(self.menu_bar.clone()))
            .child(
                div()
                    .flex_1()
                    .p(px(8.))
                    .child(crate::gui::view::view(self, window, cx)),
            )
    }
}
