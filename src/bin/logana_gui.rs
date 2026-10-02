use gpui_kit::component::{GlobalState, TitleBar};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{KeyBinding, Menu, MenuItem};
use logana::db::{Database, default_db_path};
use logana::gui::app::{App, OpenFile, Quit};
use logana::gui::runtime;
use logana::gui::theme as gui_theme;
use logana::theme::Theme as TuiTheme;
use std::sync::Arc;

fn main() {
    let (db, theme) = runtime::handle().block_on(open_database_and_theme());

    // Without this, every `Icon::new(IconName::..)` silently renders
    // nothing (just the surrounding Button/div chrome) — the SVG asset
    // never resolves because no AssetSource is registered at all. Matches
    // gpui-component's own reference app (`crates/story/src/main.rs`:
    // `gpui_platform::application().with_assets(Assets)`); `AllAssets`
    // specifically, not the smaller `Assets`, since this app's icons
    // (Bookmark, MessageSquare, Funnel, Group, ...) come from the full
    // Lucide catalog, not gpui-component's own curated subset.
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gui_theme::install(&theme, cx);
            cx.bind_keys([
                KeyBinding::new("q", Quit, None),
                KeyBinding::new("ctrl-o", OpenFile, None),
            ]);
            GlobalState::global_mut(cx).set_app_menus(vec![
                Menu::new("File")
                    .items([
                        MenuItem::action("Open...", OpenFile),
                        MenuItem::separator(),
                        MenuItem::action("Quit", Quit),
                    ])
                    .owned(),
            ]);
            // `TitleBar::window_options()` requests client-side decorations
            // (a custom titlebar the app draws and drags itself) so
            // `component::TitleBar` below can render real minimize/
            // maximize/close controls instead of relying on the window
            // manager's — the window falls back to server-side decorations
            // on its own when the platform can't honor that, in which case
            // `TitleBar` detects it and skips drawing duplicate controls.
            gpui_kit::open_window(TitleBar::window_options(), cx, |window, cx| {
                let app = cx.new(|cx| App::new(Arc::clone(&db), theme.clone(), cx));
                // Give the root element initial keyboard focus so key capture
                // starts working immediately, without requiring a click into
                // the window first.
                let root_focus = app.read(cx).root_focus.clone();
                window.defer(cx, move |window, cx| {
                    if window.focused(cx).is_none() {
                        root_focus.focus(window, cx);
                    }
                });
                app
            })
            .expect("failed to open window");
            cx.activate(true);
        });
}

async fn open_database_and_theme() -> (Arc<Database>, TuiTheme) {
    let db = open_database().await;
    let theme = gui_theme::load_current(&db).await;
    (db, theme)
}

async fn open_database() -> Arc<Database> {
    let path = default_db_path();
    let db = match Database::new(&path).await {
        Ok(db) => db,
        Err(err) => {
            eprintln!(
                "Warning: could not open database at '{}': {}. Running without persistence.",
                path, err
            );
            Database::in_memory()
                .await
                .expect("failed to open in-memory database")
        }
    };
    Arc::new(db)
}
