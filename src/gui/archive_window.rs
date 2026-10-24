use crate::gui::app::App;
use gpui_kit::{
    base::Root,
    component::{ActiveTheme, TitleBar},
    gpui::{
        self, AppContext, Context, IntoElement, Render, WeakEntity, Window, WindowHandle, div,
        prelude::*, px,
    },
};

pub(super) type ArchiveWindowHandle = WindowHandle<Root>;

struct ArchiveWindow {
    app: WeakEntity<App>,
}

impl Render for ArchiveWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else {
            window.remove_window();
            return div().into_any_element();
        };
        let content = app.update(cx, |app, cx| {
            if app.loading.selected_picker.is_none() {
                window.remove_window();
                return div().into_any_element();
            }
            app.sync_archive_search(window, cx);
            crate::gui::view::loading::picker(app, cx).unwrap_or_else(|| {
                div()
                    .p_4()
                    .children(crate::gui::view::loading::operations(app, cx))
                    .into_any_element()
            })
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .child(
                TitleBar::new()
                    .child("Archive files")
                    .on_close_window(cx.listener(|this, _, window, cx| {
                        this.app
                            .update(cx, |app, cx| {
                                app.loading.selected_picker = None;
                                cx.notify();
                            })
                            .ok();
                        window.remove_window();
                    })),
            )
            .child(div().flex_1().min_h(px(0.)).child(content))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let handled = this
                    .app
                    .update(cx, |app, cx| {
                        app.loading.focus.is_focused(window)
                            && app.picker_key(&event.keystroke.key, cx)
                    })
                    .unwrap_or(false);
                if handled {
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
    }
}

impl App {
    pub(super) fn show_archive_picker(&mut self, id: u64, cx: &mut Context<Self>) {
        self.loading.selected_picker = Some(id);
        if self.loading.picker_window_opening {
            cx.notify();
            return;
        }
        self.loading.picker_window_opening = true;
        let existing = self.loading.picker_window;
        let app = cx.entity().downgrade();
        cx.spawn(async move |this, cx| {
            let result = cx
                .update(|cx| {
                    if let Some(window) = existing
                        && window
                            .update(cx, |_, window, _| window.activate_window())
                            .is_ok()
                    {
                        return Ok(window);
                    }
                    open_archive_window(app, cx)
                })
                .map_err(|error| error.to_string());
            this.update(cx, |app, cx| {
                app.loading.picker_window_opening = false;
                match result {
                    Ok(window) => app.loading.picker_window = Some(window),
                    Err(error) => {
                        app.state
                            .notify(crate::gui::state::StatusMessage::Error(format!(
                                "Cannot open archive selector: {error}"
                            )))
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

fn open_archive_window(
    app: WeakEntity<App>,
    cx: &mut gpui::App,
) -> Result<ArchiveWindowHandle, String> {
    let closing = app.clone();
    let options = gpui::WindowOptions {
        window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
            None,
            gpui::size(px(1000.), px(760.)),
            cx,
        ))),
        window_min_size: Some(gpui::size(px(560.), px(640.))),
        ..TitleBar::window_options()
    };
    let (window, _) = gpui_kit::open_window(options, cx, move |window, cx| {
        window.on_window_should_close(cx, move |_, cx| {
            closing
                .update(cx, |app, cx| {
                    app.loading.selected_picker = None;
                    app.loading.picker_window = None;
                    cx.notify();
                })
                .ok();
            true
        });
        if let Some(entity) = app.upgrade() {
            entity.update(cx, |app, cx| app.loading.focus.focus(window, cx));
        }
        cx.new(|cx| {
            if let Some(entity) = app.upgrade() {
                cx.observe(&entity, |_, _, cx| cx.notify()).detach();
            }
            ArchiveWindow { app }
        })
    })
    .map_err(|error| error.to_string())?;
    window
        .downcast::<Root>()
        .ok_or_else(|| "Invalid archive window".into())
}
