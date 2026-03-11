//! Filter, mark, and annotation state management with SQLite persistence.
//!
//! [`LogManager`] owns `filter_defs`, `marks`, and `comments` in memory and
//! bridges to the database via `async fn` methods. `build_filter_manager`
//! converts enabled [`FilterDef`]s into a renderable [`FilterManager`] +
//! parallel style palette, skipping `@date:` prefixed entries.
//!
//! ## Filter CRUD
//!
//! `add_filter_with_color`, `remove_filter`, `toggle_filter`, `edit_filter`,
//! `move_filter_up/down`, `set_color_config`, `clear_filters`, `save_filters`
//! (JSON), `load_filters` (JSON).
//!
//! `build_filter_manager() -> (FilterManager, Vec<Style>)`: converts enabled
//! `FilterDef`s into a compiled `FilterManager` + parallel style palette (one
//! `Style` per enabled filter, indexed by `StyleId`). Skips `@date:` prefixed
//! patterns — date filters are applied separately in `refresh_visible()`.
//!
//! ## Marks API
//!
//! `toggle_mark`, `is_marked`, `get_marked_indices`, `get_marked_lines(&FileReader)`.
//!
//! ## Comments API
//!
//! `add_comment(text, line_indices)`, `get_comments() -> &[Comment]`,
//! `has_comment(line_idx) -> bool`, `set_comments(Vec<Comment>)`,
//! `remove_comment(index)`, `clear_all_marks_and_comments()`.
//!
//! `compute_file_hash(path)`: hashes file size + mtime for change detection.

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ratatui::style::Style;

use crate::date_filter::{DATE_PREFIX, DateFilterStyle, parse_date_filter};
use crate::db::{Database, FilterStore};
use crate::file_reader::FileReader;
use crate::filters::{FilterDecision, FilterManager, StyleId, build_filter};
use crate::types::{ColorConfig, Comment, FilterDef, FilterType, parse_color};

/// Manages filter definitions (persisted to SQLite), marks (in-memory), and
/// the mapping to a renderable `FilterManager` + style palette.
///
/// Does NOT own the `FileReader`; callers pass it when needed so that
/// `LogManager` stays independent of file-format concerns.
pub struct LogManager {
    pub(crate) db: Arc<Database>,
    source_file: Option<String>,
    filter_defs: Vec<FilterDef>,
    marks: HashSet<usize>,
    comments: Vec<Comment>,
}

