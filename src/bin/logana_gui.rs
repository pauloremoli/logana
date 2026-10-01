use gpui_kit::component::GlobalState;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{KeyBinding, Menu, MenuItem, WindowOptions};
use logana::db::{Database, default_db_path};
use logana::gui::app::{App, OpenFile, Quit};
use logana::gui::runtime;
use logana::gui::theme as gui_theme;
use logana::theme::Theme as TuiTheme;
use std::sync::Arc;

fn main() {
    let (db, theme) = runtime::handle().block_on(open_database_and_theme());

    gpui_kit::application().run(move |cx| {
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
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| App::new(Arc::clone(&db), theme.clone(), cx))
        })
        .expect("failed to open window");
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
