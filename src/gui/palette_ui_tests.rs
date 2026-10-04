//! Native event tests: exercise the real Input, focus, hit testing and clipboard.
use super::app::{App, OpenCommands};
use super::{runtime, theme};
use crate::db::Database;
use crate::theme::Theme;
use gpui::{
    AppContext, Bounds, Entity, KeyBinding, TestAppContext, WindowBounds, WindowHandle,
    WindowOptions, point, px, size,
};
use gpui_kit::base::Root;
use gpui_kit::gpui;
use gpui_kit::test::TestWindowExt;
use std::sync::Arc;

fn palette_window(
    cx: &mut TestAppContext,
    dimensions: (f32, f32),
    name: &str,
) -> (WindowHandle<Root>, Entity<App>) {
    let db = Arc::new(runtime::handle().block_on(Database::in_memory()).unwrap());
    let theme = Theme::from_file(format!("{name}.json")).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(&theme, cx);
        cx.bind_keys([KeyBinding::new(
            "ctrl-shift-p",
            OpenCommands,
            Some("LoganaLogs"),
        )]);
        let (window, app) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(dimensions.0), px(dimensions.1)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let app = cx.new(|cx| App::new(db, theme, cx));
                app.update(cx, |app, cx| app.root_focus.focus(window, cx));
                app
            },
        )
        .unwrap();
        (window.downcast::<Root>().unwrap(), app)
    })
}
fn add_log_tab(app: &Entity<App>, cx: &mut gpui::App, structured: bool) {
    let manager = runtime::handle().block_on(crate::db::LogManager::new(
        app.read(cx).state.db.clone(),
        None,
    ));
    let reader = crate::ingestion::FileReader::from_bytes(if structured {
        br#"{"level":"INFO","message":"hello","service":"payment"}"#.to_vec()
    } else {
        b"plain\n".to_vec()
    });
    app.update(cx, |app, cx| {
        app.state.tabs.push(crate::shared::TabState::new(
            reader,
            manager,
            "test.log".into(),
        ));
        let tab = &app.state.tabs[0];
        let key = (
            tab.file_reader.line_count(),
            tab.display
                .format
                .as_ref()
                .map(|p| p.name().to_string())
                .unwrap_or_default(),
        );
        let counts = crate::shared::TabState::field_value_counts_for(
            &tab.file_reader,
            tab.display.format.as_deref(),
        );
        app.facet_counts.insert(
            tab.id,
            super::app::FacetCache {
                key,
                counts: Some(counts),
            },
        );
        cx.notify();
    });
}

