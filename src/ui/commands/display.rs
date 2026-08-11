use crate::db::{AppSettingsStore, SettingsKey};
use crate::theme::Theme;
use crate::ui::App;
use crate::ui::SidebarSide;

impl App {
    pub(super) async fn cmd_wrap(&mut self) {
        self.display.wrap = !self.display.wrap;
        for tab in &mut self.tabs {
            tab.display.wrap = self.display.wrap;
        }
        let _ = self
            .db
            .save_app_setting(
                SettingsKey::Wrap,
                if self.display.wrap { "true" } else { "false" },
            )
            .await;
    }

    pub(super) async fn cmd_line_numbers(&mut self) {
        self.display.show_line_numbers = !self.display.show_line_numbers;
        for tab in &mut self.tabs {
            tab.display.show_line_numbers = self.display.show_line_numbers;
        }
        let _ = self
            .db
            .save_app_setting(
                SettingsKey::ShowLineNumbers,
                if self.display.show_line_numbers {
                    "true"
                } else {
                    "false"
                },
            )
            .await;
    }

    pub(super) async fn cmd_relative_line_numbers(&mut self) {
        self.display.relative_line_numbers = !self.display.relative_line_numbers;
        for tab in &mut self.tabs {
            tab.display.relative_line_numbers = self.display.relative_line_numbers;
        }
        let _ = self
            .db
            .save_app_setting(
                SettingsKey::RelativeLineNumbers,
                if self.display.relative_line_numbers {
                    "true"
                } else {
                    "false"
                },
            )
            .await;
    }

    pub(super) async fn cmd_collapse(&mut self) {
        self.set_collapse_continuations(true).await;
    }

    pub(super) async fn cmd_expand(&mut self) {
        self.set_collapse_continuations(false).await;
    }

