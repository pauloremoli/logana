//! Native GUI persistence using the same database keys and contexts as the TUI.
use crate::config::{Config, RestoreSessionPolicy};
use crate::db::{
    AppSettingsStore, Database, FileContext, FileContextStore, SessionStore, SettingsKey,
};
#[cfg(test)]
use crate::gui::message::Message;
use crate::gui::{app::App, message::FileLoaded, state::StatusMessage};
use crate::shared::{SidebarSide, TabState};
use gpui_kit::gpui::{AsyncApp, Context, PromptLevel, WeakEntity, Window};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

pub(super) const DISPLAY_KEYS: [SettingsKey; 9] = [
    SettingsKey::Wrap,
    SettingsKey::ShowLineNumbers,
    SettingsKey::RelativeLineNumbers,
    SettingsKey::ShowSidebar,
    SettingsKey::SidebarLeft,
    SettingsKey::CollapseContinuations,
    SettingsKey::ShowGroupsPanel,
    SettingsKey::ShowModeBar,
    SettingsKey::ShowBorders,
];

#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub(super) display: [bool; 9],
    sidebar_width: u16,
    pub restore_session: RestoreSessionPolicy,
    pub restore_file: RestoreSessionPolicy,
    pub preview_bytes: u64,
    pub mcp_port: u16,
    pub font_size: f32,
    pub column_widths: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f32>>,
    pub locked: Vec<SettingsKey>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            display: [true, true, false, true, false, false, true, true, true],
            sidebar_width: 30,
            restore_session: RestoreSessionPolicy::Always,
            restore_file: RestoreSessionPolicy::Always,
            preview_bytes: crate::config::DEFAULT_PREVIEW_BYTES,
            mcp_port: 9876,
            font_size: 13.,
            column_widths: Default::default(),
            locked: Vec::new(),
        }
    }
}
impl Preferences {
    pub async fn load(db: &Database, config: &Config) -> anyhow::Result<Self> {
        let mut prefs = Self::default();
        let overrides = [
            config.wrap,
            config.show_line_numbers,
            config.relative_line_numbers,
            config.show_sidebar,
            config.sidebar_side.map(SidebarSide::is_left),
            None,
            None,
            config.show_mode_bar,
            config.show_borders,
        ];
        for (i, key) in DISPLAY_KEYS.into_iter().enumerate() {
            prefs.display[i] = if let Some(value) = overrides[i] {
                prefs.locked.push(key);
                value
            } else {
                db.load_app_setting(key)
                    .await?
                    .map(|v| v == "true")
                    .unwrap_or(prefs.display[i])
            };
        }
        prefs.sidebar_width = db
            .load_app_setting(SettingsKey::SidebarWidth)
            .await?
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(30)
            .clamp(10, 200);
        for (target, key, value) in [
            (
                &mut prefs.restore_session,
                SettingsKey::RestoreSession,
                config.restore_session,
            ),
            (
                &mut prefs.restore_file,
                SettingsKey::RestoreFileContext,
                config.restore_file_context,
            ),
        ] {
            *target = match value {
                Some(value) => {
                    prefs.locked.push(key);
                    value
                }
                None => db
                    .load_app_setting(key)
                    .await?
                    .map(|v| v.parse().unwrap_or(RestoreSessionPolicy::Ask))
                    .unwrap_or(RestoreSessionPolicy::Always),
            };
        }
        for (key, value, default) in [
            (
                SettingsKey::PreviewBytes,
                config.preview_bytes,
                prefs.preview_bytes,
            ),
            (
                SettingsKey::McpPort,
                config.mcp_port.map(u64::from),
                u64::from(prefs.mcp_port),
            ),
        ] {
            let value = if let Some(value) = value {
                prefs.locked.push(key);
                value
            } else {
                db.load_app_setting(key)
                    .await?
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(default)
            };
            match key {
                SettingsKey::PreviewBytes => {
                    prefs.preview_bytes = value.clamp(1, 128 * 1024 * 1024)
                }
                SettingsKey::McpPort => prefs.mcp_port = value.clamp(1, 65535) as u16,
                _ => unreachable!(),
            }
        }
        prefs.font_size = db
            .load_app_setting(SettingsKey::GuiFontSize)
            .await?
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(prefs.font_size)
            .clamp(10., 32.);
        prefs.column_widths = db
            .load_app_setting(SettingsKey::GuiColumnWidths)
            .await?
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        Ok(prefs)
    }
    pub(super) fn settings(&self) -> Vec<(SettingsKey, String)> {
        let mut settings: Vec<_> = DISPLAY_KEYS
            .into_iter()
            .zip(self.display)
            .map(|(k, v)| (k, v.to_string()))
            .collect();
        settings.extend([
            (
                SettingsKey::RestoreSession,
                self.restore_session.to_string(),
            ),
            (
                SettingsKey::RestoreFileContext,
                self.restore_file.to_string(),
            ),
            (SettingsKey::PreviewBytes, self.preview_bytes.to_string()),
            (SettingsKey::McpPort, self.mcp_port.to_string()),
            (SettingsKey::GuiFontSize, self.font_size.to_string()),
            (
                SettingsKey::GuiColumnWidths,
                serde_json::to_string(&self.column_widths).expect("finite column widths"),
            ),
        ]);
        settings.retain(|(key, _)| !self.locked.contains(key));
        settings
    }
    pub(super) fn apply_font_size(&self, window: &mut Window, cx: &mut gpui_kit::gpui::App) {
        let size = gpui_kit::gpui::px(self.font_size);
        gpui_kit::component::theme::Theme::update(cx, |theme| theme.font_size = size);
        window.set_rem_size(size);
    }
    pub(super) fn apply(&self, tab: &mut TabState) {
        let d = &mut tab.display;
        d.wrap = self.display[0];
        d.show_line_numbers = self.display[1];
        d.relative_line_numbers = self.display[2];
        d.show_sidebar = self.display[3];
        d.sidebar_side = if self.display[4] {
            SidebarSide::Left
        } else {
            SidebarSide::Right
        };
        d.collapse_continuations = self.display[5];
        d.show_groups_panel = self.display[6];
        d.show_mode_bar = self.display[7];
        d.show_borders = self.display[8];
        d.sidebar_width = self.sidebar_width;
    }
    fn display_of(tab: &TabState) -> ([bool; 9], u16) {
        let d = &tab.display;
        (
            [
                d.wrap,
                d.show_line_numbers,
                d.relative_line_numbers,
                d.show_sidebar,
                d.sidebar_side.is_left(),
                d.collapse_continuations,
                d.show_groups_panel,
                d.show_mode_bar,
                d.show_borders,
            ],
            d.sidebar_width,
        )
    }
}