impl LogManager {
    pub async fn new(db: Arc<Database>, source_file: Option<String>) -> Self {
        let mut mgr = LogManager {
            db,
            source_file,
            filter_defs: Vec::new(),
            marks: HashSet::new(),
            comments: Vec::new(),
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
        fg: Option<&str>,
        bg: Option<&str>,
        match_only: bool,
    ) -> bool {
        let color_config = if filter_type == FilterType::Include {
            let fg_color = fg.and_then(parse_color);
            let bg_color = bg.and_then(parse_color);
            if fg_color.is_some() || bg_color.is_some() || !match_only {
                Some(ColorConfig {
                    fg: fg_color,
                    bg: bg_color,
                    match_only,
                })
            } else {
                None
            }
        } else {
            None
        };

        if let Some(pos) = self
            .filter_defs
            .iter()
            .position(|f| f.pattern == pattern && f.filter_type == filter_type)
        {
            self.filter_defs[pos].color_config = color_config.clone();
            let id = self.filter_defs[pos].id;
            let _ = self
                .db
                .update_filter_color(id as i64, color_config.as_ref())
                .await;
            return false;
        }

        let pattern_clone = pattern.clone();
        let filter_type_clone = filter_type.clone();
        let cc_clone = color_config.clone();
        let source = self.source_file.clone();

        let id = self
            .db
            .insert_filter(
                &pattern_clone,
                &filter_type_clone,
                true,
                cc_clone.as_ref(),
                source.as_deref(),
            )
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
        let ids_to_disable: Vec<usize> = self
            .filter_defs
            .iter()
            .filter(|f| f.enabled)
            .map(|f| f.id)
            .collect();
        for f in self.filter_defs.iter_mut() {
            f.enabled = false;
        }
        let db = self.db.clone();
        for id in ids_to_disable {
            let _ = db.toggle_filter(id as i64).await;
        }
    }

    pub async fn enable_all_filters(&mut self) {
        let ids_to_enable: Vec<usize> = self
            .filter_defs
            .iter()
            .filter(|f| !f.enabled)
            .map(|f| f.id)
            .collect();
        for f in self.filter_defs.iter_mut() {
            f.enabled = true;
        }
        for id in ids_to_enable {
            let _ = self.db.toggle_filter(id as i64).await;
        }
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

    /// Update an existing filter's pattern, type, and color in-place, preserving its
    /// position in the filter list.
    pub async fn update_filter(
        &mut self,
        id: usize,
        pattern: String,
        filter_type: FilterType,
        fg: Option<&str>,
        bg: Option<&str>,
        match_only: bool,
    ) {
        let color_config = if filter_type == FilterType::Include {
            let fg_color = fg.and_then(parse_color);
            let bg_color = bg.and_then(parse_color);
            if fg_color.is_some() || bg_color.is_some() || !match_only {
                Some(ColorConfig {
                    fg: fg_color,
                    bg: bg_color,
                    match_only,
                })
            } else {
                None
            }
        } else {
            None
        };
        if let Some(f) = self.filter_defs.iter_mut().find(|f| f.id == id) {
            f.pattern = pattern.clone();
            f.filter_type = filter_type.clone();
            f.color_config = color_config.clone();
        }
        let _ = self
            .db
            .update_filter(id as i64, &pattern, &filter_type, color_config.as_ref())
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

    pub fn toggle_mark(&mut self, line_idx: usize) {
        if self.marks.contains(&line_idx) {
            self.marks.remove(&line_idx);
        } else {
            self.marks.insert(line_idx);
        }
    }

    pub fn is_marked(&self, line_idx: usize) -> bool {
        self.marks.contains(&line_idx)
    }

    pub fn get_marked_indices(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.marks.iter().copied().collect();
        v.sort_unstable();
        v
    }

    pub fn set_marks(&mut self, indices: Vec<usize>) {
        self.marks = indices.into_iter().collect();
    }

    /// Return the raw text of all marked lines.
    pub fn get_marked_lines<'a>(&self, reader: &'a FileReader) -> Vec<&'a [u8]> {
        let mut indices: Vec<usize> = self.marks.iter().copied().collect();
        indices.sort_unstable();
        indices
            .into_iter()
            .filter(|&i| i < reader.line_count())
            .map(|i| reader.get_line(i))
            .collect()
    }

    /// Append a new comment for the given line indices.
    pub fn add_comment(&mut self, text: String, line_indices: Vec<usize>) {
        if !line_indices.is_empty() {
            self.comments.push(Comment { text, line_indices });
        }
    }

    pub fn get_comments(&self) -> &[Comment] {
        &self.comments
    }

    /// Returns true if `line_idx` belongs to at least one comment.
    pub fn has_comment(&self, line_idx: usize) -> bool {
        self.comments
            .iter()
            .any(|a| a.line_indices.contains(&line_idx))
    }

    pub fn set_comments(&mut self, comments: Vec<Comment>) {
        self.comments = comments;
    }

    /// Remove a single comment by index.
    pub fn remove_comment(&mut self, index: usize) {
        if index < self.comments.len() {
            self.comments.remove(index);
        }
    }

    /// Clear all marks and comments at once.
    pub fn clear_all_marks_and_comments(&mut self) {
        self.marks.clear();
        self.comments.clear();
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
        Vec<crate::field_filter::FieldFilterStyle>,
    ) {
        let mut filters: Vec<Box<dyn crate::filters::Filter>> = Vec::new();
        let mut styles: Vec<Style> = Vec::new();
        let mut date_filter_styles: Vec<DateFilterStyle> = Vec::new();
        let mut field_filter_styles: Vec<crate::field_filter::FieldFilterStyle> = Vec::new();
        let mut has_include = false;

        let mut style_idx: usize = 0;
        for def in self.filter_defs.iter().filter(|f| f.enabled) {
            // Field filters: applied separately for visibility; collect styles for highlighting.
            if def.pattern.starts_with(crate::field_filter::FIELD_PREFIX) {
                if let Some(cc) = &def.color_config
                    && (cc.fg.is_some() || cc.bg.is_some())
                    && let Ok((field, pattern)) = crate::field_filter::parse_field_filter(
                        &def.pattern[crate::field_filter::FIELD_PREFIX.len()..],
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
                    field_filter_styles.push(crate::field_filter::FieldFilterStyle {
                        field_filter: crate::field_filter::FieldFilter {
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

            if let Some(f) = build_filter(&def.pattern, decision, match_only, style_id) {
                filters.push(f);
            }
        }

        // Reserve the last slot for search highlights (StyleId = styles.len()).
        // The caller appends the search style.

        (
            FilterManager::new(filters, has_include),
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
            .add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;

        // A placeholder tab (no source) must not expose those filters.
        let mgr = LogManager::new(db, None).await;
        assert!(mgr.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_add_and_get_filters() {
        let mut mgr = make_manager().await;
        assert!(mgr.get_filters().is_empty());

        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("debug".into(), FilterType::Exclude, None, None, true)
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
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
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
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("debug".into(), FilterType::Exclude, None, None, true)
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
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        let id = mgr.get_filters()[0].id;

        mgr.edit_filter(id, "critical".into()).await;
        assert_eq!(mgr.get_filters()[0].pattern, "critical");
    }

    #[tokio::test]
    async fn test_move_filter_up_down() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("first".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("second".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("third".into(), FilterType::Include, None, None, true)
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
    async fn test_marks() {
        let mut mgr = make_manager().await;
        assert!(!mgr.is_marked(0));
        assert!(!mgr.is_marked(5));

        mgr.toggle_mark(0);
        mgr.toggle_mark(5);
        assert!(mgr.is_marked(0));
        assert!(mgr.is_marked(5));

        mgr.toggle_mark(0);
        assert!(!mgr.is_marked(0));
        assert!(mgr.is_marked(5));

        let indices = mgr.get_marked_indices();
        assert_eq!(indices, vec![5]);
    }

    #[tokio::test]
    async fn test_set_marks() {
        let mut mgr = make_manager().await;
        mgr.set_marks(vec![1, 3, 7]);
        assert!(mgr.is_marked(1));
        assert!(mgr.is_marked(3));
        assert!(mgr.is_marked(7));
        assert!(!mgr.is_marked(0));
    }

    #[tokio::test]
    async fn test_build_filter_manager_include() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("ERROR".into(), FilterType::Include, None, None, true)
            .await;

        let (fm, styles, _, _) = mgr.build_filter_manager();
        assert_eq!(styles.len(), 1);
        assert!(fm.is_visible(b"ERROR: something bad"));
        assert!(!fm.is_visible(b"INFO: all good"));
    }

    #[tokio::test]
    async fn test_build_filter_manager_exclude() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("DEBUG".into(), FilterType::Exclude, None, None, true)
            .await;

        let (fm, _styles, _, _) = mgr.build_filter_manager();
        assert!(fm.is_visible(b"INFO: something"));
        assert!(!fm.is_visible(b"DEBUG: verbose"));
    }

    #[tokio::test]
    async fn test_build_filter_manager_disabled_filter_ignored() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("ERROR".into(), FilterType::Include, None, None, true)
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
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("debug".into(), FilterType::Exclude, None, None, true)
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
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        mgr.clear_filters().await;
        assert!(mgr.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_disable_all_filters() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("warn".into(), FilterType::Include, None, None, true)
            .await;
        assert!(mgr.get_filters().iter().all(|f| f.enabled));

        mgr.disable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| !f.enabled));
    }

    #[tokio::test]
    async fn test_disable_all_filters_already_disabled_is_noop() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
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
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("warn".into(), FilterType::Include, None, None, true)
            .await;
        mgr.disable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| !f.enabled));

        mgr.enable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| f.enabled));
    }

    #[tokio::test]
    async fn test_enable_all_filters_already_enabled_is_noop() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        assert!(mgr.get_filters()[0].enabled);

        mgr.enable_all_filters().await; // should keep it enabled
        assert!(mgr.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn test_disable_then_enable_restores_state() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("debug".into(), FilterType::Exclude, None, None, true)
            .await;

        mgr.disable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| !f.enabled));

        mgr.enable_all_filters().await;
        assert!(mgr.get_filters().iter().all(|f| f.enabled));
    }

