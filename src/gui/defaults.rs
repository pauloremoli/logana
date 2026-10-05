use crate::db::{AppSettingsStore, SettingsKey};
use crate::gui::{app::App, message::Message, state::StatusMessage};
use gpui_kit::gpui::Context;

async fn mappings(db: &crate::db::Database) -> std::collections::HashMap<String, String> {
    crate::config::resolve_default_filter_files(
        &crate::config::Config::load().unwrap_or_default(),
        db,
    )
    .await
}
impl App {
    pub(super) fn prompt_default_mapping(
        &self,
        epoch: u64,
        format: String,
        cx: &mut Context<Self>,
    ) {
        let prompt = cx.prompt_for_paths(gpui_kit::gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose default filters".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = prompt
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()))
                .map(|paths| paths.and_then(|mut paths| paths.pop()));
            this.update_in(cx, |app, window, cx| {
                app.dispatch(
                    Message::DefaultMappingPicked(epoch, format, result),
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn load_default_mappings(&self, epoch: u64, cx: &mut Context<Self>) {
        let db = self.state.db.clone();
        cx.spawn(async move |this, cx| {
            let map = crate::gui::runtime::handle()
                .spawn(async move { mappings(&db).await })
                .await
                .unwrap_or_default();
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::DefaultMappingsLoaded(epoch, map), window, cx)
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn save_default_mapping(
        &self,
        format: String,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let db = self.state.db.clone();
        cx.spawn(async move |this, cx| {
            let result = crate::gui::runtime::handle()
                .spawn(async move {
                    let mut map: std::collections::HashMap<String, String> = db
                        .load_app_setting(SettingsKey::DefaultFilterFiles)
                        .await?
                        .and_then(|json| serde_json::from_str(&json).ok())
                        .unwrap_or_default();
                    if path.trim().is_empty() {
                        map.remove(&format);
                    } else {
                        map.insert(format, path);
                    }
                    db.save_app_setting(
                        SettingsKey::DefaultFilterFiles,
                        &serde_json::to_string(&map)?,
                    )
                    .await
                })
                .await;
            this.update_in(cx, |app, _, cx| {
                app.state.notify(match result {
                    Ok(Ok(())) => StatusMessage::Info("Default filter mapping saved.".into()),
                    Ok(Err(e)) => StatusMessage::Error(e.to_string()),
                    Err(e) => StatusMessage::Error(e.to_string()),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn poll_defaults(&mut self, cx: &mut Context<Self>) {
        self.default_formats
            .retain(|id, _| self.state.tabs.iter().any(|t| t.id == *id));
        for tab in &self.state.tabs {
            let parser = crate::gui::merge::source_parser(tab);
            let name = parser
                .as_deref()
                .map(|f| f.name())
                .unwrap_or("")
                .to_string();
            if self.default_formats.get(&tab.id) == Some(&name) {
                continue;
            }
            self.default_formats.insert(tab.id, name.clone());
            if name.is_empty() || !tab.log_manager.get_filters().is_empty() {
                continue;
            }
            let id = tab.id;
            let mut manager = tab.log_manager.clone();
            let db = self.state.db.clone();
            cx.spawn(async move |this, cx| {
                let format = name.clone();
                let result = crate::gui::runtime::handle()
                    .spawn(async move {
                        let Some(path) = mappings(&db).await.remove(&format) else {
                            return Ok(None);
                        };
                        manager
                            .load_filters(
                                &crate::commands::auto_complete::expand_tilde(&path),
                                false,
                            )
                            .await
                            .map_err(|e| format!("Default filter file {path}: {e}"))?;
                        Ok::<_, String>(Some(manager))
                    })
                    .await;
                this.update_in(cx, |app, _, cx| {
                    let handle = crate::gui::runtime::handle();
                    let _guard = handle.enter();
                    let Some(tab) = app.state.tabs.iter_mut().find(|t| t.id == id) else {
                        return;
                    };
                    if tab.display.format.as_deref().map(|f| f.name()) != Some(name.as_str())
                        || !tab.log_manager.get_filters().is_empty()
                    {
                        return;
                    }
                    match result {
                        Ok(Ok(Some(manager))) => {
                            tab.log_manager = manager;
                            tab.begin_filter_refresh();
                            cx.notify();
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(error)) => {
                            app.state.notify(StatusMessage::Error(error));
                            cx.notify();
                        }
                        Err(error) => {
                            app.state.notify(StatusMessage::Error(error.to_string()));
                            cx.notify();
                        }
                    }
                })
                .ok();
            })
            .detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{AppSettingsStore, SettingsKey};
    #[tokio::test]
    async fn default_file_selection_updates_only_the_current_schema_editor() {
        use crate::gui::{actions::Action, effect::Effect, message::Message, update::update};
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut state = crate::gui::state::GuiState::new(db);
        crate::gui::actions::open(&mut state, Action::DefaultFilters, None, None);
        let epoch = state.editor.as_ref().unwrap().epoch;
        update(
            &mut state,
            Message::DefaultMappingsLoaded(
                epoch,
                std::collections::HashMap::from([
                    ("json".into(), "json-filters.json".into()),
                    ("syslog".into(), "syslog-filters.json".into()),
                ]),
            ),
        );
        update(&mut state, Message::EditorText(0, "json".into()));
        assert_eq!(
            state.editor.as_ref().unwrap().values[1],
            "json-filters.json"
        );
        assert!(
            matches!(update(&mut state, Message::BrowseDefaultMapping), Effect::DefaultMappingDialog { format, .. } if format == "json")
        );
        update(
            &mut state,
            Message::DefaultMappingPicked(epoch, "json".into(), Ok(Some("chosen.json".into()))),
        );
        assert_eq!(state.editor.as_ref().unwrap().values[1], "chosen.json");
        update(&mut state, Message::EditorText(0, "syslog".into()));
        update(
            &mut state,
            Message::DefaultMappingPicked(epoch, "json".into(), Ok(Some("late.json".into()))),
        );
        assert_eq!(
            state.editor.as_ref().unwrap().values[1],
            "syslog-filters.json"
        );
    }
    #[tokio::test]
    async fn persisted_defaults_obey_existing_per_schema_config_precedence() {
        let db = crate::db::Database::in_memory().await.unwrap();
        db.save_app_setting(
            SettingsKey::DefaultFilterFiles,
            r#"{"syslog":"db.json","json":"keep.json"}"#,
        )
        .await
        .unwrap();
        let config = crate::config::Config {
            default_filter_files: Some(std::collections::HashMap::from([(
                "syslog".into(),
                "config.json".into(),
            )])),
            ..Default::default()
        };
        let map = crate::config::resolve_default_filter_files(&config, &db).await;
        assert_eq!(map["syslog"], "config.json");
        assert_eq!(map["json"], "keep.json");
    }
}