#[derive(Default)]
struct Save {
    contexts: Vec<FileContext>,
    session: Option<Vec<String>>,
    settings: Vec<(SettingsKey, String)>,
}
impl Save {
    fn merge(&mut self, next: Self) {
        for context in next.contexts {
            self.contexts
                .retain(|c| c.source_file != context.source_file);
            self.contexts.push(context);
        }
        for (key, value) in next.settings {
            self.settings.retain(|(k, _)| *k != key);
            self.settings.push((key, value));
        }
        if next.session.is_some() {
            self.session = next.session;
        }
    }
    async fn persist(&self, db: &Database) -> anyhow::Result<()> {
        for context in &self.contexts {
            db.save_file_context(context).await?;
        }
        for (key, value) in &self.settings {
            db.save_app_setting(*key, value).await?;
        }
        if let Some(files) = &self.session {
            db.save_session(files).await?;
        }
        Ok(())
    }
}
struct Request {
    save: Save,
    flushed: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
}
pub(super) struct Persistence {
    sender: tokio::sync::mpsc::UnboundedSender<Request>,
    errors: tokio::sync::mpsc::UnboundedReceiver<String>,
    contexts: HashMap<String, FileContext>,
    session: Option<Vec<String>>,
    pub restoring: bool,
    next_save: std::time::Instant,
}
impl Persistence {
    pub fn new(db: Arc<Database>) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let (error_sender, errors) = tokio::sync::mpsc::unbounded_channel();
        crate::gui::runtime::handle().spawn(async move {
            let mut save = Save::default();
            while let Some(request) = receiver.recv().await {
                save.merge(request.save);
                let mut flushed: Vec<_> = request.flushed.into_iter().collect();
                while let Ok(next) = receiver.try_recv() {
                    save.merge(next.save);
                    flushed.extend(next.flushed);
                }
                let result = save.persist(&db).await.map_err(|error| error.to_string());
                match &result {
                    Err(error) => {
                        let _ = error_sender.send(error.clone());
                    }
                    Ok(()) => save = Save::default(),
                }
                for flushed in flushed {
                    let _ = flushed.send(result.clone());
                }
            }
        });
        Self {
            sender,
            errors,
            contexts: HashMap::new(),
            session: None,
            restoring: false,
            next_save: std::time::Instant::now(),
        }
    }
}

