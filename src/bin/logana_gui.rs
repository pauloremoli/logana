use clap::Parser;
use gpui_kit::component::{GlobalState, TitleBar};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{KeyBinding, Menu, MenuItem};
use logana::cli::{Args, expand_cli_files, run_headless_mode, validate_startup_args};
use logana::db::{Database, SessionStore, default_db_path};
use logana::gui::app::{App, OpenCommands, OpenFile, Quit};
use logana::gui::persistence::Preferences;
use logana::gui::runtime;
use logana::gui::theme as gui_theme;
use logana::theme::Theme as TuiTheme;
use std::sync::Arc;

fn main() {
    let mut args = Args::parse();
    args.files = expand_cli_files(args.files).unwrap_or_else(|error| fail(error));
    validate_startup_args(&args).unwrap_or_else(|error| fail(error));
    if args.headless {
        runtime::handle()
            .block_on(run_headless_mode(args))
            .unwrap_or_else(|error| fail(error.to_string()));
        return;
    }
    let mut schema_warnings = logana::config::init_schemas();
    schema_warnings.extend(logana::parser::validate_custom_schemas(
        logana::config::custom_schemas(),
    ));
    for warning in schema_warnings {
        eprintln!("Warning: {warning}");
    }
    let (db, theme) = runtime::handle().block_on(open_database_and_theme());
    let (preferences, files) = runtime::handle().block_on(async {
        let config = logana::config::Config::load().unwrap_or_default();
        let preferences = Preferences::load(&db, &config)
            .await
            .unwrap_or_else(|error| {
                eprintln!("Cannot load GUI settings: {error}");
                Preferences::default()
            });
        let files = if args.files.is_empty() {
            db.load_session().await.unwrap_or_else(|error| {
                eprintln!("Cannot load previous session: {error}");
                Vec::new()
            })
        } else {
            Vec::new()
        };
        (preferences, files)
    });

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
            cx.bind_keys([KeyBinding::new(
                "ctrl-shift-p",
                OpenCommands,
                Some("LoganaLogs"),
            )]);
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
                app.update(cx, |app, cx| {
                    app.start_cli(args.clone(), preferences.clone(), files.clone(), window, cx);
                });
                let closing_app = app.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    closing_app.update(cx, |app, cx| app.request_quit(cx)).ok();
                    false
                });
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

fn fail(error: String) -> ! {
    eprintln!("{error}");
    std::process::exit(1);
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
