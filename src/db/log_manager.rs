use crate::db::{Database, FilterStore, GroupStore};
use crate::filters::{ColorConfig, FilterDef, FilterInsertOptions, FilterOptions, FilterType};
use crate::filters::{DATE_PREFIX, DateFilterStyle, parse_date_filter};
use crate::filters::{FilterDecision, FilterManager, GroupDef, StyleId, build_filter};
use crate::theme::parse_color;
use aho_corasick::AhoCorasick;
use ratatui::style::Style;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// On-disk shape for `:save-filters`/`:load-filters`.
#[derive(Serialize, Deserialize)]
struct FiltersExport {
    filters: Vec<FilterDef>,
    #[serde(default)]
    groups: Vec<GroupDef>,
}

/// Rebuilds the `FilterOptions` a `FilterDef` was originally created with, so
/// append-merge paths (`load_filters`, `import_npp_filters`) can feed it back
/// through `add_filter_with_color`.
fn filter_options_from_def(def: &FilterDef) -> FilterOptions {
    let mut options = FilterOptions::default();
    if let Some(cc) = &def.color_config {
        if let Some(fg) = cc.fg {
            options = options.fg(&crate::theme::color_to_string(fg));
        }
        if let Some(bg) = cc.bg {
            options = options.bg(&crate::theme::color_to_string(bg));
        }
        if !cc.match_only {
            options = options.line_mode();
        }
    }
    if def.use_regex {
        options = options.regex();
    }
    if def.ignore_case {
        options = options.ignore_case();
    }
    if let Some(group) = &def.group {
        options = options.group(group);
    }
    options
}

pub struct LogManager {
    pub db: Arc<Database>,
    source_file: Option<String>,
    filter_defs: Vec<FilterDef>,
    group_defs: Vec<GroupDef>,
}

impl LogManager {
    pub async fn new(db: Arc<Database>, source_file: Option<String>) -> Self {
        let mut mgr = LogManager {
            db,
            source_file,
            filter_defs: Vec::new(),
            group_defs: Vec::new(),
        };
        mgr.reload_filters_from_db().await;
        mgr.reload_groups_from_db().await;
        mgr
    }

    pub fn source_file(&self) -> Option<&str> {
        self.source_file.as_deref()
    }

    /// `source_file`, defaulting to `""` (the "no file" bucket filters
    /// already use for sourceless/stdin tabs) — the scoping key every
    /// group-related DB write/read needs to stay isolated per tab.
    fn group_source(&self) -> &str {
        self.source_file.as_deref().unwrap_or("")
    }

    pub async fn set_source_file(&mut self, source: Option<String>) {
        self.source_file = source;
        self.reload_filters_from_db().await;
        self.reload_groups_from_db().await;
    }

    pub fn get_filters(&self) -> &[FilterDef] {
        &self.filter_defs
    }

    pub async fn add_filter_with_color(
        &mut self,
        pattern: String,
        filter_type: FilterType,
        options: FilterOptions,
    ) -> bool {
        let color_config = matches!(filter_type, FilterType::Include | FilterType::Highlight)
            .then(|| ColorConfig::try_from(&options).ok())
            .flatten();

        if let Some(pos) = self
            .filter_defs
            .iter()
            .position(|f| f.pattern == pattern && f.filter_type == filter_type)
        {
            self.filter_defs[pos].color_config = color_config.clone();
            self.filter_defs[pos].group = options.group.clone();
            let id = self.filter_defs[pos].id;
            let _ = self
                .db
                .update_filter_color(id as i64, color_config.as_ref())
                .await;
            let _ = self
                .db
                .update_filter_group(id as i64, options.group.as_deref())
                .await;
            return false;
        }

        let pattern_clone = pattern.clone();
        let filter_type_clone = filter_type.clone();
        let cc_clone = color_config.clone();
        let source = self.source_file.clone();

        let mut insert_opts = FilterInsertOptions::new();
        if let Some(cc) = cc_clone {
            insert_opts = insert_opts.color(cc);
        }
        if let Some(src) = source {
            insert_opts = insert_opts.source(src);
        }
        if options.use_regex {
            insert_opts = insert_opts.regex();
        }
        if options.ignore_case {
            insert_opts = insert_opts.ignore_case();
        }
        let id = self
            .db
            .insert_filter(&pattern_clone, &filter_type_clone, insert_opts)
            .await
            .unwrap_or(0) as usize;

        let next_id = if id > 0 {
            id
        } else {
            self.filter_defs.iter().map(|f| f.id).max().unwrap_or(0) + 1
        };

        self.filter_defs.push(FilterDef {
            id: next_id,
            pattern,
            filter_type,
            enabled: true,
            color_config,
            use_regex: options.use_regex,
            ignore_case: options.ignore_case,
            group: options.group.clone(),
        });
        true
    }

    pub async fn toggle_filter(&mut self, id: usize) {
        if let Some(f) = self.filter_defs.iter_mut().find(|f| f.id == id) {
            f.enabled = !f.enabled;
        }
        let _ = self.db.toggle_filter(id as i64).await;
    }

    pub async fn remove_filter(&mut self, id: usize) {
        self.filter_defs.retain(|f| f.id != id);
        let _ = self.db.delete_filter(id as i64).await;
    }

    pub async fn disable_all_filters(&mut self) {
        for f in self.filter_defs.iter_mut() {
            f.enabled = false;
        }
        let _ = self.db.set_all_filters_enabled(false).await;
    }

    pub async fn enable_all_filters(&mut self) {
        for f in self.filter_defs.iter_mut() {
            f.enabled = true;
        }
        let _ = self.db.set_all_filters_enabled(true).await;
    }