    #[tokio::test]
    async fn test_remove_comment() {
        let mut mgr = make_manager().await;
        mgr.add_comment("first".into(), vec![0]);
        mgr.add_comment("second".into(), vec![1]);
        assert_eq!(mgr.get_comments().len(), 2);

        mgr.remove_comment(0);
        assert_eq!(mgr.get_comments().len(), 1);
        assert_eq!(mgr.get_comments()[0].text, "second");
    }

    #[tokio::test]
    async fn test_remove_comment_out_of_bounds() {
        let mut mgr = make_manager().await;
        mgr.add_comment("only".into(), vec![0]);
        mgr.remove_comment(5); // out of bounds, should be a no-op
        assert_eq!(mgr.get_comments().len(), 1);
    }

    #[tokio::test]
    async fn test_clear_all_marks_and_comments() {
        let mut mgr = make_manager().await;
        mgr.toggle_mark(0);
        mgr.toggle_mark(3);
        mgr.add_comment("note".into(), vec![1]);
        mgr.add_comment("another".into(), vec![2]);
        assert!(!mgr.get_marked_indices().is_empty());
        assert!(!mgr.get_comments().is_empty());

        mgr.clear_all_marks_and_comments();
        assert!(mgr.get_marked_indices().is_empty());
        assert!(mgr.get_comments().is_empty());
    }