    async fn set_collapse_continuations(&mut self, enabled: bool) {
        self.display.collapse_continuations = enabled;
        for tab in &mut self.tabs {
            let current_line = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset);
            tab.display.collapse_continuations = enabled;
            // Restore the pristine baseline first (whether the previous
            // masked state came from the global default or from individual
            // `<`/`>` overrides), then clear overrides and re-derive from
            // scratch — `:collapse`/`:expand` are bulk, idempotent resets,
            // not a toggle relative to whatever was individually flipped.
            if let Some(baseline) = tab.filter.pre_collapse_visible.take() {
                tab.filter.visible_indices = baseline;
            }
            tab.filter.overridden_groups.clear();
            tab.sync_collapse_mask();
            // The line the cursor was on may now be hidden (bulk collapse)
            // or the view may have grown (bulk expand) — re-pin to the
            // nearest still-visible line so the cursor doesn't silently
            // drift onto an unrelated entry.
            tab.restore_scroll_to_line(current_line);
        }
        let _ = self
            .db
            .save_app_setting(
                SettingsKey::CollapseContinuations,
                if enabled { "true" } else { "false" },
            )
            .await;
    }

    pub(super) fn cmd_level_colors(&mut self) -> Result<bool, String> {
        use crate::mode::value_colors_mode::{
            ValueColorEntry, ValueColorGroup as VCGroup, ValueColorsMode,
        };
        let disabled = &self.tabs[self.active_tab].display.level_colors_disabled;
        let levels: Vec<(&str, &str, ratatui::style::Color)> = vec![
            ("trace", "TRACE", self.theme.trace_fg),
            ("debug", "DEBUG", self.theme.debug_fg),
            ("info", "INFO", self.theme.info_fg),
            ("notice", "NOTICE", self.theme.notice_fg),
            ("warning", "WARNING", self.theme.warning_fg),
            ("error", "ERROR", self.theme.error_fg),
            ("fatal", "FATAL", self.theme.fatal_fg),
        ];
        let groups = vec![VCGroup {
            label: "Log levels".to_string(),
            children: levels
                .into_iter()
                .map(|(key, label, color)| ValueColorEntry {
                    key: key.to_string(),
                    label: label.to_string(),
                    color,
                    enabled: !disabled.contains(key),
                })
                .collect(),
        }];
        let original_disabled = disabled.clone();
        self.tabs[self.active_tab].interaction.mode =
            Box::new(ValueColorsMode::new_level_colors(groups, original_disabled));
        Ok(true)
    }

    /// Applies `theme` immediately and invalidates every tab's render cache
    /// so the new colors actually show up — shared by `:set-theme`, the
    /// `:theme` picker's live preview/confirm, and its Esc-revert.
    pub(crate) fn apply_theme(&mut self, theme: Theme) {
        self.theme = theme;
        for tab in &mut self.tabs {
            tab.cache.render_gen = tab.cache.render_gen.wrapping_add(1);
            tab.cache.render_line.clear();
        }
    }

    pub(super) async fn cmd_set_theme(&mut self, theme_name: String) -> Result<bool, String> {
        let theme_filename = format!("{}.json", theme_name.to_lowercase());
        let theme = Theme::from_file(&theme_filename)
            .map_err(|e| format!("Failed to load theme '{}': {}", theme_name, e))?;
        self.apply_theme(theme);
        let _ = self
            .db
            .save_app_setting(SettingsKey::Theme, &theme_name)
            .await;
        Ok(false)
    }

    /// Opens the `:theme` picker (see `ThemePickerMode`), snapshotting the
    /// active theme so Esc can restore it after a live preview.
    pub(super) fn cmd_theme_picker(&mut self) -> Result<bool, String> {
        let entries = Theme::list_available_themes();
        if entries.is_empty() {
            return Err("No themes available".to_string());
        }
        let original_theme = self.theme.clone();
        self.tabs[self.active_tab].interaction.mode = Box::new(
            crate::mode::theme_picker_mode::ThemePickerMode::new(entries, original_theme),
        );
        Ok(true)
    }

    /// Live-previews `theme_name` while the `:theme` picker is open —
    /// applied immediately, not persisted. Silently no-ops on a load
    /// failure (the name came from `Theme::list_available_themes`, so this
    /// should be unreachable in practice).
    pub(crate) fn apply_theme_preview(&mut self, theme_name: &str) {
        let theme_filename = format!("{}.json", theme_name.to_lowercase());
        if let Ok(theme) = Theme::from_file(&theme_filename) {
            self.apply_theme(theme);
        }
    }

    /// Confirms `theme_name` from the `:theme` picker on Enter — loads,
    /// applies, and persists it, surfacing a notification if it fails to
    /// load (the picker already closed, so there's no command-error slot to
    /// report through).
    pub(crate) async fn confirm_theme(&mut self, theme_name: String) {
        if let Err(e) = self.cmd_set_theme(theme_name).await {
            self.tabs[self.active_tab].set_notification(e);
        }
    }

    pub(super) async fn cmd_sidebar_position(&mut self, side: SidebarSide) -> Result<bool, String> {
        self.display.sidebar_side = side;
        for tab in &mut self.tabs {
            tab.display.sidebar_side = side;
        }
        let _ = self
            .db
            .save_app_setting(
                SettingsKey::SidebarLeft,
                if side.is_left() { "true" } else { "false" },
            )
            .await;
        Ok(false)
    }

    pub(super) fn cmd_hide_field(&mut self, field: String) -> Result<bool, String> {
        let resolved = super::resolve_hide_field_arg(&mut self.tabs[self.active_tab], &field)?;
        let tab = &mut self.tabs[self.active_tab];
        tab.display.hidden_fields.insert(resolved);
        tab.invalidate_parse_cache();
        Ok(false)
    }

    pub(super) fn cmd_show_field(&mut self, field: String) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.hidden_fields.remove(&field);
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_show_all_fields(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.hidden_fields.clear();
        tab.display.field_layout.columns = None;
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_select_fields(&mut self) -> Result<bool, String> {
        let tab = &mut self.tabs[self.active_tab];
        let all_names = tab.collect_field_names();
        if all_names.is_empty() {
            return Err("No structured fields found in visible lines".to_string());
        }
        let default_order = all_names.clone();
        let saved_order = &tab.display.field_layout.columns;
        let fields: Vec<(String, bool)> = match saved_order {
            Some(order) => {
                let mut ordered: Vec<(String, bool)> = order
                    .iter()
                    .filter(|n| all_names.contains(n))
                    .map(|n| (n.clone(), !tab.display.hidden_fields.contains(n.as_str())))
                    .collect();
                for name in &all_names {
                    if !order.contains(name) {
                        ordered.push((
                            name.clone(),
                            !tab.display.hidden_fields.contains(name.as_str()),
                        ));
                    }
                }
                ordered
            }
            None => all_names
                .into_iter()
                .map(|n| {
                    let enabled = !tab.display.hidden_fields.contains(n.as_str());
                    (n, enabled)
                })
                .collect(),
        };
        let original_layout = tab.display.field_layout.clone();
        let original_hidden_fields = tab.display.hidden_fields.clone();
        tab.interaction.mode = Box::new(crate::mode::select_fields_mode::SelectFieldsMode::new(
            fields,
            original_layout,
            original_hidden_fields,
            default_order,
        ));
        Ok(true)
    }

    pub(super) fn cmd_merge(&mut self) -> Result<bool, String> {
        self.handle_open_merge_select();
        Ok(true)
    }

    pub(super) fn cmd_show_keys(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.show_keys = true;
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_hide_keys(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.show_keys = false;
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_raw(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.raw_mode = !tab.display.raw_mode;
        tab.begin_filter_refresh();
    }

    pub(super) async fn cmd_schema(&mut self, name: Option<String>) -> Result<bool, String> {
        let tab = &self.tabs[self.active_tab];
        match name {
            None => {
                let schema_name = tab
                    .display
                    .format
                    .as_deref()
                    .map(|f| f.name().to_string())
                    .unwrap_or_else(|| "none".to_string());
                return Err(format!("active schema: {schema_name}"));
            }
            Some(schema_name) if schema_name == "none" => {
                let tab_idx = self.active_tab;
                self.tabs[tab_idx].apply_format(None);
            }
            Some(schema_name) => {
                let custom = crate::config::custom_schemas()
                    .iter()
                    .find(|s| s.name == schema_name)
                    .cloned();
                let parser: std::sync::Arc<dyn crate::parser::LogFormatParser> = match custom {
                    Some(cfg) => std::sync::Arc::new(
                        crate::parser::CustomParser::from_config(&cfg)
                            .map_err(|e| format!("invalid schema '{schema_name}': {e}"))?,
                    ),
                    None => crate::parser::find_builtin_parser(&schema_name)
                        .map(std::sync::Arc::from)
                        .ok_or_else(|| format!("no schema named '{schema_name}'"))?,
                };
                let tab_idx = self.active_tab;
                self.tabs[tab_idx].apply_format(Some(parser));
                self.apply_default_filters_if_empty_at(tab_idx).await;
            }
        }
        Ok(false)
    }

    /// Persists the current `default_filter_files` map, shared by
    /// `cmd_default_filters` (the direct `:default-filters <format> <path>`
    /// form) and the `KeyResult::SetDefaultFilterFile` handler (the popup),
    /// so the SQLite-write line isn't duplicated between the two entry
    /// points that mutate the map.
    pub(crate) async fn persist_default_filter_files(&mut self) {
        let json = serde_json::to_string(&self.default_filter_files).unwrap_or_default();
        let _ = self
            .db
            .save_app_setting(SettingsKey::DefaultFilterFiles, &json)
            .await;
    }

    /// `:default-filters` — no args opens a popup listing every format's
    /// mapping; `<format> <path>` sets one directly; `<format>` alone clears
    /// it. The direct form never touches the currently open tab's filters,
    /// even if its format matches — only tabs whose format is assigned
    /// afterward pick up the change (see `apply_default_filters_if_empty`).
    pub(super) async fn cmd_default_filters(
        &mut self,
        format: Option<String>,
        path: Option<String>,
    ) -> Result<bool, String> {
        match format {
            None => {
                let custom_names: Vec<String> = crate::config::custom_schemas()
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                self.tabs[self.active_tab].interaction.mode =
                    Box::new(crate::mode::default_filters_mode::DefaultFiltersMode::new(
                        &custom_names,
                        &self.default_filter_files,
                    ));
                Ok(true)
            }
            Some(format_name) => {
                self.validate_format_name(&format_name)?;
                match &path {
                    Some(p) => {
                        self.default_filter_files.insert(format_name, p.clone());
                    }
                    None => {
                        self.default_filter_files.remove(&format_name);
                    }
                }
                self.persist_default_filter_files().await;
                Ok(false)
            }
        }
    }

    /// Errors unless `format_name` names a custom schema or a built-in
    /// format — the same name space `:schema` validates against, so
    /// `:default-filters <typo>` fails the same way `:schema <typo>` does.
    fn validate_format_name(&self, format_name: &str) -> Result<(), String> {
        let is_custom = crate::config::custom_schemas()
            .iter()
            .any(|s| s.name == format_name);
        let is_builtin = crate::parser::builtin_format_names()
            .iter()
            .any(|n| n == format_name);
        if is_custom || is_builtin {
            Ok(())
        } else {
            Err(format!("no schema named '{format_name}'"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use crate::theme::Theme;
    use std::sync::Arc;

    async fn make_app() -> App {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line\n".to_vec());
        let lm = LogManager::new(db, None).await;
        App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
            .await
    }

    #[tokio::test]
    async fn test_cmd_schema_no_arg_shows_active_schema_name() {
        let mut app = make_app().await;
        let result = app.cmd_schema(None).await;
        assert_eq!(result, Err("active schema: none".to_string()));
    }

    #[tokio::test]
    async fn test_cmd_schema_unknown_name_returns_error() {
        let mut app = make_app().await;
        let result = app.cmd_schema(Some("nonexistent".to_string())).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("nonexistent"));
    }

    #[tokio::test]
    async fn test_cmd_schema_builtin_name_switches_format() {
        let mut app = make_app().await;
        let result = app.cmd_schema(Some("syslog".to_string())).await;
        assert_eq!(result, Ok(false));
        let tab = &app.tabs[app.active_tab];
        assert_eq!(
            tab.display.format.as_deref().map(|f| f.name()),
            Some("syslog")
        );
    }

    #[tokio::test]
    async fn test_cmd_schema_none_clears_format() {
        let mut app = make_app().await;
        app.cmd_schema(Some("syslog".to_string())).await.unwrap();
        let result = app.cmd_schema(Some("none".to_string())).await;
        assert_eq!(result, Ok(false));
        let tab = &app.tabs[app.active_tab];
        assert!(tab.display.format.is_none());
    }

    /// Drains an active filter scan by polling `advance_filter_computation`
    /// (the real per-tick production path) until it completes.
    async fn drain_filter_scan(app: &mut App) {
        for _ in 0..200 {
            app.advance_filter_computation();
            if app.tabs[app.active_tab].filter.handle.is_none() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("filter scan did not complete in time");
    }

    /// Regression test: toggling raw mode via `:raw` (`cmd_raw`) must bypass
    /// a stale multiline continuation map the same way `:schema none` does —
    /// this exercises the real production scan path
    /// (`App::advance_filter_computation`), not just the sync test helper.
    #[tokio::test]
    async fn test_cmd_raw_bypasses_stale_continuation_grouping_end_to_end() {
        use crate::filters::{FilterOptions, FilterType};
        use crate::parser::CustomParser;

        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(
            b"INFO hello\n  boom NullPointerException\n  at foo.bar(Baz.java:42)\n".to_vec(),
        );
        let lm = LogManager::new(db, None).await;
        let mut app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
            .await;

        // Only the continuation lines mention "NullPointerException" — the
        // header line's own text ("hello") does not.
        let cfg = crate::config::CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{level} {message}".to_string().into()),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
            multiline: true,
            ..Default::default()
        };
        let parser = CustomParser::from_config(&cfg).unwrap();
        let tab = &mut app.tabs[app.active_tab];
        let cmap = crate::ui::build_continuation_map(&tab.file_reader, &parser);
        tab.continuation_map = Some(Arc::new(cmap));
        tab.display.format = Some(Arc::new(parser));

        app.tabs[app.active_tab]
            .log_manager
            .add_filter_with_color(
                "NullPointerException".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;

        // Sanity: under structured (non-raw) multiline grouping, the
        // matching continuation promotes its whole record, including the
        // header, whose own text never matched.
        app.tabs[app.active_tab].begin_filter_refresh();
        drain_filter_scan(&mut app).await;
        let grouped: Vec<usize> = app.tabs[app.active_tab]
            .filter
            .visible_indices
            .iter()
            .collect();
        assert_eq!(
            grouped,
            vec![0, 1, 2],
            "sanity: the whole record should be promoted by the continuation's match"
        );

        // Toggle raw mode via the real `:raw` command path.
        app.cmd_raw();
        drain_filter_scan(&mut app).await;

        let visible: Vec<usize> = app.tabs[app.active_tab]
            .filter
            .visible_indices
            .iter()
            .collect();
        assert_eq!(
            visible,
            vec![1],
            "raw mode must evaluate lines independently of the stale multiline \
             continuation map — only the individually matching line, no group \
             promotion — got {visible:?}"
        );
    }

    /// Regression test: `:schema none` must clear the stale continuation
    /// map and trigger a rescan — previously it only cleared `format`,
    /// leaving the multiline continuation map from the old schema active
    /// and never re-running the filter scan against the new (absent) format.
    #[tokio::test]
    async fn test_cmd_schema_none_clears_continuation_map_and_rescans() {
        use crate::filters::{FilterOptions, FilterType};

        let mut app = make_app().await;
        app.cmd_schema(Some("syslog".to_string())).await.unwrap();
        app.tabs[app.active_tab].continuation_map =
            Some(std::sync::Arc::new(vec![
                0usize;
                app.tabs[app.active_tab]
                    .file_reader
                    .line_count()
            ]));
        app.tabs[app.active_tab]
            .log_manager
            .add_filter_with_color(
                "line".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;

        app.cmd_schema(Some("none".to_string())).await.unwrap();

        let tab = &app.tabs[app.active_tab];
        assert!(
            tab.continuation_map.is_none(),
            "switching to no schema must drop the previous schema's continuation map"
        );
        assert!(
            tab.filter.handle.is_some(),
            "switching schema must (re)run the filter scan, not just clear the format"
        );
    }

    #[tokio::test]
    async fn test_cmd_schema_none_does_not_apply_default_filters() {
        let mut app = make_app().await;
        app.default_filter_files
            .insert("syslog".to_string(), "/nonexistent.json".to_string());
        app.cmd_schema(Some("none".to_string())).await.unwrap();
        let tab = &app.tabs[app.active_tab];
        assert!(tab.log_manager.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_cmd_schema_switch_applies_default_filters_when_empty() {
        let mut app = make_app().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        std::fs::write(
            &path,
            r#"[{"id":1,"pattern":"error","filter_type":"Include","enabled":true,"color_config":null,"use_regex":false,"ignore_case":false,"group":null}]"#,
        )
        .unwrap();
        app.default_filter_files
            .insert("syslog".to_string(), path.to_str().unwrap().to_string());
        app.cmd_schema(Some("syslog".to_string())).await.unwrap();
        let tab = &app.tabs[app.active_tab];
        assert_eq!(tab.log_manager.get_filters().len(), 1);
        assert_eq!(tab.log_manager.get_filters()[0].pattern, "error");
    }

    #[tokio::test]
    async fn test_cmd_schema_switch_does_not_apply_when_tab_has_filters() {
        use crate::filters::FilterType;
        let mut app = make_app().await;
        app.default_filter_files
            .insert("syslog".to_string(), "/nonexistent.json".to_string());
        app.tabs[app.active_tab]
            .log_manager
            .add_filter_with_color(
                "existing".to_string(),
                FilterType::Include,
                Default::default(),
            )
            .await;
        app.cmd_schema(Some("syslog".to_string())).await.unwrap();
        let tab = &app.tabs[app.active_tab];
        assert_eq!(tab.log_manager.get_filters().len(), 1);
        assert_eq!(tab.log_manager.get_filters()[0].pattern, "existing");
    }

    #[tokio::test]
    async fn test_cmd_schema_switch_no_mapping_is_noop() {
        let mut app = make_app().await;
        app.cmd_schema(Some("syslog".to_string())).await.unwrap();
        let tab = &app.tabs[app.active_tab];
        assert!(tab.log_manager.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_cmd_schema_unknown_name_still_errors_before_any_apply() {
        let mut app = make_app().await;
        app.default_filter_files
            .insert("acme".to_string(), "/nonexistent.json".to_string());
        let result = app.cmd_schema(Some("acme".to_string())).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("acme"));
        assert!(
            app.tabs[app.active_tab]
                .log_manager
                .get_filters()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_cmd_default_filters_bare_opens_popup() {
        use crate::mode::app_mode::ModeRenderState;
        let mut app = make_app().await;
        let result = app.cmd_default_filters(None, None).await;
        assert_eq!(result, Ok(true));
        assert!(matches!(
            app.tabs[app.active_tab].interaction.mode.render_state(),
            ModeRenderState::DefaultFilters { .. }
        ));
    }

    #[tokio::test]
    async fn test_cmd_default_filters_set_direct_form() {
        let mut app = make_app().await;
        let result = app
            .cmd_default_filters(Some("syslog".to_string()), Some("/tmp/f.json".to_string()))
            .await;
        assert_eq!(result, Ok(false));
        assert_eq!(
            app.default_filter_files.get("syslog"),
            Some(&"/tmp/f.json".to_string())
        );
    }

    #[tokio::test]
    async fn test_cmd_default_filters_clear_direct_form() {
        let mut app = make_app().await;
        app.default_filter_files
            .insert("syslog".to_string(), "/tmp/f.json".to_string());
        let result = app
            .cmd_default_filters(Some("syslog".to_string()), None)
            .await;
        assert_eq!(result, Ok(false));
        assert!(!app.default_filter_files.contains_key("syslog"));
    }

    #[tokio::test]
    async fn test_cmd_default_filters_unknown_format_errors() {
        let mut app = make_app().await;
        let result = app
            .cmd_default_filters(
                Some("not-a-real-format".to_string()),
                Some("/tmp/f.json".to_string()),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not-a-real-format"));
        assert!(app.default_filter_files.is_empty());
    }

    #[tokio::test]
    async fn test_cmd_default_filters_set_persists_to_sqlite() {
        let mut app = make_app().await;
        app.cmd_default_filters(Some("syslog".to_string()), Some("/tmp/f.json".to_string()))
            .await
            .unwrap();
        let saved = app
            .db
            .load_app_setting(SettingsKey::DefaultFilterFiles)
            .await
            .unwrap()
            .unwrap();
        assert!(saved.contains("syslog"));
        assert!(saved.contains("/tmp/f.json"));
    }

    #[tokio::test]
    async fn test_cmd_default_filters_direct_form_no_retroactive_effect() {
        let mut app = make_app().await;
        app.tabs[app.active_tab].display.format =
            crate::parser::find_builtin_parser("syslog").map(std::sync::Arc::from);
        app.cmd_default_filters(Some("syslog".to_string()), Some("/tmp/f.json".to_string()))
            .await
            .unwrap();
        assert!(
            app.tabs[app.active_tab]
                .log_manager
                .get_filters()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_cmd_default_filters_custom_schema_direct_form_accepted() {
        // Custom schemas are read from a process-global OnceLock that's never
        // set in tests, so this exercises only the built-in-name branch of
        // validate_format_name — the custom-schema branch is covered by
        // `schema_completion_names`/`complete_schema_with_builtins`'s own
        // tests in auto_complete.rs.
        let mut app = make_app().await;
        let result = app
            .cmd_default_filters(Some("json".to_string()), Some("/tmp/f.json".to_string()))
            .await;
        assert_eq!(result, Ok(false));
    }

    #[tokio::test]
    async fn test_cmd_theme_picker_opens_mode_and_keeps_current_theme() {
        let mut app = make_app().await;
        let before = app.theme.clone();
        let result = app.cmd_theme_picker();
        assert_eq!(result, Ok(true));
        assert_eq!(
            app.theme, before,
            "opening the picker must not change the theme yet"
        );
        assert!(matches!(
            app.tabs[app.active_tab].interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::ThemePicker { .. }
        ));
    }

    #[tokio::test]
    async fn test_apply_theme_preview_applies_bundled_theme_without_persisting() {
        let mut app = make_app().await;
        let gen_before = app.tabs[app.active_tab].cache.render_gen;
        app.apply_theme_preview("dracula");
        assert_ne!(app.theme, Theme::default());
        assert_ne!(app.tabs[app.active_tab].cache.render_gen, gen_before);
        assert!(
            app.db
                .load_app_setting(crate::db::SettingsKey::Theme)
                .await
                .unwrap()
                .is_none(),
            "preview must not persist to the DB"
        );
    }

    #[tokio::test]
    async fn test_apply_theme_preview_unknown_name_is_a_no_op() {
        let mut app = make_app().await;
        let before = app.theme.clone();
        app.apply_theme_preview("does-not-exist");
        assert_eq!(app.theme, before);
    }

    #[tokio::test]
    async fn test_confirm_theme_applies_and_persists() {
        let mut app = make_app().await;
        app.confirm_theme("dracula".to_string()).await;
        assert_ne!(app.theme, Theme::default());
        assert_eq!(
            app.db
                .load_app_setting(crate::db::SettingsKey::Theme)
                .await
                .unwrap(),
            Some("dracula".to_string())
        );
    }

    #[tokio::test]
    async fn test_confirm_theme_unknown_name_sets_notification() {
        let mut app = make_app().await;
        app.confirm_theme("does-not-exist".to_string()).await;
        assert!(app.tabs[app.active_tab].interaction.notification.is_some());
    }

    #[tokio::test]
    async fn test_apply_theme_reverts_to_given_theme_and_bumps_cache() {
        let mut app = make_app().await;
        app.apply_theme_preview("dracula");
        let gen_after_preview = app.tabs[app.active_tab].cache.render_gen;
        app.apply_theme(Theme::default());
        assert_eq!(app.theme, Theme::default());
        assert_ne!(app.tabs[app.active_tab].cache.render_gen, gen_after_preview);
    }
}