    pub async fn clear_filters(&mut self) {
        self.filter_defs.clear();
        let source = self.source_file.clone();
        if let Some(src) = source {
            let _ = self.db.clear_filters_for_source(&src).await;
        } else {
            let _ = self.db.clear_filters().await;
        }
    }

    pub async fn edit_filter(&mut self, id: usize, new_pattern: String) {
        if let Some(f) = self.filter_defs.iter_mut().find(|f| f.id == id) {
            f.pattern = new_pattern.clone();
        }
        let _ = self.db.update_filter_pattern(id as i64, &new_pattern).await;
    }

    pub async fn update_filter(
        &mut self,
        id: usize,
        pattern: String,
        filter_type: FilterType,
        options: FilterOptions,
    ) {
        let color_config = matches!(filter_type, FilterType::Include | FilterType::Highlight)
            .then(|| ColorConfig::try_from(&options).ok())
            .flatten();
        if let Some(f) = self.filter_defs.iter_mut().find(|f| f.id == id) {
            f.pattern = pattern.clone();
            f.filter_type = filter_type.clone();
            f.color_config = color_config.clone();
            f.use_regex = options.use_regex;
            f.ignore_case = options.ignore_case;
            f.group = options.group.clone();
        }
        let _ = self
            .db
            .update_filter(
                id as i64,
                &pattern,
                &filter_type,
                color_config.as_ref(),
                options.use_regex,
                options.ignore_case,
                options.group.as_deref(),
            )
            .await;
    }

    /// Distinct group names among the current filters, unioned with groups
    /// that have a predefined style but no filters yet, sorted alphabetically.
    pub fn group_names(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> =
            crate::filters::known_groups(&self.filter_defs)
                .into_iter()
                .collect();
        set.extend(self.group_defs.iter().map(|g| g.name.clone()));
        set.into_iter().collect()
    }

    pub fn get_group_styles(&self) -> &[GroupDef] {
        &self.group_defs
    }

    /// Set (or update) the predefined style for group `name`. `fg`/`bg` are
    /// string-parsed via `theme::parse_color`, same as filter colors.
    pub async fn set_group_style(
        &mut self,
        name: &str,
        fg: Option<&str>,
        bg: Option<&str>,
        match_only: bool,
    ) {
        let cc = ColorConfig {
            fg: fg.and_then(parse_color),
            bg: bg.and_then(parse_color),
            match_only,
        };
        if let Some(g) = self.group_defs.iter_mut().find(|g| g.name == name) {
            g.color_config = Some(cc.clone());
        } else {
            self.group_defs.push(GroupDef {
                name: name.to_string(),
                color_config: Some(cc.clone()),
                ..Default::default()
            });
        }
        let _ = self
            .db
            .upsert_group_style(self.group_source(), name, &cc)
            .await;
    }

    /// Remove group `name`'s predefined style, if any. A no-op if the group
    /// has no stored style.
    pub async fn clear_group_style(&mut self, name: &str) {
        self.group_defs.retain(|g| g.name != name);
        let _ = self.db.clear_group_style(self.group_source(), name).await;
    }

    /// Removes group `name` entirely: every filter belonging to it plus its
    /// predefined style, if any. Mirrors `clear_group_style` + a bulk
    /// `remove_filter` over the group's members.
    pub async fn remove_group(&mut self, name: &str) {
        self.filter_defs
            .retain(|f| f.group.as_deref() != Some(name));
        self.group_defs.retain(|g| g.name != name);
        let _ = self
            .db
            .remove_filters_by_group(self.group_source(), name)
            .await;
        let _ = self.db.clear_group_style(self.group_source(), name).await;
    }

