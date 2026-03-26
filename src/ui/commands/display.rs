use crate::theme::Theme;
use crate::ui::App;

impl App {
    pub(super) fn cmd_wrap(&mut self) {
        self.tabs[self.active_tab].display.wrap = !self.tabs[self.active_tab].display.wrap;
    }

    pub(super) fn cmd_line_numbers(&mut self) {
        self.tabs[self.active_tab].display.show_line_numbers =
            !self.tabs[self.active_tab].display.show_line_numbers;
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

    pub(super) fn cmd_set_theme(&mut self, theme_name: String) -> Result<bool, String> {
        let theme_filename = format!("{}.json", theme_name.to_lowercase());
        self.theme = Theme::from_file(&theme_filename)
            .map_err(|e| format!("Failed to load theme '{}': {}", theme_name, e))?;
        for tab in &mut self.tabs {
            tab.cache.render_gen = tab.cache.render_gen.wrapping_add(1);
            tab.cache.render_line.clear();
        }
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
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_select_fields(&mut self) -> Result<bool, String> {
        let tab = &mut self.tabs[self.active_tab];
        let all_names = tab.collect_field_names();
        if all_names.is_empty() {
            return Err("No structured fields found in visible lines".to_string());
        }
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
        ));
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
        tab.invalidate_parse_cache();
    }
}
