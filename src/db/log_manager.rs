use crate::db::{Database, FilterStore};
use crate::filters::{ColorConfig, FilterDef, FilterInsertOptions, FilterOptions, FilterType};
use crate::filters::{DATE_PREFIX, DateFilterStyle, parse_date_filter};
use crate::filters::{FilterDecision, FilterManager, StyleId, build_filter};
use crate::theme::parse_color;
use aho_corasick::AhoCorasick;
use ratatui::style::Style;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub struct LogManager {
    pub db: Arc<Database>,
    source_file: Option<String>,
    filter_defs: Vec<FilterDef>,
}

impl LogManager {
    pub async fn new(db: Arc<Database>, source_file: Option<String>) -> Self {
        let mut mgr = LogManager {
            db,
            source_file,
            filter_defs: Vec::new(),
        };
        mgr.reload_filters_from_db().await;
        mgr
    }

    pub fn source_file(&self) -> Option<&str> {
        self.source_file.as_deref()
    }

    pub async fn set_source_file(&mut self, source: Option<String>) {
        self.source_file = source;
        self.reload_filters_from_db().await;
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
        let color_config = (filter_type == FilterType::Include)
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
        let color_config = (filter_type == FilterType::Include)
            .then(|| ColorConfig::try_from(&options).ok())
            .flatten();
        if let Some(f) = self.filter_defs.iter_mut().find(|f| f.id == id) {
            f.pattern = pattern.clone();
            f.filter_type = filter_type.clone();
            f.color_config = color_config.clone();
            f.use_regex = options.use_regex;
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
                options.group.as_deref(),
            )
            .await;
    }

    /// Distinct group names among the current filters, sorted alphabetically.
    pub fn group_names(&self) -> Vec<String> {
        crate::filters::known_groups(&self.filter_defs)
    }

    /// Set `enabled` on every filter belonging to `group`.
    pub async fn set_filters_enabled_by_group(&mut self, group: &str, enabled: bool) {
        for f in self.filter_defs.iter_mut() {
            if f.group.as_deref() == Some(group) {
                f.enabled = enabled;
            }
        }
        let _ = self.db.set_filters_enabled_by_group(group, enabled).await;
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
        let json = serde_json::to_string_pretty(&self.filter_defs)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub async fn load_filters(&mut self, path: &str) -> anyhow::Result<()> {
        let json = std::fs::read_to_string(path)?;
        let filters: Vec<FilterDef> = serde_json::from_str(&json)?;
        let source = self.source_file.clone();
        self.db
            .replace_all_filters(&filters, source.as_deref())
            .await?;
        self.filter_defs = if let Some(src) = source.as_deref() {
            self.db.get_filters_for_source(src).await
        } else {
            self.db.get_filters().await
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
        let mut regex_filter_indices: Vec<usize> = Vec::new();

        let mut style_idx: usize = 0;
        for def in self.filter_defs.iter().filter(|f| f.enabled) {
            // Field filters: applied separately for visibility; collect styles for highlighting.
            if def.pattern.starts_with(crate::filters::FIELD_PREFIX) {
                if let Some(cc) = &def.color_config
                    && (cc.fg.is_some() || cc.bg.is_some())
                    && let Ok((field, pattern)) = crate::filters::parse_field_filter(
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
                    let decision = if def.filter_type == FilterType::Include {
                        FilterDecision::Include
                    } else {
                        FilterDecision::Exclude
                    };
                    field_filter_styles.push(crate::filters::FieldFilterStyle {
                        field_filter: crate::filters::FieldFilter {
                            field,
                            pattern,
                            decision,
                        },
                        style_id,
                        match_only: cc.match_only,
                    });
                }
                continue;
            }

            if def.pattern.starts_with(DATE_PREFIX) {
                if let Some(cc) = &def.color_config
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

            let style = def
                .color_config
                .as_ref()
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

            let decision = if def.filter_type == FilterType::Include {
                has_include = true;
                FilterDecision::Include
            } else {
                FilterDecision::Exclude
            };

            let match_only = def
                .color_config
                .as_ref()
                .map(|cc| cc.match_only)
                .unwrap_or(true);

            if let Some(f) =
                build_filter(&def.pattern, decision, match_only, style_id, def.use_regex)
            {
                let filter_idx = filters.len();
                if def.use_regex {
                    regex_filter_indices.push(filter_idx);
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
                regex_filter_indices,
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
        mgr2.load_filters(path).await.unwrap();

        let filters = mgr2.get_filters();
        assert_eq!(filters.len(), 2);
        // save_filters preserves in-memory order (oldest first): ["error", "debug"]
        // replace_all_filters assigns display_order 0, 1 to that slice → same order on reload
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[1].pattern, "debug");
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