fn session_files(tabs: &[TabState]) -> Vec<String> {
    tabs.iter()
        .filter(|tab| {
            !tab.is_temp_backed()
                && tab.merged.is_none()
                && !tab
                    .stream
                    .watch
                    .as_ref()
                    .is_some_and(|w| w.temp_file.is_some())
        })
        .filter_map(|tab| tab.log_manager.source_file())
        .filter(|source| {
            !source.contains("://")
                && !source.starts_with("run:")
                && crate::ingestion::detect_archive_type(source).is_none()
        })
        .map(str::to_owned)
        .collect()
}

impl App {
    pub(super) fn save_preferences(&mut self, prefs: Preferences) {
        let save = Save {
            settings: prefs.settings(),
            ..Default::default()
        };
        self.state.preferences = prefs;
        let _ = self.persistence.sender.send(Request {
            save,
            flushed: None,
        });
    }
    pub(super) fn save_session_state(&mut self) {
        if self.persistence.restoring {
            return;
        }
        let mut save = Save::default();
        for tab in &self.state.tabs {
            if let Some(mut context) = tab.to_file_context() {
                if let Some(anchor) = tab.filter.handle.as_ref().and_then(|h| h.scroll_anchor) {
                    context.scroll_offset = anchor;
                }
                if tab.filter.handle.is_none()
                    && let Some((id, _, row)) = self.state.log_selection
                    && id == tab.id
                    && let Some(line) = tab.filter.visible_indices.get_opt(row)
                {
                    context.scroll_offset = line;
                }
                context.search_query = tab.search.query.get_pattern().unwrap_or_default().into();
                if self.persistence.contexts.get(&context.source_file) != Some(&context) {
                    self.persistence
                        .contexts
                        .insert(context.source_file.clone(), context.clone());
                    save.contexts.push(context);
                }
            }
        }
        let files = session_files(&self.state.tabs);
        // Opening and immediately quitting an empty window must not erase the last session.
        if (!self.state.tabs.is_empty() || self.persistence.session.is_some())
            && self.persistence.session.as_ref() != Some(&files)
        {
            self.persistence.session = Some(files.clone());
            save.session = Some(files);
        }
        if !save.contexts.is_empty() || save.session.is_some() {
            let _ = self.persistence.sender.send(Request {
                save,
                flushed: None,
            });
        }
    }
    pub(super) fn poll_persistence(&mut self, cx: &mut Context<Self>) {
        while let Ok(error) = self.persistence.errors.try_recv() {
            self.persistence.contexts.clear();
            self.persistence.session = None;
            self.state.notify(StatusMessage::Error(format!(
                "Cannot save session: {error}"
            )));
            cx.notify();
        }
        if std::time::Instant::now() >= self.persistence.next_save {
            self.persistence.next_save =
                std::time::Instant::now() + std::time::Duration::from_secs(1);
            self.save_session_state();
        }
    }
    pub(super) fn display_snapshot(&self) -> Option<(crate::shared::TabId, [bool; 9], u16)> {
        self.state.active_tab().map(|tab| {
            let (display, width) = Preferences::display_of(tab);
            (tab.id, display, width)
        })
    }
    pub(super) fn save_display_change(
        &mut self,
        before: Option<(crate::shared::TabId, [bool; 9], u16)>,
    ) {
        let (Some((old_id, old, width)), Some((id, new, new_width))) =
            (before, self.display_snapshot())
        else {
            return;
        };
        if old_id != id {
            return;
        }
        let mut save = Save::default();
        for (i, key) in DISPLAY_KEYS.into_iter().enumerate() {
            if old[i] != new[i] {
                self.state.preferences.display[i] = new[i];
                save.settings.push((key, new[i].to_string()));
            }
        }
        if width != new_width {
            self.state.preferences.sidebar_width = new_width;
            save.settings
                .push((SettingsKey::SidebarWidth, new_width.to_string()));
        }
        if !save.settings.is_empty() {
            let _ = self.persistence.sender.send(Request {
                save,
                flushed: None,
            });
        }
    }
    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        let flushed = self.flush_persistence();
        cx.spawn(async move |this, cx| match flushed.await {
            Ok(()) => cx.update(|cx| cx.quit()),
            Err(error) => {
                this.update(cx, |app, cx| {
                    app.state.notify(StatusMessage::Error(format!(
                        "Cannot save before quitting: {error}"
                    )));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
    pub(super) fn flush_persistence(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), String>> + use<> {
        self.save_session_state();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let _ = self.persistence.sender.send(Request {
            save: Save::default(),
            flushed: Some(sender),
        });
        async move {
            receiver
                .await
                .map_err(|_| "Session writer stopped".to_string())?
        }
    }
    /// Called by the native launcher after preferences have been resolved.
    pub fn restore_session(
        &mut self,
        preferences: Preferences,
        files: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_session_with_args(preferences, files, None, window, cx);
    }
    pub(super) fn restore_session_with_args(
        &mut self,
        preferences: Preferences,
        files: Vec<String>,
        args: Option<crate::cli::Args>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        preferences.apply_font_size(window, cx);
        self.state.preferences = preferences;
        cx.notify();
        if files.is_empty() || self.state.preferences.restore_session == RestoreSessionPolicy::Never
        {
            return;
        }
        self.persistence.restoring = true;
        let prompt =
            (self.state.preferences.restore_session == RestoreSessionPolicy::Ask).then(|| {
                window.prompt(
                    PromptLevel::Info,
                    "Restore the previous log session?",
                    Some(&format!("Reopen {} files.", files.len())),
                    &["Restore", "Skip"],
                    cx,
                )
            });
        cx.spawn(async move |this, cx| {
            let restore = match prompt {
                Some(prompt) => prompt.await.ok() == Some(0),
                None => true,
            };
            if restore {
                this.update(cx, |app, cx| {
                    for file in files {
                        app.queue_file_with_args(PathBuf::from(file), args.clone(), cx);
                    }
                })
                .ok();
            }
            this.update(cx, |app, _| {
                if app.loading.operations.is_empty() {
                    app.persistence.restoring = false;
                    app.save_session_state();
                }
            })
            .ok();
        })
        .detach();
    }
}

pub(super) async fn prepare_loaded(
    this: &WeakEntity<App>,
    cx: &mut AsyncApp,
    mut result: Result<FileLoaded, String>,
    control: &crate::ingestion::archive_job::Control,
) -> Result<FileLoaded, String> {
    control.check().map_err(|e| e.to_string())?;
    let policy = this
        .update(cx, |app, _| app.state.preferences.restore_file)
        .ok();
    if let Ok(loaded) = &mut result {
        match policy {
            Some(RestoreSessionPolicy::Always) => {}
            Some(RestoreSessionPolicy::Ask) if loaded.context.is_some() => {
                let prompt = this.update_in(cx, |_, window, cx| {
                    window.prompt(
                        PromptLevel::Info,
                        "Restore saved file context?",
                        Some(&loaded.path.display().to_string()),
                        &["Restore", "Skip"],
                        cx,
                    )
                });
                let accepted = match prompt {
                    Ok(prompt) => {
                        let control = control.clone();
                        let mut canceled = crate::gui::runtime::handle().spawn(async move {
                            loop {
                                if control.cancelled() {
                                    return;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        });
                        let accepted = tokio::select! {
                            answer = prompt => answer.ok() == Some(0),
                            _ = &mut canceled => false,
                        };
                        canceled.abort();
                        accepted
                    }
                    Err(_) => false,
                };
                if !accepted {
                    loaded.context = None;
                }
            }
            _ => loaded.context = None,
        }
    }
    control.check().map_err(|e| e.to_string())?;
    result
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn saved_font_sizes_validate_on_load() {
        let db = Database::in_memory().await.unwrap();
        for (stored, expected) in [
            ("NaN", 13.),
            ("inf", 13.),
            ("bad", 13.),
            ("5", 10.),
            ("100", 32.),
            ("17.5", 17.5),
        ] {
            db.save_app_setting(SettingsKey::GuiFontSize, stored)
                .await
                .unwrap();
            let loaded = Preferences::load(&db, &Config::default()).await.unwrap();
            assert_eq!(loaded.font_size, expected);
        }
    }

    #[tokio::test]
    async fn general_preferences_validate_persist_and_respect_config_pins() {
        use crate::gui::{
            actions::{self, Action},
            effect::Effect,
            message::Message,
            update::update,
        };
        let db = Arc::new(Database::in_memory().await.unwrap());
        let config = Config {
            wrap: Some(true),
            restore_session: Some(RestoreSessionPolicy::Never),
            ..Default::default()
        };
        let mut state = crate::gui::state::GuiState::new(db.clone());
        state.preferences = Preferences::load(&db, &config).await.unwrap();
        update(
            &mut state,
            Message::NavPageSelected(crate::gui::state::NavPage::Settings),
        );
        assert_eq!(state.editor.as_ref().unwrap().action, Action::Settings);
        update(&mut state, Message::EditorText(2, "0".into()));
        assert!(matches!(actions::apply(&mut state), Effect::None));
        assert!(state.editor.as_ref().unwrap().error.is_some());
        update(&mut state, Message::EditorText(2, "1048576".into()));
        update(&mut state, Message::EditorText(1, "ask".into()));
        update(&mut state, Message::EditorText(0, "always".into()));
        update(&mut state, Message::EditorField(0));
        let Effect::SavePreferences(prefs) = actions::apply(&mut state) else {
            panic!("settings should save");
        };
        assert_eq!(prefs.restore_session, RestoreSessionPolicy::Never);
        assert!(prefs.display[0]);
        Save {
            settings: prefs.settings(),
            ..Default::default()
        }
        .persist(&db)
        .await
        .unwrap();
        let loaded = Preferences::load(&db, &config).await.unwrap();
        assert_eq!(loaded.restore_file, RestoreSessionPolicy::Ask);
        assert_eq!(loaded.preview_bytes, 1048576);
        assert_eq!(loaded.restore_session, RestoreSessionPolicy::Never);
    }
    use super::*;
    use crate::db::LogManager;
    use crate::gui::{state::GuiState, update};
    use crate::ingestion::FileReader;

    #[tokio::test]
    async fn settings_use_shared_keys_and_config_overrides_take_priority() {
        let db = Database::in_memory().await.unwrap();
        db.save_app_setting(SettingsKey::Wrap, "false")
            .await
            .unwrap();
        db.save_app_setting(SettingsKey::SidebarLeft, "true")
            .await
            .unwrap();
        db.save_app_setting(SettingsKey::RestoreSession, "never")
            .await
            .unwrap();
        db.save_app_setting(SettingsKey::RestoreFileContext, "ask")
            .await
            .unwrap();
        db.save_app_setting(SettingsKey::SidebarWidth, "999")
            .await
            .unwrap();
        let prefs = Preferences::load(&db, &Config::default()).await.unwrap();
        assert!(!prefs.display[0]);
        assert!(prefs.display[4]);
        assert_eq!(prefs.sidebar_width, 200);
        assert_eq!(prefs.restore_session, RestoreSessionPolicy::Never);
        assert_eq!(prefs.restore_file, RestoreSessionPolicy::Ask);
        let config = Config {
            wrap: Some(true),
            restore_session: Some(RestoreSessionPolicy::Always),
            restore_file_context: Some(RestoreSessionPolicy::Never),
            ..Default::default()
        };
        let pinned = Preferences::load(&db, &config).await.unwrap();
        assert!(pinned.display[0]);
        assert_eq!(pinned.restore_session, RestoreSessionPolicy::Always);
        assert_eq!(pinned.restore_file, RestoreSessionPolicy::Never);
    }

    #[tokio::test]
    async fn file_context_round_trip_restores_source_cursor_and_layout_after_filtering() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"drop\nkeep first\ndrop\nkeep last\n").unwrap();
        let path = std::fs::canonicalize(file.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut manager = LogManager::new(db.clone(), Some(path.clone())).await;
        manager = update::execute_command(
            manager,
            update::parse_command("filter keep").unwrap().unwrap(),
            (0, 0, 0),
            None,
        )
        .await
        .unwrap();
        let mut original = TabState::new(FileReader::new(&path).unwrap(), manager, "saved".into());
        original.refresh_visible();
        original.scroll.scroll_offset = 1;
        original.scroll.horizontal_scroll = 17;
        original.display.show_keys = true;
        original.display.hidden_fields.insert("secret".into());
        original.display.field_layout.columns = Some(vec!["message".into(), "secret".into()]);
        original.display.level_colors_disabled.insert("info".into());
        original.mark_manager.mark(3);
        original
            .comment_manager
            .add("Unicode note α".into(), vec![3]);
        let context = original.to_file_context().unwrap();
        assert_eq!(context.scroll_offset, 3);
        Save {
            contexts: vec![context],
            session: Some(vec![path.clone()]),
            ..Default::default()
        }
        .persist(&db)
        .await
        .unwrap();
        let loaded = update::load_file(file.path().into(), db.clone())
            .await
            .unwrap();
        assert_eq!(loaded.log_manager.get_filters().len(), 1);
        let mut state = GuiState::new(db.clone());
        update::update(&mut state, Message::FileLoaded(Ok(loaded)));
        let tab = &mut state.tabs[0];
        for _ in 0..100 {
            tab.advance_filter_computation();
            if tab.filter.handle.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        assert!(tab.filter.handle.is_none());
        assert_eq!(
            tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset),
            Some(3)
        );
        assert_eq!(tab.scroll.horizontal_scroll, 17);
        assert_eq!(tab.comment_manager.get()[0].text, "Unicode note α");
        assert_eq!(tab.mark_manager.get_indices(), vec![3]);
        assert!(tab.display.show_keys);
        assert!(tab.display.hidden_fields.contains("secret"));
        assert_eq!(
            tab.display.field_layout.columns.as_ref().unwrap()[0],
            "message"
        );
        assert!(tab.display.level_colors_disabled.contains("info"));
        assert_eq!(db.load_session().await.unwrap(), vec![path]);
    }

    #[tokio::test]
    async fn coalesced_saves_keep_latest_context_and_clear_closed_session() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let manager = LogManager::new(db.clone(), Some("first.log".into())).await;
        let tab = TabState::new(
            FileReader::from_bytes(b"a\nb\n".to_vec()),
            manager,
            "first".into(),
        );
        let mut initial = tab.to_file_context().unwrap();
        let mut last = initial.clone();
        last.scroll_offset = 1;
        initial.marked_lines = vec![0];
        let mut save = Save {
            contexts: vec![initial],
            session: Some(vec!["first.log".into()]),
            settings: vec![(SettingsKey::Wrap, "false".into())],
        };
        save.merge(Save {
            contexts: vec![last.clone()],
            session: Some(vec![]),
            settings: vec![(SettingsKey::Wrap, "true".into())],
        });
        save.persist(&db).await.unwrap();
        assert_eq!(db.load_file_context("first.log").await.unwrap(), Some(last));
        assert!(db.load_session().await.unwrap().is_empty());
        assert_eq!(
            db.load_app_setting(SettingsKey::Wrap)
                .await
                .unwrap()
                .as_deref(),
            Some("true")
        );
    }

    #[tokio::test]
    async fn sessions_preserve_tab_order_and_exclude_temporary_stream_sources() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut tabs = Vec::new();
        for path in ["second.log", "run:echo", "first.log", "otlp://4318"] {
            let manager = LogManager::new(db.clone(), Some(path.into())).await;
            tabs.push(TabState::new(
                FileReader::from_bytes(vec![]),
                manager,
                path.into(),
            ));
        }
        tabs[2].merged_temp = Some(tempfile::NamedTempFile::new().unwrap());
        assert_eq!(session_files(&tabs), vec!["second.log"]);
        tabs[2].merged_temp = None;
        assert_eq!(session_files(&tabs), vec!["second.log", "first.log"]);
    }
}

#[cfg(all(test, feature = "gui-test"))]
mod native_tests {
    use super::*;
    use gpui_kit::gpui::{AppContext, Entity, TestAppContext, px};
    use gpui_kit::test::TestWindowExt;

    fn app(cx: &mut TestAppContext) -> Entity<App> {
        let db = Arc::new(
            crate::gui::runtime::handle()
                .block_on(Database::in_memory())
                .unwrap(),
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            let theme = crate::theme::Theme::default();
            crate::gui::theme::install(&theme, cx);
            let (_, app) = gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| App::new(db, theme, cx))
            })
            .unwrap();
            app
        })
    }
    fn loaded(app: &Entity<App>, cx: &mut TestAppContext) -> FileLoaded {
        let db = app.read_with(cx, |app, _| app.state.db.clone());
        let manager = crate::gui::runtime::handle().block_on(crate::db::LogManager::new(
            db,
            Some("/logs/sample.log".into()),
        ));
        let mut tab = TabState::new(
            crate::ingestion::FileReader::from_bytes(b"first\nsecond\n".to_vec()),
            manager,
            "sample".into(),
        );
        tab.mark_manager.mark(1);
        tab.comment_manager.add("saved note".into(), vec![1]);
        tab.scroll.scroll_offset = 1;
        let context = tab.to_file_context();
        let (_, receiver) = tokio::sync::watch::channel(());
        FileLoaded {
            path: PathBuf::from("/logs/sample.log"),
            context,
            reader: tab.file_reader,
            log_manager: tab.log_manager,
            watch: crate::shared::watch_state_from_file(receiver, "/logs/sample.log".into()),
        }
    }