    /// Sets group `name`'s own `enabled` flag, creating a bare (styleless)
    /// entry for it if none exists yet. This is the source of truth `A`/space
    /// falls back to in `GroupManagementMode` when the group has no filters
    /// of its own to derive a toggle state from.
    pub async fn set_group_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(g) = self.group_defs.iter_mut().find(|g| g.name == name) {
            g.enabled = enabled;
        } else {
            self.group_defs.push(GroupDef {
                name: name.to_string(),
                enabled,
                ..Default::default()
            });
        }
        let _ = self
            .db
            .set_group_enabled(self.group_source(), name, enabled)
            .await;
    }

    /// Set `enabled` on every filter belonging to `group`.
    pub async fn set_filters_enabled_by_group(&mut self, group: &str, enabled: bool) {
        for f in self.filter_defs.iter_mut() {
            if f.group.as_deref() == Some(group) {
                f.enabled = enabled;
            }
        }
        let _ = self
            .db
            .set_filters_enabled_by_group(self.group_source(), group, enabled)
            .await;
    }

    pub async fn move_filter_up(&mut self, id: usize) {
        if let Some(idx) = self.filter_defs.iter().position(|f| f.id == id)
            && idx > 0
        {
            self.filter_defs.swap(idx, idx - 1);
            let other_id = self.filter_defs[idx].id;
            let _ = self.db.swap_filter_order(id as i64, other_id as i64).await;
        }
    }

    pub async fn move_filter_down(&mut self, id: usize) {
        if let Some(idx) = self.filter_defs.iter().position(|f| f.id == id)
            && idx + 1 < self.filter_defs.len()
        {
            self.filter_defs.swap(idx, idx + 1);
            let other_id = self.filter_defs[idx].id;
            let _ = self.db.swap_filter_order(id as i64, other_id as i64).await;
        }
    }

    pub async fn set_color_config(
        &mut self,
        filter_id: usize,
        fg: Option<&str>,
        bg: Option<&str>,
        match_only: bool,
    ) {
        let fg_color = fg.and_then(parse_color);
        let bg_color = bg.and_then(parse_color);
        if fg_color.is_none() && bg_color.is_none() && match_only {
            return;
        }
        let cc = ColorConfig {
            fg: fg_color,
            bg: bg_color,
            match_only,
        };
        if let Some(f) = self.filter_defs.iter_mut().find(|f| f.id == filter_id) {
            f.color_config = Some(cc.clone());
        }
        let _ = self
            .db
            .update_filter_color(filter_id as i64, Some(&cc))
            .await;
    }

    pub fn save_filters(&self, path: &str) -> anyhow::Result<()> {
        let export = FiltersExport {
            filters: self.filter_defs.clone(),
            groups: self.group_defs.clone(),
        };
        let json = serde_json::to_string_pretty(&export)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Loads filters (and, per `FiltersExport`, group styles) from `path`.
    /// Tries the current `{ filters, groups }` object shape first; a bare
    /// `[...]` array — the pre-groups format — can never deserialize into
    /// that struct, so falling back to it is deterministic, not heuristic.
    /// With `append`, entries are merged into the current filters/groups
    /// (added, or updated in place if a same-pattern-and-type filter or
    /// same-name group already exists); otherwise the current filters and
    /// groups are replaced entirely.
    pub async fn load_filters(&mut self, path: &str, append: bool) -> anyhow::Result<()> {
        let json = std::fs::read_to_string(path)?;
        let (filters, groups) = match serde_json::from_str::<FiltersExport>(&json) {
            Ok(export) => (export.filters, export.groups),
            Err(_) => (serde_json::from_str::<Vec<FilterDef>>(&json)?, Vec::new()),
        };
        if append {
            self.merge_filters_and_groups(filters, groups).await;
            Ok(())
        } else {
            self.replace_filters_and_reload(filters, groups).await
        }
    }

    /// Adds/updates each of `filters` and `groups` in place, leaving
    /// everything else untouched. Shared merge logic for `load_filters` and
    /// `import_npp_filters`'s append mode.
    async fn merge_filters_and_groups(&mut self, filters: Vec<FilterDef>, groups: Vec<GroupDef>) {
        for def in filters {
            let options = filter_options_from_def(&def);
            self.add_filter_with_color(def.pattern, def.filter_type, options)
                .await;
        }
        for group in groups {
            if let Some(cc) = &group.color_config {
                self.set_group_style(
                    &group.name,
                    cc.fg.map(crate::theme::color_to_string).as_deref(),
                    cc.bg.map(crate::theme::color_to_string).as_deref(),
                    cc.match_only,
                )
                .await;
            }
            self.set_group_enabled(&group.name, group.enabled).await;
        }
    }

    /// Imports a Notepad++ Analyze-plugin XML config (AnalyseDoc or User
    /// Defined Language export) from `path` as `Include` filters. With
    /// `append`, converted entries are merged into the current filters
    /// (added, or updated in place if a same-pattern-and-type filter already
    /// exists); otherwise the current filters and groups are replaced
    /// entirely, same as `load_filters`.
    pub async fn import_npp_filters(&mut self, path: &str, append: bool) -> anyhow::Result<()> {
        let xml = std::fs::read_to_string(path)?;
        let filters = crate::commands::convert_npp_xml(&xml).map_err(anyhow::Error::msg)?;
        if append {
            self.merge_filters_and_groups(filters, Vec::new()).await;
            Ok(())
        } else {
            self.replace_filters_and_reload(filters, Vec::new()).await
        }
    }

    /// Replaces all filters/groups for the current source with `filters`/
    /// `groups`, persists to the db, and reloads `self.filter_defs`/
    /// `self.group_defs` from it. Shared tail of `load_filters` and
    /// `import_npp_filters`'s replace mode.
    async fn replace_filters_and_reload(
        &mut self,
        filters: Vec<FilterDef>,
        groups: Vec<GroupDef>,
    ) -> anyhow::Result<()> {
        let source = self.source_file.clone();
        self.db
            .replace_all_filters(&filters, source.as_deref())
            .await?;
        self.db
            .replace_all_groups(&groups, source.as_deref())
            .await?;
        self.filter_defs = if let Some(src) = source.as_deref() {
            self.db.get_filters_for_source(src).await
        } else {
            self.db.get_filters().await
        }
        .unwrap_or_default();
        self.group_defs = if let Some(src) = source.as_deref() {
            self.db.get_groups_for_source(src).await
        } else {
            self.db.get_groups().await
        }
        .unwrap_or_default();
        Ok(())
    }

    pub fn reset_in_memory(&mut self) {
        self.filter_defs.clear();
    }

    /// Build a `FilterManager`, its associated `Vec<Style>`, and date filter styles
    /// from the current enabled filter definitions.
    ///
    /// `StyleId` is the index into the returned `Vec<Style>`. Date filters with a
    /// `color_config` are returned separately in `Vec<DateFilterStyle>` so the render
    /// path can highlight the timestamp column of matching lines.
    pub fn build_filter_manager(
        &self,
    ) -> (
        FilterManager,
        Vec<Style>,
        Vec<DateFilterStyle>,
        Vec<crate::filters::FieldFilterStyle>,
    ) {
        let mut filters: Vec<Box<dyn crate::filters::Filter>> = Vec::new();
        let mut styles: Vec<Style> = Vec::new();
        let mut date_filter_styles: Vec<DateFilterStyle> = Vec::new();
        let mut field_filter_styles: Vec<crate::filters::FieldFilterStyle> = Vec::new();
        let mut has_include = false;
        let mut literal_patterns: Vec<String> = Vec::new();
        let mut combined_ac_meta: Vec<(usize, FilterDecision)> = Vec::new();
        let mut slow_path_filter_indices: Vec<usize> = Vec::new();

        let mut style_idx: usize = 0;
        for def in self.filter_defs.iter().filter(|f| f.enabled) {
            // Field filters: applied separately for visibility; collect styles for highlighting.
            if def.pattern.starts_with(crate::filters::FIELD_PREFIX) {
                if let Some(cc) = crate::filters::effective_color_config(def, &self.group_defs)
                    && (cc.fg.is_some() || cc.bg.is_some())
                    && let Ok((conditions, text)) = crate::filters::parse_field_filter_expr(
                        &def.pattern[crate::filters::FIELD_PREFIX.len()..],
                    )
                {
                    let style_id = style_idx as crate::filters::StyleId;
                    style_idx += 1;
                    let mut s = Style::default();
                    if let Some(fg) = cc.fg {
                        s = s.fg(fg);
                    }
                    if let Some(bg) = cc.bg {
                        s = s.bg(bg);
                    }
                    styles.push(s);
                    let decision = match def.filter_type {
                        FilterType::Include => FilterDecision::Include,
                        FilterType::Exclude => FilterDecision::Exclude,
                        FilterType::Highlight => FilterDecision::Highlight,
                    };
                    field_filter_styles.push(crate::filters::FieldFilterStyle {
                        field_filter: crate::filters::FieldFilter {
                            conditions,
                            text,
                            decision,
                        },
                        style_id,
                        match_only: cc.match_only,
                    });
                }
                continue;
            }

            if def.pattern.starts_with(DATE_PREFIX) {
                if let Some(cc) = crate::filters::effective_color_config(def, &self.group_defs)
                    && (cc.fg.is_some() || cc.bg.is_some())
                    && let Ok(df) = parse_date_filter(&def.pattern[DATE_PREFIX.len()..])
                {
                    let style_id = style_idx as StyleId;
                    style_idx += 1;
                    let mut s = Style::default();
                    if let Some(fg) = cc.fg {
                        s = s.fg(fg);
                    }
                    if let Some(bg) = cc.bg {
                        s = s.bg(bg);
                    }
                    styles.push(s);
                    date_filter_styles.push(DateFilterStyle {
                        filter: df,
                        style_id,
                        match_only: cc.match_only,
                    });
                }
                continue;
            }

            let style_id = style_idx as StyleId;
            style_idx += 1;

            let effective_cc = crate::filters::effective_color_config(def, &self.group_defs);

            let style = effective_cc
                .map(|cc| {
                    let mut s = Style::default();
                    if let Some(fg) = cc.fg {
                        s = s.fg(fg);
                    }
                    if let Some(bg) = cc.bg {
                        s = s.bg(bg);
                    }
                    s
                })
                .unwrap_or_default();

            styles.push(style);

            let decision = match def.filter_type {
                FilterType::Include => {
                    has_include = true;
                    FilterDecision::Include
                }
                FilterType::Exclude => FilterDecision::Exclude,
                FilterType::Highlight => FilterDecision::Highlight,
            };

            let match_only = effective_cc.map(|cc| cc.match_only).unwrap_or(true);

            if let Some(f) = build_filter(
                &def.pattern,
                decision,
                match_only,
                style_id,
                def.use_regex,
                def.ignore_case,
            ) {
                let filter_idx = filters.len();
                // Case-insensitive literal patterns can't share the shared
                // case-sensitive combined automaton (see FilterManager's
                // `slow_path_filter_indices` doc comment) — they fall back to
                // individual evaluation, same as regex filters.
                if def.use_regex || def.ignore_case {
                    slow_path_filter_indices.push(filter_idx);
                } else {
                    combined_ac_meta.push((filter_idx, decision));
                    literal_patterns.push(def.pattern.clone());
                }
                filters.push(f);
            }
        }

        let combined_ac = if literal_patterns.len() >= 2 {
            AhoCorasick::builder()
                .ascii_case_insensitive(false)
                .build(&literal_patterns)
                .ok()
        } else {
            None
        };

        // Reserve the last slot for search highlights (StyleId = styles.len()).
        // The caller appends the search style.

        (
            FilterManager::new_with_combined(
                filters,
                has_include,
                combined_ac,
                combined_ac_meta,
                slow_path_filter_indices,
            ),
            styles,
            date_filter_styles,
            field_filter_styles,
        )
    }

    pub fn compute_file_hash(path: &str) -> Option<String> {
        let metadata = std::fs::metadata(path).ok()?;
        let size = metadata.len();
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let mut hasher = DefaultHasher::new();
        size.hash(&mut hasher);
        modified.hash(&mut hasher);
        Some(format!("{:x}", hasher.finish()))
    }

    async fn reload_filters_from_db(&mut self) {
        let source = match self.source_file.as_deref() {
            Some(src) => src.to_string(),
            None => return,
        };
        self.filter_defs = self
            .db
            .get_filters_for_source(&source)
            .await
            .unwrap_or_default();
    }

    async fn reload_groups_from_db(&mut self) {
        let source = match self.source_file.as_deref() {
            Some(src) => src.to_string(),
            None => return,
        };
        self.group_defs = self
            .db
            .get_groups_for_source(&source)
            .await
            .unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_manager() -> LogManager {
        let db = Arc::new(Database::in_memory().await.unwrap());
        LogManager::new(db, None).await
    }

    #[tokio::test]
    async fn test_new_without_source_has_no_filters() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        // Insert a filter without a source (global) directly via a manager that has a source.
        let mut seeder = LogManager::new(db.clone(), Some("file.log".into())).await;
        seeder
            .add_filter_with_color(
                "error".into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;

        // A placeholder tab (no source) must not expose those filters.
        let mgr = LogManager::new(db, None).await;
        assert!(mgr.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_group_style_is_scoped_per_source_like_filters() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut a = LogManager::new(db.clone(), Some("a.log".into())).await;
        let mut b = LogManager::new(db, Some("b.log".into())).await;

        a.set_group_style("errors", Some("Red"), None, true).await;
        b.set_group_style("errors", Some("Blue"), None, true).await;

        let a_style = crate::filters::group_style(a.get_group_styles(), "errors").unwrap();
        let b_style = crate::filters::group_style(b.get_group_styles(), "errors").unwrap();
        assert_eq!(a_style.fg, Some(ratatui::style::Color::Red));
        assert_eq!(b_style.fg, Some(ratatui::style::Color::Blue));

        a.clear_group_style("errors").await;
        assert!(a.get_group_styles().is_empty());
        assert_eq!(
            b.get_group_styles().len(),
            1,
            "clearing a's group must not affect b"
        );
    }

    #[tokio::test]
    async fn test_add_and_get_filters() {
        let mut mgr = make_manager().await;
        assert!(mgr.get_filters().is_empty());

        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color(
            "debug".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;

        let filters = mgr.get_filters();
        assert_eq!(filters.len(), 2);
        // Oldest first: "error" was added first so it sits at index 0
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[1].pattern, "debug");
        assert_eq!(filters[1].filter_type, FilterType::Exclude);
    }

    #[tokio::test]
    async fn test_toggle_filter() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        let id = mgr.get_filters()[0].id;

        assert!(mgr.get_filters()[0].enabled);
        mgr.toggle_filter(id).await;
        assert!(!mgr.get_filters()[0].enabled);
        mgr.toggle_filter(id).await;
        assert!(mgr.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn test_remove_filter() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color(
            "debug".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;
        let id = mgr.get_filters()[0].id;

        // "error" was added first → it is at index 0; removing it leaves "debug"
        mgr.remove_filter(id).await;
        assert_eq!(mgr.get_filters().len(), 1);
        assert_eq!(mgr.get_filters()[0].pattern, "debug");
    }

    #[tokio::test]
    async fn test_edit_filter() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        let id = mgr.get_filters()[0].id;

        mgr.edit_filter(id, "critical".into()).await;
        assert_eq!(mgr.get_filters()[0].pattern, "critical");
    }

    #[tokio::test]
    async fn test_move_filter_up_down() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "first".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color(
            "second".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color(
            "third".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

        // After three inserts (oldest first): ["first", "second", "third"]
        // "second" is at index 1
        let id_second = mgr.get_filters()[1].id;
        mgr.move_filter_up(id_second).await;

        // Swaps [1] and [0]: ["second", "first", "third"]
        let filters = mgr.get_filters();
        assert_eq!(filters[0].pattern, "second");
        assert_eq!(filters[1].pattern, "first");
        assert_eq!(filters[2].pattern, "third");

        // "first" is now at index 1
        let id_at_1 = mgr.get_filters()[1].id;
        mgr.move_filter_down(id_at_1).await;

        // Swaps [1] and [2]: ["second", "third", "first"]
        let filters = mgr.get_filters();
        assert_eq!(filters[0].pattern, "second");
        assert_eq!(filters[1].pattern, "third");
        assert_eq!(filters[2].pattern, "first");
    }

    #[tokio::test]
    async fn test_build_filter_manager_include() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "ERROR".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

        let (fm, styles, _, _) = mgr.build_filter_manager();
        assert_eq!(styles.len(), 1);
        assert!(fm.is_visible(b"ERROR: something bad"));
        assert!(!fm.is_visible(b"INFO: all good"));
    }

    #[tokio::test]
    async fn test_build_filter_manager_exclude() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "DEBUG".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;

        let (fm, _styles, _, _) = mgr.build_filter_manager();
        assert!(fm.is_visible(b"INFO: something"));
        assert!(!fm.is_visible(b"DEBUG: verbose"));
    }

    #[tokio::test]
    async fn test_build_filter_manager_ignore_case_matches_different_case() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "ERROR".into(),
            FilterType::Include,
            FilterOptions::default().ignore_case(),
        )
        .await;

        let (fm, _, _, _) = mgr.build_filter_manager();
        assert!(fm.is_visible(b"error: lowercase"));
        assert!(fm.is_visible(b"ERROR: uppercase"));
        assert!(!fm.is_visible(b"INFO: unrelated"));
    }

    /// Regression guard for the `combined_ac`/`slow_path_filter_indices`
    /// split: a case-sensitive literal filter (fast path) and a
    /// case-insensitive literal filter (slow path) must both still match
    /// correctly when compiled together.
    #[tokio::test]
    async fn test_build_filter_manager_mixed_case_sensitivity() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "FATAL".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color(
            "WARN".into(),
            FilterType::Include,
            FilterOptions::default().ignore_case(),
        )
        .await;

        let (fm, _, _, _) = mgr.build_filter_manager();
        assert!(fm.is_visible(b"FATAL: crash"), "case-sensitive filter");
        assert!(!fm.is_visible(b"fatal: crash"), "wrong case must not match");
        assert!(fm.is_visible(b"warn: careful"), "case-insensitive filter");
        assert!(fm.is_visible(b"WARN: careful"), "case-insensitive filter");
        assert!(!fm.is_visible(b"INFO: unrelated"));
    }

    #[tokio::test]
    async fn test_build_filter_manager_disabled_filter_ignored() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "ERROR".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        let id = mgr.get_filters()[0].id;
        mgr.toggle_filter(id).await; // disable it

        let (fm, _, _, _) = mgr.build_filter_manager();
        // No enabled include filters → everything visible
        assert!(fm.is_visible(b"INFO: all good"));
        assert!(fm.is_visible(b"ERROR: bad"));
    }

    #[tokio::test]
    async fn test_build_filter_manager_uses_group_style_fallback() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "ERROR".into(),
            FilterType::Include,
            FilterOptions::default().group("errs"),
        )
        .await;
        mgr.set_group_style("errs", Some("Red"), Some("Black"), true)
            .await;

        let (_fm, styles, _, _) = mgr.build_filter_manager();
        assert_eq!(styles.len(), 1);
        assert_eq!(styles[0].fg, Some(ratatui::style::Color::Red));
        assert_eq!(styles[0].bg, Some(ratatui::style::Color::Black));
    }

    #[tokio::test]
    async fn test_build_filter_manager_filter_own_color_overrides_group_style() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "ERROR".into(),
            FilterType::Include,
            FilterOptions::default().group("errs").fg("Green"),
        )
        .await;
        mgr.set_group_style("errs", Some("Red"), Some("Black"), true)
            .await;

        let (_fm, styles, _, _) = mgr.build_filter_manager();
        assert_eq!(styles.len(), 1);
        assert_eq!(styles[0].fg, Some(ratatui::style::Color::Green));
        assert_eq!(styles[0].bg, None);
    }

    #[tokio::test]
    async fn test_build_filter_manager_field_filter_uses_group_style_fallback() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            crate::filters::encode_field_filter(
                &[("level".to_string(), "error".to_string())],
                None,
            ),
            FilterType::Include,
            FilterOptions::default().group("errs"),
        )
        .await;
        mgr.set_group_style("errs", Some("Red"), None, true).await;

        let (_fm, styles, _, field_styles) = mgr.build_filter_manager();
        assert_eq!(field_styles.len(), 1);
        assert_eq!(
            styles[field_styles[0].style_id as usize].fg,
            Some(ratatui::style::Color::Red)
        );
    }

    #[tokio::test]
    async fn test_build_filter_manager_date_filter_uses_group_style_fallback() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            format!("{DATE_PREFIX}> 2024-01-01"),
            FilterType::Include,
            FilterOptions::default().group("errs"),
        )
        .await;
        mgr.set_group_style("errs", Some("Red"), None, true).await;

        let (_fm, styles, date_styles, _) = mgr.build_filter_manager();
        assert_eq!(date_styles.len(), 1);
        assert_eq!(
            styles[date_styles[0].style_id as usize].fg,
            Some(ratatui::style::Color::Red)
        );
    }

    #[tokio::test]
    async fn test_group_names_includes_filterless_predefined_groups() {
        let mut mgr = make_manager().await;
        mgr.set_group_style("newgroup", Some("Cyan"), None, true)
            .await;
        assert_eq!(mgr.group_names(), vec!["newgroup".to_string()]);
    }

    #[tokio::test]
    async fn test_save_and_load_filters() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color(
            "debug".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        mgr.save_filters(path).unwrap();

        let mut mgr2 = make_manager().await;
        mgr2.load_filters(path, false).await.unwrap();

        let filters = mgr2.get_filters();
        assert_eq!(filters.len(), 2);
        // save_filters preserves in-memory order (oldest first): ["error", "debug"]
        // replace_all_filters assigns display_order 0, 1 to that slice → same order on reload
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[1].pattern, "debug");
    }

    #[tokio::test]
    async fn test_save_and_load_filters_round_trips_group_style() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default().group("errs"),
        )
        .await;
        mgr.set_group_style("errs", Some("Red"), None, true).await;
        // A predefined group with no filters at all should also round-trip.
        mgr.set_group_style("filterless", Some("Blue"), None, true)
            .await;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        mgr.save_filters(path).unwrap();

        let mut mgr2 = make_manager().await;
        mgr2.load_filters(path, false).await.unwrap();

        let groups = mgr2.get_group_styles();
        assert_eq!(groups.len(), 2);
        let errs = groups.iter().find(|g| g.name == "errs").unwrap();
        assert_eq!(
            errs.color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Red)
        );
        let filterless = groups.iter().find(|g| g.name == "filterless").unwrap();
        assert_eq!(
            filterless.color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Blue)
        );
    }

    #[tokio::test]
    async fn test_load_filters_does_not_wipe_another_tabs_groups() {
        // Regression test for the reported bug: importing filters/groups
        // into one tab must not clobber a different tab's (source file's)
        // groups, since `load_filters` replaces the *current* source's rows
        // only, not the whole `groups` table.
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut other = LogManager::new(db.clone(), Some("other.log".into())).await;
        other
            .set_group_style("kept", Some("Green"), None, true)
            .await;

        let mut source_mgr = make_manager().await;
        source_mgr
            .set_group_style("errs", Some("Red"), None, true)
            .await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        source_mgr.save_filters(path).unwrap();

        let mut target = LogManager::new(db, Some("target.log".into())).await;
        target.load_filters(path, false).await.unwrap();

        assert_eq!(target.get_group_styles().len(), 1);
        assert_eq!(target.get_group_styles()[0].name, "errs");
        assert_eq!(
            other.get_group_styles().len(),
            1,
            "loading filters into `target` must not touch `other`'s groups"
        );
        assert_eq!(other.get_group_styles()[0].name, "kept");
    }

    #[tokio::test]
    async fn test_load_filters_accepts_old_bare_array_format() {
        let mut mgr = make_manager().await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        // Pre-groups on-disk format: a bare JSON array of FilterDef, no
        // top-level `{ filters, groups }` wrapper.
        std::fs::write(
            path,
            r#"[{"id":1,"pattern":"ERROR","filter_type":"Include","enabled":true,"color_config":null,"use_regex":false,"ignore_case":false,"group":null}]"#,
        )
        .unwrap();

        mgr.load_filters(path, false).await.unwrap();

        let filters = mgr.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].pattern, "ERROR");
        assert!(mgr.get_group_styles().is_empty());
    }

    #[tokio::test]
    async fn test_load_filters_append_merges_into_existing_filters_and_groups() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("old".into(), FilterType::Include, FilterOptions::default())
            .await;
        mgr.set_group_style("kept", Some("Green"), None, true).await;

        let mut other = make_manager().await;
        other
            .add_filter_with_color(
                "error".into(),
                FilterType::Include,
                FilterOptions::default().group("errs"),
            )
            .await;
        other.set_group_style("errs", Some("Red"), None, true).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        other.save_filters(path).unwrap();

        mgr.load_filters(path, true).await.unwrap();

        let filters = mgr.get_filters();
        assert_eq!(
            filters.len(),
            2,
            "append must not remove the existing filter"
        );
        assert!(filters.iter().any(|f| f.pattern == "old"));
        assert!(filters.iter().any(|f| f.pattern == "error"));

        let groups = mgr.get_group_styles();
        assert_eq!(groups.len(), 2, "append must not remove the existing group");
        let kept = groups.iter().find(|g| g.name == "kept").unwrap();
        assert_eq!(
            kept.color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Green)
        );
        let errs = groups.iter().find(|g| g.name == "errs").unwrap();
        assert_eq!(
            errs.color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Red)
        );
    }

    const NPP_ANALYSEDOC_XML: &str = r##"
        <AnalyseDoc>
            <SearchText bgColor="green" group="errs">error</SearchText>
        </AnalyseDoc>
    "##;

    #[tokio::test]
    async fn test_import_npp_filters_replaces_existing_filters_by_default() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("old".into(), FilterType::Include, FilterOptions::default())
            .await;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        std::fs::write(path, NPP_ANALYSEDOC_XML).unwrap();

        mgr.import_npp_filters(path, false).await.unwrap();

        let filters = mgr.get_filters();
        // "error" only — the pre-existing "old" filter is gone.
        assert_eq!(filters.len(), 1);
        assert!(filters.iter().all(|f| f.pattern != "old"));
        assert!(filters.iter().any(|f| f.pattern == "error"));
    }

    #[tokio::test]
    async fn test_import_npp_filters_append_merges_into_existing_filters() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("old".into(), FilterType::Include, FilterOptions::default())
            .await;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        std::fs::write(path, NPP_ANALYSEDOC_XML).unwrap();

        mgr.import_npp_filters(path, true).await.unwrap();

        let filters = mgr.get_filters();
        // "old" + "error" — nothing removed.
        assert_eq!(filters.len(), 2);
        assert!(filters.iter().any(|f| f.pattern == "old"));
        assert!(filters.iter().any(|f| f.pattern == "error"));
        let error_filter = filters.iter().find(|f| f.pattern == "error").unwrap();
        assert_eq!(error_filter.group.as_deref(), Some("errs"));
        assert_eq!(
            error_filter.color_config.as_ref().unwrap().bg,
            Some(ratatui::style::Color::Green)
        );
    }

    #[tokio::test]
    async fn test_clear_filters() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.clear_filters().await;
        assert!(mgr.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_disable_all_filters() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color("warn".into(), FilterType::Include, FilterOptions::default())
            .await;
        assert!(mgr.get_filters().iter().all(|f| f.enabled));

        mgr.disable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| !f.enabled));
    }

    #[tokio::test]
    async fn test_disable_all_filters_already_disabled_is_noop() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        let id = mgr.get_filters()[0].id;
        mgr.toggle_filter(id).await; // disable it first
        assert!(!mgr.get_filters()[0].enabled);

        mgr.disable_all_filters().await; // should keep it disabled
        assert!(!mgr.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn test_enable_all_filters() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color("warn".into(), FilterType::Include, FilterOptions::default())
            .await;
        mgr.disable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| !f.enabled));

        mgr.enable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| f.enabled));
    }

    #[tokio::test]
    async fn test_enable_all_filters_already_enabled_is_noop() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        assert!(mgr.get_filters()[0].enabled);

        mgr.enable_all_filters().await; // should keep it enabled
        assert!(mgr.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn test_add_filter_with_color_stores_group() {
        let mut mgr = make_manager().await;
        let opts = FilterOptions::default().group("errors");
        mgr.add_filter_with_color("error".into(), FilterType::Include, opts)
            .await;
        assert_eq!(mgr.get_filters()[0].group.as_deref(), Some("errors"));
    }

    #[tokio::test]
    async fn test_group_names_sorted_and_deduped() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "a".into(),
            FilterType::Include,
            FilterOptions::default().group("zeta"),
        )
        .await;
        mgr.add_filter_with_color(
            "b".into(),
            FilterType::Include,
            FilterOptions::default().group("alpha"),
        )
        .await;
        mgr.add_filter_with_color(
            "c".into(),
            FilterType::Include,
            FilterOptions::default().group("alpha"),
        )
        .await;
        mgr.add_filter_with_color("d".into(), FilterType::Include, FilterOptions::default())
            .await;

        assert_eq!(
            mgr.group_names(),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }

    #[tokio::test]
    async fn test_set_filters_enabled_by_group() {
        let mut mgr = make_manager().await;
        let opts = FilterOptions::default().group("errors");
        mgr.add_filter_with_color("error".into(), FilterType::Include, opts.clone())
            .await;
        mgr.add_filter_with_color("warn".into(), FilterType::Include, opts)
            .await;
        mgr.add_filter_with_color(
            "debug".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

        mgr.set_filters_enabled_by_group("errors", false).await;

        let filters = mgr.get_filters();
        let by_pattern = |p: &str| filters.iter().find(|f| f.pattern == p).unwrap();
        assert!(!by_pattern("error").enabled);
        assert!(!by_pattern("warn").enabled);
        assert!(by_pattern("debug").enabled);
    }

    #[tokio::test]
    async fn test_remove_group_deletes_its_filters_and_style() {
        let mut mgr = make_manager().await;
        let opts = FilterOptions::default().group("errors");
        mgr.add_filter_with_color("error".into(), FilterType::Include, opts.clone())
            .await;
        mgr.add_filter_with_color("warn".into(), FilterType::Include, opts)
            .await;
        mgr.add_filter_with_color(
            "debug".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.set_group_style("errors", Some("red"), None, true).await;

        mgr.remove_group("errors").await;

        let patterns: Vec<_> = mgr
            .get_filters()
            .iter()
            .map(|f| f.pattern.clone())
            .collect();
        assert_eq!(patterns, vec!["debug".to_string()]);
        assert!(!mgr.group_names().contains(&"errors".to_string()));
        assert!(crate::filters::group_style(mgr.get_group_styles(), "errors").is_none());
    }

    #[tokio::test]
    async fn test_remove_group_on_unknown_group_is_noop() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default().group("errors"),
        )
        .await;

        mgr.remove_group("missing").await;

        assert_eq!(mgr.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn test_set_group_enabled_creates_bare_entry_for_unknown_group() {
        let mut mgr = make_manager().await;
        assert!(crate::filters::group_enabled(
            mgr.get_group_styles(),
            "empty"
        ));

        mgr.set_group_enabled("empty", false).await;

        assert!(!crate::filters::group_enabled(
            mgr.get_group_styles(),
            "empty"
        ));
        assert!(mgr.group_names().contains(&"empty".to_string()));
    }

    #[tokio::test]
    async fn test_set_group_enabled_preserves_existing_style() {
        let mut mgr = make_manager().await;
        mgr.set_group_style("errors", Some("red"), None, true).await;

        mgr.set_group_enabled("errors", false).await;

        assert!(!crate::filters::group_enabled(
            mgr.get_group_styles(),
            "errors"
        ));
        assert_eq!(
            crate::filters::group_style(mgr.get_group_styles(), "errors")
                .unwrap()
                .fg,
            Some(ratatui::style::Color::Red)
        );
    }

    #[tokio::test]
    async fn test_disable_then_enable_restores_state() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color(
            "debug".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;

        mgr.disable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| !f.enabled));

        mgr.enable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| f.enabled));
    }

    #[tokio::test]
    async fn test_add_duplicate_pattern_does_not_insert() {
        let mut mgr = make_manager().await;
        let was_new = mgr
            .add_filter_with_color(
                "error".into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        assert!(was_new);
        let was_new2 = mgr
            .add_filter_with_color(
                "error".into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        assert!(!was_new2);
        assert_eq!(mgr.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn test_add_duplicate_updates_color_config() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        assert!(mgr.get_filters()[0].color_config.is_none());

        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default().fg("red").line_mode(),
        )
        .await;
        assert_eq!(mgr.get_filters().len(), 1);
        let cc = mgr.get_filters()[0].color_config.as_ref().unwrap();
        assert!(cc.fg.is_some());
        assert!(!cc.match_only);
    }

    #[tokio::test]
    async fn test_add_same_pattern_different_type_inserts() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;
        assert_eq!(mgr.get_filters().len(), 2);
    }

    #[tokio::test]
    async fn test_add_field_filter_duplicate_no_insert() {
        let mut mgr = make_manager().await;
        let was_new = mgr
            .add_filter_with_color(
                "@field:level:error".into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        assert!(was_new);
        let was_new2 = mgr
            .add_filter_with_color(
                "@field:level:error".into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        assert!(!was_new2);
        assert_eq!(mgr.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn test_add_filter_with_color_highlight_gets_color_config() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "ERROR".into(),
            FilterType::Highlight,
            FilterOptions::default().fg("red"),
        )
        .await;
        assert!(mgr.get_filters()[0].color_config.is_some());
    }

    #[tokio::test]
    async fn test_build_filter_manager_highlight_does_not_set_has_include() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "ERROR".into(),
            FilterType::Highlight,
            FilterOptions::default(),
        )
        .await;

        let (fm, _, _, _) = mgr.build_filter_manager();
        assert!(!fm.has_include());
        assert!(fm.is_visible(b"ERROR: something bad"));
        assert!(fm.is_visible(b"INFO: all good"));
    }

    #[tokio::test]
    async fn test_build_filter_manager_highlight_produces_style() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "ERROR".into(),
            FilterType::Highlight,
            FilterOptions::default().fg("red"),
        )
        .await;

        let (_, styles, _, _) = mgr.build_filter_manager();
        assert_eq!(styles.len(), 1);
        assert!(styles[0].fg.is_some());
    }

    #[tokio::test]
    async fn test_build_filter_manager_field_highlight_produces_field_style() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "@field:level:error".into(),
            FilterType::Highlight,
            FilterOptions::default().fg("red"),
        )
        .await;

        let (_, _, _, field_styles) = mgr.build_filter_manager();
        assert_eq!(field_styles.len(), 1);
    }

    #[tokio::test]
    async fn test_build_filter_manager_skips_field_prefix() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "@field:level:error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

        let (fm, styles, date_styles, field_styles) = mgr.build_filter_manager();
        // The field filter must not produce a text-filter entry or a style.
        // (Field filter styles are only collected when color_config is set with fg/bg.)
        assert!(styles.is_empty(), "expected no styles for field filter");
        assert!(date_styles.is_empty());
        assert!(field_styles.is_empty());
        // With no text include filters active, the FilterManager should
        // have no has_include flag — every line is visible.
        assert!(fm.is_visible(b"INFO: something unrelated"));
        assert!(fm.is_visible(b"ERROR: bad thing"));
    }

    #[tokio::test]
    async fn test_reset_in_memory() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

        mgr.reset_in_memory();

        assert!(mgr.get_filters().is_empty());
    }
}
