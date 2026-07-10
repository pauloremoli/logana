use crate::filters::{FilterOptions, FilterType};
use crate::ui::App;

/// Arguments for `:filter`, bundled to keep `cmd_filter`'s signature small.
pub(super) struct FilterArgs {
    pub pattern: String,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub line_mode: bool,
    pub field: bool,
    pub regex: bool,
    pub group: Option<String>,
}

impl App {
    pub(super) async fn cmd_filter(&mut self, args: FilterArgs) -> Result<bool, String> {
        let FilterArgs {
            pattern,
            fg,
            bg,
            line_mode,
            field,
            regex,
            group,
        } = args;
        let stored_pattern = if field {
            let (key, value) = super::parse_key_value(&pattern)?;
            format!("{}{}:{}", crate::filters::FIELD_PREFIX, key, value)
        } else {
            pattern.clone()
        };

        let mut opts = FilterOptions::default();
        if line_mode {
            opts = opts.line_mode();
        }
        if regex {
            opts = opts.regex();
        }
        if let Some(ref c) = fg {
            opts = opts.fg(c);
        }
        if let Some(ref c) = bg {
            opts = opts.bg(c);
        }
        if let Some(ref g) = group {
            opts = opts.group(g);
        }

        let can_incremental = !field
            && !regex
            && self.tabs[self.active_tab]
                .filter
                .editing_filter_id
                .is_none()
            && {
                let tab = &self.tabs[self.active_tab];
                tab.filter.enabled
                    && !tab.filter.show_marks_only
                    && !tab.log_manager.get_filters().iter().any(|f| {
                        f.enabled
                            && f.filter_type == FilterType::Include
                            && !f.pattern.starts_with(crate::filters::DATE_PREFIX)
                            && !f.pattern.starts_with(crate::filters::FIELD_PREFIX)
                    })
            };

        if let Some(old_id) = self.tabs[self.active_tab].filter.editing_filter_id.take() {
            self.tabs[self.active_tab]
                .log_manager
                .update_filter(old_id, stored_pattern.clone(), FilterType::Include, opts)
                .await;
        } else {
            let was_new = self.tabs[self.active_tab]
                .log_manager
                .add_filter_with_color(stored_pattern.clone(), FilterType::Include, opts)
                .await;
            if was_new && can_incremental {
                self.tabs[self.active_tab].apply_incremental_include(&stored_pattern);
            } else {
                self.tabs[self.active_tab].begin_filter_refresh();
            }
            return Ok(false);
        }
        self.tabs[self.active_tab].begin_filter_refresh();
        Ok(false)
    }

    pub(super) async fn cmd_exclude(
        &mut self,
        pattern: String,
        field: bool,
        regex: bool,
        group: Option<String>,
    ) -> Result<bool, String> {
        let stored_pattern = if field {
            let (key, value) = super::parse_key_value(&pattern)?;
            format!("{}{}:{}", crate::filters::FIELD_PREFIX, key, value)
        } else {
            pattern.clone()
        };

        let mut opts = FilterOptions::default();
        if regex {
            opts = opts.regex();
        }
        if let Some(ref g) = group {
            opts = opts.group(g);
        }

        if let Some(old_id) = self.tabs[self.active_tab].filter.editing_filter_id.take() {
            self.tabs[self.active_tab]
                .log_manager
                .update_filter(old_id, stored_pattern, FilterType::Exclude, opts)
                .await;
            self.tabs[self.active_tab].begin_filter_refresh();
        } else {
            let was_new = self.tabs[self.active_tab]
                .log_manager
                .add_filter_with_color(stored_pattern.clone(), FilterType::Exclude, opts)
                .await;
            if was_new {
                if field {
                    self.tabs[self.active_tab].begin_filter_refresh();
                } else {
                    self.tabs[self.active_tab].apply_incremental_exclude(&stored_pattern);
                }
            }
        }
        Ok(false)
    }

    pub(super) async fn cmd_set_color(
        &mut self,
        fg: Option<String>,
        bg: Option<String>,
        line_mode: bool,
    ) -> Result<bool, String> {
        let selected_filter_index = self.tabs[self.active_tab]
            .filter
            .filter_context
            .unwrap_or(0);
        let filters = self.tabs[self.active_tab].log_manager.get_filters();
        if let Some(filter) = filters.get(selected_filter_index)
            && filter.filter_type == FilterType::Include
        {
            let match_only = if line_mode {
                false
            } else {
                filter
                    .color_config
                    .as_ref()
                    .map(|cc| cc.match_only)
                    .unwrap_or(true)
            };
            let filter_id = filter.id;
            self.tabs[self.active_tab]
                .log_manager
                .set_color_config(filter_id, fg.as_deref(), bg.as_deref(), match_only)
                .await;
            self.tabs[self.active_tab].refresh_filter_colors();
        }
        Ok(false)
    }

    pub(super) async fn cmd_clear_filters(&mut self) -> Result<bool, String> {
        self.tabs[self.active_tab].log_manager.clear_filters().await;
        self.tabs[self.active_tab].begin_filter_refresh();
        Ok(false)
    }

    pub(super) async fn cmd_disable_filters(&mut self) -> Result<bool, String> {
        self.tabs[self.active_tab]
            .log_manager
            .disable_all_filters()
            .await;
        self.tabs[self.active_tab].begin_filter_refresh();
        Ok(false)
    }

    pub(super) async fn cmd_enable_filters(&mut self) -> Result<bool, String> {
        self.tabs[self.active_tab]
            .log_manager
            .enable_all_filters()
            .await;
        self.tabs[self.active_tab].begin_filter_refresh();
        Ok(false)
    }

    pub(super) fn cmd_filtering(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.filter.enabled = !tab.filter.enabled;
        tab.begin_filter_refresh();
    }

    /// Toggle every filter in `group` on/off together: if any member is
    /// currently enabled, disable the whole group; otherwise enable it.
    pub(super) async fn cmd_toggle_group(&mut self, name: String) -> Result<bool, String> {
        let tab = &mut self.tabs[self.active_tab];
        let filters = tab.log_manager.get_filters();
        if !filters
            .iter()
            .any(|f| f.group.as_deref() == Some(name.as_str()))
        {
            return Err(format!("No such filter group: '{name}'"));
        }
        let any_enabled = filters
            .iter()
            .any(|f| f.group.as_deref() == Some(name.as_str()) && f.enabled);
        tab.log_manager
            .set_filters_enabled_by_group(&name, !any_enabled)
            .await;
        tab.begin_filter_refresh();
        Ok(false)
    }
}