    fn cache_facets(app: &mut App) {
        if let Some(tab) = app.state.active_tab() {
            app.facet_counts.insert(
                tab.id,
                crate::gui::app::FacetCache {
                    key: (
                        tab.file_reader.line_count(),
                        tab.display
                            .format
                            .as_ref()
                            .map(|p| p.name().to_owned())
                            .unwrap_or_default(),
                    ),
                    counts: Some(Default::default()),
                },
            );
        }
    }
    #[gpui_kit::test]
    fn file_restore_policy_controls_native_prompt_and_context(cx: &mut TestAppContext) {
        for (policy, answer, restored) in [
            (RestoreSessionPolicy::Always, None, true),
            (RestoreSessionPolicy::Never, None, false),
            (RestoreSessionPolicy::Ask, Some("Restore"), true),
            (RestoreSessionPolicy::Ask, Some("Skip"), false),
        ] {
            let app = app(cx);
            let loaded = loaded(&app, cx);
            app.update(cx, |app, _| app.state.preferences.restore_file = policy);
            let this = app.downgrade();
            cx.spawn(async move |mut cx| {
                let result = prepare_loaded(
                    &this,
                    &mut cx,
                    Ok(loaded),
                    &crate::ingestion::archive_job::Control::new().0,
                )
                .await;
                this.update(&mut cx, |app, _| {
                    let handle = crate::gui::runtime::handle();
                    let _guard = handle.enter();
                    crate::gui::update::update(&mut app.state, Message::FileLoaded(result));
                    cache_facets(app);
                })
                .ok();
            })
            .detach();
            cx.run_until_parked();
            assert_eq!(cx.has_pending_prompt(), answer.is_some());
            if let Some(answer) = answer {
                cx.simulate_prompt_answer(answer);
                cx.run_until_parked();
            }
            app.read_with(cx, |app, _| {
                assert_eq!(app.state.tabs.len(), 1);
                let tab = &app.state.tabs[0];
                assert_eq!(tab.mark_manager.is_marked(1), restored);
                assert_eq!(tab.comment_manager.get().len(), usize::from(restored));
                assert_eq!(tab.scroll.scroll_offset, usize::from(restored));
            });
        }
    }