#[gpui_kit::test]
fn launcher_is_native_search_and_tui_keys_do_not_execute(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("q", cx);
        window.press("j", cx);
        window.press("f", cx);
        assert!(!app.read(cx).state.palette.open);
        assert!(app.read(cx).state.editor.is_none());
        window.press("ctrl-shift-p", cx);
        assert_eq!(window.find("command-input").focused(), Some(true));
        window.input("theme", cx);
        window.press("enter", cx);
        assert!(app.read(cx).state.editor.is_none());
        window.click(("command-suggestion", 0usize), cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().action,
            crate::gui::actions::Action::Appearance
        );
        window.press("ctrl-shift-p", cx);
        assert!(!app.read(cx).state.palette.open);
        window.click("editor-cancel", cx);
        assert!(!app.read(cx).state.palette.open);
        window.press("ctrl-shift-p", cx);
        assert!(app.read(cx).state.palette.open);
        window.press("escape", cx);
        assert!(!app.read(cx).state.palette.open);
    })
    .unwrap();
}
#[gpui_kit::test]
fn colon_types_in_native_inputs_and_opens_launcher_on_logs(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        window.render_frame(cx);
        window.click("log-search-input", cx);
        window.input("value:東京", cx);
        assert!(!app.read(cx).state.palette.open);
        assert_eq!(window.find("log-search-input").value(), Some("value:東京"));
        app.update(cx, |app, cx| app.root_focus.focus(window, cx));
        window.press(":", cx);
        assert!(app.read(cx).state.palette.open);
        window.press("escape", cx);
        window.press("q", cx);
        assert!(!app.read(cx).state.palette.open);
    })
    .unwrap();
}
#[gpui_kit::test]
fn columns_are_mouse_operated_and_cancel_preserves_preferences(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        window.render_frame(cx);
        window.click("open-commands", cx);
        window.input("select-fields", cx);
        window.click(("command-suggestion", 0usize), cx);
        assert!(app.read(cx).state.editor.is_some());
        window.click("editor-none", cx);
        window.click("editor-cancel", cx);
        assert!(app.read(cx).state.tabs[0].display.hidden_fields.is_empty());
        assert!(!app.read(cx).state.palette.open);
        window.click("open-commands", cx);
        window.input("select-fields", cx);
        window.click(("command-suggestion", 0usize), cx);
        window.click("editor-none", cx);
        window.click("editor-apply", cx);
        assert!(!app.read(cx).state.tabs[0].display.hidden_fields.is_empty());
        assert!(!app.read(cx).state.palette.open);
        assert!(app.read(cx).state.editor.is_none());
    })
    .unwrap();
}
#[gpui_kit::test]
fn results_copy_clear_reopen_and_narrow_layout(cx: &mut TestAppContext) {
    for name in ["dracula", "github-light"] {
        let (handle, app) = palette_window(cx, (480., 640.), name);
        cx.update_window(handle.into(), |_, window, cx| {
            app.update(cx, |app, _| {
                crate::gui::update::gui_outcome(
                    &mut app.state,
                    "Test action".into(),
                    crate::gui::palette::ExecutionState::Error,
                    "first line\nsecond line 東京".into(),
                )
            });
            window.render_frame(cx);
            window.click("open-commands", cx);
            let bounds = window.find("launcher-focus").bounds();
            assert!(bounds.size.width <= px(432.));
            assert!(bounds.size.height <= px(512.));
            window.click("command-copy", cx);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "first line\nsecond line 東京"
            );
            window.click("command-close", cx);
            window.click("open-commands", cx);
            assert!(app.read(cx).state.palette.outcome.is_some());
            window.click("command-clear", cx);
            assert!(app.read(cx).state.palette.outcome.is_none());
        })
        .unwrap();
    }
}
#[gpui_kit::test]
fn filter_form_shows_validation_without_terminal_mode(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "github-light");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        window.render_frame(cx);
        window.click("add-filter", cx);
        window.click("editor-apply", cx);
        assert!(app.read(cx).state.editor.as_ref().unwrap().error.is_some());
        assert!(matches!(
            app.read(cx).state.tabs[0].interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::Normal
        ));
        window.click("editor-cancel", cx);
        assert!(app.read(cx).state.editor.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn notifications_are_dismissible_and_expire_five_seconds_after_each_event(cx: &mut TestAppContext) {
    use crate::gui::message::Message;
    use std::time::Duration;
    let (handle, app) = palette_window(cx, (1000., 800.), "github-light");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        window.render_frame(cx);
        window.click("log-search-input", cx);
        app.update(cx, |app, cx| {
            app.dispatch(
                Message::FilterPersistenceFailed("first line\n東京".into()),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(window.find("log-search-input").focused(), Some(true));
        assert!(window.find("notification-popup").bounds().size.width <= px(448.));
        window.click("notification-dismiss", cx);
        assert!(app.read(cx).state.status.is_none());
        app.update(cx, |app, cx| {
            app.dispatch(
                Message::FilterPersistenceFailed("repeated".into()),
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.background_executor.advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(app.read(cx).state.status.is_some());
        app.update(cx, |app, cx| {
            app.dispatch(
                Message::FilterPersistenceFailed("repeated".into()),
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, _, cx| {
        assert!(app.read(cx).state.status.is_some());
    })
    .unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, _, cx| {
        assert!(app.read(cx).state.status.is_none());
    })
    .unwrap();
}