    #[tokio::test]
    async fn test_get_marked_lines() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "line zero").unwrap();
        writeln!(f, "line one").unwrap();
        writeln!(f, "line two").unwrap();
        let reader = FileReader::new(f.path().to_str().unwrap()).unwrap();

        let mut mgr = make_manager().await;
        mgr.toggle_mark(0);
        mgr.toggle_mark(2);

        let lines = mgr.get_marked_lines(&reader);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], b"line zero");
        assert_eq!(lines[1], b"line two");
    }

    #[tokio::test]
    async fn test_add_duplicate_pattern_does_not_insert() {
        let mut mgr = make_manager().await;
        let was_new = mgr
            .add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        assert!(was_new);
        let was_new2 = mgr
            .add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        assert!(!was_new2);
        assert_eq!(mgr.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn test_add_duplicate_updates_color_config() {
        let mut mgr = make_manager().await;
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        assert!(mgr.get_filters()[0].color_config.is_none());

        mgr.add_filter_with_color(
            "error".into(),
            FilterType::Include,
            Some("red"),
            None,
            false,
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
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, true)
            .await;
        mgr.add_filter_with_color("error".into(), FilterType::Exclude, None, None, true)
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
                None,
                None,
                true,
            )
            .await;
        assert!(was_new);
        let was_new2 = mgr
            .add_filter_with_color(
                "@field:level:error".into(),
                FilterType::Include,
                None,
                None,
                true,
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
            None,
            None,
            true,
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
}