    #[gpui_kit::test]
    fn display_edits_and_last_tab_close_are_flushed_to_shared_storage(cx: &mut TestAppContext) {
        let app = app(cx);
        let loaded = loaded(&app, cx);
        let db = app.read_with(cx, |app, _| app.state.db.clone());
        let window = cx.windows().last().copied().unwrap();
        let flushed = cx
            .update_window(window, |_, window, cx| {
                app.update(cx, |app, cx| {
                    let handle = crate::gui::runtime::handle();
                    let _guard = handle.enter();
                    crate::gui::update::update(&mut app.state, Message::FileLoaded(Ok(loaded)));
                    cache_facets(app);
                    app.dispatch(
                        Message::GuiCommand(crate::commands::Commands::Wrap, "Wrap".into()),
                        window,
                        cx,
                    );
                    app.state.tabs[0]
                        .comment_manager
                        .add("latest note".into(), vec![0]);
                    app.state.log_selection = Some((app.state.tabs[0].id, 0, 0));
                    app.dispatch(Message::TabClosed(0), window, cx);
                    app.flush_persistence()
                })
            })
            .unwrap();
        crate::gui::runtime::handle().block_on(flushed).unwrap();
        assert_eq!(
            crate::gui::runtime::handle()
                .block_on(db.load_app_setting(SettingsKey::Wrap))
                .unwrap()
                .as_deref(),
            Some("false")
        );
        let context = crate::gui::runtime::handle()
            .block_on(db.load_file_context("/logs/sample.log"))
            .unwrap()
            .unwrap();
        assert_eq!(context.scroll_offset, 0);
        assert_eq!(context.comments.len(), 2);
        assert_eq!(context.comments[1].text, "latest note");
        assert_eq!(context.marked_lines, vec![1]);
        assert!(
            crate::gui::runtime::handle()
                .block_on(db.load_session())
                .unwrap()
                .is_empty()
        );
    }

