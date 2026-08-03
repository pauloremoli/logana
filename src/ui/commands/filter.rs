use crate::filters::{FilterOptions, FilterType};
use crate::ui::App;

/// Arguments for `:filter`, bundled to keep `cmd_filter`'s signature small.
pub(super) struct FilterArgs {
    pub pattern: String,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub line_mode: bool,
    pub field: Vec<String>,
    pub regex: bool,
    pub ignore_case: bool,
    pub group: Option<String>,
    /// Generate a random, readable fg/bg pair instead of `fg`/`bg`.
    /// Mutually exclusive with them — see `resolve_auto_colors`.
    pub auto: bool,
}

/// Resolves `fg`/`bg` when `--auto` is set: generates a random, readable
/// color pair (see `theme::random_readable_color_pair`) and formats it as
/// `[r,g,b]` strings, the same shape `FilterOptions::fg`/`bg` already
/// accept from `--fg`/`--bg`. Errors if `--auto` is combined with an
/// explicit `--fg`/`--bg` — silently overriding one or the other would be
/// more surprising than just asking the user to pick one.
fn resolve_auto_colors(
    auto: bool,
    fg: Option<String>,
    bg: Option<String>,
) -> Result<(Option<String>, Option<String>), String> {
    if !auto {
        return Ok((fg, bg));
    }
    if fg.is_some() || bg.is_some() {
        return Err("--auto cannot be combined with --fg/--bg".to_string());
    }
    let (fg_rgb, bg_rgb) = crate::theme::random_readable_color_pair();
    Ok((
        Some(format!("[{},{},{}]", fg_rgb.0, fg_rgb.1, fg_rgb.2)),
        Some(format!("[{},{},{}]", bg_rgb.0, bg_rgb.1, bg_rgb.2)),
    ))
}

/// Builds the stored pattern for `:filter`/`:exclude`/`:highlight` from
/// repeatable `--field key=value` values and the trailing free-text
/// `pattern`. Every field entry becomes an AND'd condition; a non-empty
/// trailing pattern becomes an additional AND'd free-text condition
/// (matched against the full line, like a plain non-field filter).
pub(crate) fn build_field_filter_pattern(
    field: &[String],
    pattern: &str,
) -> Result<String, String> {
    let conditions = field
        .iter()
        .map(|kv| {
            let (k, v) = super::parse_key_value(kv)?;
            Ok((k.to_string(), v.to_string()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let text = if pattern.trim().is_empty() {
        None
    } else {
        Some(pattern)
    };
    Ok(crate::filters::encode_field_filter(&conditions, text))
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
            ignore_case,
            group,
            auto,
        } = args;
        let (fg, bg) = resolve_auto_colors(auto, fg, bg)?;
        let is_field = !field.is_empty();
        let stored_pattern = if is_field {
            build_field_filter_pattern(&field, &pattern)?
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
        if ignore_case {
            opts = opts.ignore_case();
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

        let can_incremental = !is_field
            && !regex
            && !ignore_case
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
        field: Vec<String>,
        regex: bool,
        ignore_case: bool,
        group: Option<String>,
    ) -> Result<bool, String> {
        let is_field = !field.is_empty();
        let stored_pattern = if is_field {
            build_field_filter_pattern(&field, &pattern)?
        } else {
            pattern.clone()
        };

        let mut opts = FilterOptions::default();
        if regex {
            opts = opts.regex();
        }
        if ignore_case {
            opts = opts.ignore_case();
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
                // The incremental path always compiles a case-sensitive
                // literal filter, so a regex or case-insensitive pattern must
                // go through the full refresh instead to match correctly.
                if is_field || regex || ignore_case {
                    self.tabs[self.active_tab].begin_filter_refresh();
                } else {
                    self.tabs[self.active_tab].apply_incremental_exclude(&stored_pattern);
                }
            }
        }
        Ok(false)
    }

    /// `:highlight` (alias `:h`) — like `:filter` but never affects
    /// visibility, so unlike `cmd_filter` there is no incremental
    /// visible-set update to attempt.
    pub(super) async fn cmd_highlight(&mut self, args: FilterArgs) -> Result<bool, String> {
        let FilterArgs {
            pattern,
            fg,
            bg,
            line_mode,
            field,
            regex,
            ignore_case,
            group,
            auto,
        } = args;
        let (fg, bg) = resolve_auto_colors(auto, fg, bg)?;
        let stored_pattern = if !field.is_empty() {
            build_field_filter_pattern(&field, &pattern)?
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
        if ignore_case {
            opts = opts.ignore_case();
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

        if let Some(old_id) = self.tabs[self.active_tab].filter.editing_filter_id.take() {
            self.tabs[self.active_tab]
                .log_manager
                .update_filter(old_id, stored_pattern, FilterType::Highlight, opts)
                .await;
        } else {
            self.tabs[self.active_tab]
                .log_manager
                .add_filter_with_color(stored_pattern, FilterType::Highlight, opts)
                .await;
        }
        self.tabs[self.active_tab].begin_filter_refresh();
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
            && matches!(
                filter.filter_type,
                FilterType::Include | FilterType::Highlight
            )
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

    /// Set, update, or clear the predefined style for group `name`. Filters
    /// in the group with no `color_config` of their own fall back to it
    /// (see `effective_color_config`). The group need not have any filters
    /// yet — this is how a style gets predefined ahead of time.
    pub(super) async fn cmd_group(
        &mut self,
        name: String,
        fg: Option<String>,
        bg: Option<String>,
        line_mode: bool,
        auto: bool,
        clear: bool,
    ) -> Result<bool, String> {
        if clear {
            if fg.is_some() || bg.is_some() || line_mode || auto {
                return Err("--clear cannot be combined with --fg/--bg/-l/--auto".to_string());
            }
            self.tabs[self.active_tab]
                .log_manager
                .clear_group_style(&name)
                .await;
            self.tabs[self.active_tab].begin_filter_refresh();
            return Ok(false);
        }
        let (fg, bg) = resolve_auto_colors(auto, fg, bg)?;
        if fg.is_none() && bg.is_none() && !line_mode {
            return Err("Specify --fg/--bg, -l, or --auto".to_string());
        }
        self.tabs[self.active_tab]
            .log_manager
            .set_group_style(&name, fg.as_deref(), bg.as_deref(), !line_mode)
            .await;
        self.tabs[self.active_tab].begin_filter_refresh();
        Ok(false)
    }
}