    #[gpui_kit::test]
    fn filtered_context_restores_native_viewport_after_background_scan(cx: &mut TestAppContext) {
        let app = app(cx);
        let mut loaded = loaded(&app, cx);
        let bytes = (0..10_000)
            .map(|i| if i % 10 == 0 { "keep\n" } else { "drop\n" })
            .collect::<String>();
        loaded.reader = crate::ingestion::FileReader::from_bytes(bytes.into_bytes());
        loaded.log_manager = crate::gui::runtime::handle()
            .block_on(crate::gui::update::execute_command(
                loaded.log_manager,
                crate::gui::update::parse_command("filter keep")
                    .unwrap()
                    .unwrap(),
                (0, 0, 0),
                None,
            ))
            .unwrap();
        let context = loaded.context.as_mut().unwrap();
        context.scroll_offset = 5000;
        context.marked_lines.clear();
        context.comments.clear();
        let window = cx.windows().last().copied().unwrap();
        cx.update_window(window, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.dispatch(Message::FileLoaded(Ok(loaded)), window, cx);
                cache_facets(app);
            });
            window.render_frame(cx);
        })
        .unwrap();
        let before = app.read_with(cx, |app, _| {
            app.log_scroll.scroll_px_offset_for_scrollbar().y
        });
        for _ in 0..100 {
            let done = app.update(cx, |app, _| {
                app.state.tabs[0].advance_filter_computation();
                app.state.tabs[0].filter.handle.is_none()
            });
            if done {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        cx.update_window(window, |_, window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.state.tabs[0].filter.handle.is_none());
                assert_eq!(app.state.tabs[0].scroll.scroll_offset, 500);
                assert!(app.finish_context_scroll());
                cx.notify();
            });
            window.render_frame(cx);
        })
        .unwrap();
        app.read_with(cx, |app, _| {
            let after = app.log_scroll.scroll_px_offset_for_scrollbar().y;
            assert_ne!(after, before);
            let cursor_top = px(500. * 30.) + after;
            let viewport_height = app.log_scroll.viewport_bounds().size.height;
            assert!(cursor_top >= px(0.));
            assert!(cursor_top + px(30.) <= viewport_height + px(1.));
        });
    }

    #[gpui_kit::test]
    fn never_session_policy_skips_reopening_and_prompting(cx: &mut TestAppContext) {
        let app = app(cx);
        let window = cx.windows().last().copied().unwrap();
        cx.update_window(window, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.restore_session(
                    Preferences {
                        restore_session: RestoreSessionPolicy::Never,
                        ..Default::default()
                    },
                    vec!["/logs/previous.log".into()],
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
        app.read_with(cx, |app, _| assert!(app.state.tabs.is_empty()));
    }

    #[gpui_kit::test]
    fn declining_session_restore_leaves_existing_session_untouched(cx: &mut TestAppContext) {
        let app = app(cx);
        let files = vec!["/logs/first.log".into(), "/logs/second.log".into()];
        let db = app.read_with(cx, |app, _| app.state.db.clone());
        crate::gui::runtime::handle()
            .block_on(db.save_session(&files))
            .unwrap();
        let window = cx.windows().last().copied().unwrap();
        cx.update_window(window, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.restore_session(
                    Preferences {
                        restore_session: RestoreSessionPolicy::Ask,
                        ..Default::default()
                    },
                    files.clone(),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Skip");
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.state.tabs.is_empty());
            assert!(!app.persistence.restoring);
        });
        assert_eq!(
            crate::gui::runtime::handle()
                .block_on(db.load_session())
                .unwrap(),
            files
        );
    }
}
