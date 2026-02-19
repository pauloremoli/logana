use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ratatui::style::{Color, Style};

use crate::db::{Database, FilterStore};
use crate::file_reader::FileReader;
use crate::filters::{FilterDecision, FilterManager, StyleId, build_filter};
use crate::types::{ColorConfig, FilterDef, FilterType};

/// Manages filter definitions (persisted to SQLite), marks (in-memory), and
/// the mapping to a renderable `FilterManager` + style palette.
///
/// Does NOT own the `FileReader`; callers pass it when needed so that
/// `LogManager` stays independent of file-format concerns.
pub struct LogManager {
    pub(crate) db: Arc<Database>,
    pub(crate) rt: Arc<tokio::runtime::Runtime>,
    source_file: Option<String>,
    filter_defs: Vec<FilterDef>,
    marks: HashSet<usize>,
}

impl LogManager {
    pub fn new(
        db: Arc<Database>,
        rt: Arc<tokio::runtime::Runtime>,
        source_file: Option<String>,
    ) -> Self {
        let mut mgr = LogManager {
            db,
            rt,
            source_file,
            filter_defs: Vec::new(),
            marks: HashSet::new(),
        };
        mgr.reload_filters_from_db();
        mgr
    }

    // ── Source file ──────────────────────────────────────────────────────────

    pub fn source_file(&self) -> Option<&str> {
        self.source_file.as_deref()
    }

    pub fn set_source_file(&mut self, source: Option<String>) {
        self.source_file = source;
        self.reload_filters_from_db();
    }

    // ── Filter management ────────────────────────────────────────────────────

    pub fn get_filters(&self) -> &[FilterDef] {
        &self.filter_defs
    }

    pub fn add_filter_with_color(
        &mut self,
        pattern: String,
        filter_type: FilterType,
        fg: Option<&str>,
        bg: Option<&str>,
        match_only: bool,
    ) {
        let color_config = if filter_type == FilterType::Include {
            let fg_color = fg.and_then(|s| s.parse::<Color>().ok());
            let bg_color = bg.and_then(|s| s.parse::<Color>().ok());
            if fg_color.is_some() || bg_color.is_some() || match_only {
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

        let db = self.db.clone();
        let pattern_clone = pattern.clone();
        let filter_type_clone = filter_type.clone();
        let cc_clone = color_config.clone();
        let source = self.source_file.clone();

        let id = self
            .rt
            .block_on(db.insert_filter(
                &pattern_clone,
                &filter_type_clone,
                true,
                cc_clone.as_ref(),
                source.as_deref(),
            ))
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
    }

    pub fn toggle_filter(&mut self, id: usize) {
        if let Some(f) = self.filter_defs.iter_mut().find(|f| f.id == id) {
            f.enabled = !f.enabled;
        }
        let db = self.db.clone();
        self.rt.spawn(async move {
            let _ = db.toggle_filter(id as i64).await;
        });
    }

    pub fn remove_filter(&mut self, id: usize) {
        self.filter_defs.retain(|f| f.id != id);
        let db = self.db.clone();
        self.rt.spawn(async move {
            let _ = db.delete_filter(id as i64).await;
        });
    }

    pub fn clear_filters(&mut self) {
        self.filter_defs.clear();
        let db = self.db.clone();
        let source = self.source_file.clone();
        self.rt.spawn(async move {
            if let Some(src) = source {
                let _ = db.clear_filters_for_source(&src).await;
            } else {
                let _ = db.clear_filters().await;
            }
        });
    }

    pub fn edit_filter(&mut self, id: usize, new_pattern: String) {
        if let Some(f) = self.filter_defs.iter_mut().find(|f| f.id == id) {
            f.pattern = new_pattern.clone();
        }
        let db = self.db.clone();
        self.rt.spawn(async move {
            let _ = db.update_filter_pattern(id as i64, &new_pattern).await;
        });
    }

    pub fn move_filter_up(&mut self, id: usize) {
        if let Some(idx) = self.filter_defs.iter().position(|f| f.id == id)
            && idx > 0
        {
            self.filter_defs.swap(idx, idx - 1);
            let other_id = self.filter_defs[idx].id;
            let db = self.db.clone();
            self.rt.spawn(async move {
                let _ = db.swap_filter_order(id as i64, other_id as i64).await;
            });
        }
    }

    pub fn move_filter_down(&mut self, id: usize) {
        if let Some(idx) = self.filter_defs.iter().position(|f| f.id == id)
            && idx + 1 < self.filter_defs.len()
        {
            self.filter_defs.swap(idx, idx + 1);
            let other_id = self.filter_defs[idx].id;
            let db = self.db.clone();
            self.rt.spawn(async move {
                let _ = db.swap_filter_order(id as i64, other_id as i64).await;
            });
        }
    }

    pub fn set_color_config(
        &mut self,
        filter_id: usize,
        fg: Option<&str>,
        bg: Option<&str>,
        match_only: bool,
    ) {
        let fg_color = fg.and_then(|s| s.parse::<Color>().ok());
        let bg_color = bg.and_then(|s| s.parse::<Color>().ok());
        if fg_color.is_none() && bg_color.is_none() && !match_only {
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
        let db = self.db.clone();
        self.rt.spawn(async move {
            let _ = db.update_filter_color(filter_id as i64, Some(&cc)).await;
        });
    }

    pub fn save_filters(&self, path: &str) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(&self.filter_defs)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_filters(&mut self, path: &str) -> anyhow::Result<()> {
        let json = std::fs::read_to_string(path)?;
        let filters: Vec<FilterDef> = serde_json::from_str(&json)?;
        let source = self.source_file.as_deref();
        self.rt
            .block_on(self.db.replace_all_filters(&filters, source))?;
        self.reload_filters_from_db();
        Ok(())
    }

    // ── Marks ────────────────────────────────────────────────────────────────

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

    // ── Filter-manager construction ──────────────────────────────────────────

    /// Build a `FilterManager` and its associated `Vec<Style>` from the current
    /// enabled filter definitions.
    ///
    /// `StyleId` is the index into the returned `Vec<Style>`.
    pub fn build_filter_manager(&self) -> (FilterManager, Vec<Style>) {
        let mut filters: Vec<Box<dyn crate::filters::Filter>> = Vec::new();
        let mut styles: Vec<Style> = Vec::new();
        let mut has_include = false;

        for (style_idx, def) in self
            .filter_defs
            .iter()
            .filter(|f| f.enabled)
            .enumerate()
        {
            let style_id = style_idx as StyleId;

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
                .unwrap_or(false);

            if let Some(f) = build_filter(&def.pattern, decision, match_only, style_id) {
                filters.push(f);
            }
        }

        // Reserve the last slot for search highlights (StyleId = styles.len()).
        // The caller appends the search style.

        (FilterManager::new(filters, has_include), styles)
    }

    // ── File hash ────────────────────────────────────────────────────────────

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

    // ── Internal helpers ─────────────────────────────────────────────────────

    fn reload_filters_from_db(&mut self) {
        let db = self.db.clone();
        let source = self.source_file.clone();
        self.filter_defs = self
            .rt
            .block_on(async move {
                if let Some(src) = source {
                    db.get_filters_for_source(&src).await
                } else {
                    db.get_filters().await
                }
            })
            .unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager() -> LogManager {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = Arc::new(rt.block_on(Database::in_memory()).unwrap());
        LogManager::new(db, rt, None)
    }

    #[test]
    fn test_add_and_get_filters() {
        let mut mgr = make_manager();
        assert!(mgr.get_filters().is_empty());

        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, false);
        mgr.add_filter_with_color("debug".into(), FilterType::Exclude, None, None, false);

        let filters = mgr.get_filters();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[1].pattern, "debug");
        assert_eq!(filters[1].filter_type, FilterType::Exclude);
    }

    #[test]
    fn test_toggle_filter() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, false);
        let id = mgr.get_filters()[0].id;

        assert!(mgr.get_filters()[0].enabled);
        mgr.toggle_filter(id);
        assert!(!mgr.get_filters()[0].enabled);
        mgr.toggle_filter(id);
        assert!(mgr.get_filters()[0].enabled);
    }

    #[test]
    fn test_remove_filter() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, false);
        mgr.add_filter_with_color("debug".into(), FilterType::Exclude, None, None, false);
        let id = mgr.get_filters()[0].id;

        mgr.remove_filter(id);
        assert_eq!(mgr.get_filters().len(), 1);
        assert_eq!(mgr.get_filters()[0].pattern, "debug");
    }

    #[test]
    fn test_edit_filter() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, false);
        let id = mgr.get_filters()[0].id;

        mgr.edit_filter(id, "critical".into());
        assert_eq!(mgr.get_filters()[0].pattern, "critical");
    }

    #[test]
    fn test_move_filter_up_down() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("first".into(), FilterType::Include, None, None, false);
        mgr.add_filter_with_color("second".into(), FilterType::Include, None, None, false);
        mgr.add_filter_with_color("third".into(), FilterType::Include, None, None, false);

        let id_second = mgr.get_filters()[1].id;
        mgr.move_filter_up(id_second);

        let filters = mgr.get_filters();
        assert_eq!(filters[0].pattern, "second");
        assert_eq!(filters[1].pattern, "first");

        let id_first = mgr.get_filters()[1].id;
        mgr.move_filter_down(id_first);

        let filters = mgr.get_filters();
        assert_eq!(filters[0].pattern, "second");
        assert_eq!(filters[1].pattern, "third");
        assert_eq!(filters[2].pattern, "first");
    }

    #[test]
    fn test_marks() {
        let mut mgr = make_manager();
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

    #[test]
    fn test_set_marks() {
        let mut mgr = make_manager();
        mgr.set_marks(vec![1, 3, 7]);
        assert!(mgr.is_marked(1));
        assert!(mgr.is_marked(3));
        assert!(mgr.is_marked(7));
        assert!(!mgr.is_marked(0));
    }

    #[test]
    fn test_build_filter_manager_include() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("ERROR".into(), FilterType::Include, None, None, false);

        let (fm, styles) = mgr.build_filter_manager();
        assert_eq!(styles.len(), 1);
        assert!(fm.is_visible(b"ERROR: something bad"));
        assert!(!fm.is_visible(b"INFO: all good"));
    }

    #[test]
    fn test_build_filter_manager_exclude() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("DEBUG".into(), FilterType::Exclude, None, None, false);

        let (fm, _styles) = mgr.build_filter_manager();
        assert!(fm.is_visible(b"INFO: something"));
        assert!(!fm.is_visible(b"DEBUG: verbose"));
    }

    #[test]
    fn test_build_filter_manager_disabled_filter_ignored() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("ERROR".into(), FilterType::Include, None, None, false);
        let id = mgr.get_filters()[0].id;
        mgr.toggle_filter(id); // disable it

        let (fm, _) = mgr.build_filter_manager();
        // No enabled include filters → everything visible
        assert!(fm.is_visible(b"INFO: all good"));
        assert!(fm.is_visible(b"ERROR: bad"));
    }

    #[test]
    fn test_save_and_load_filters() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, false);
        mgr.add_filter_with_color("debug".into(), FilterType::Exclude, None, None, false);

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        mgr.save_filters(path).unwrap();

        let mut mgr2 = make_manager();
        mgr2.load_filters(path).unwrap();

        let filters = mgr2.get_filters();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[1].pattern, "debug");
    }

    #[test]
    fn test_clear_filters() {
        let mut mgr = make_manager();
        mgr.add_filter_with_color("error".into(), FilterType::Include, None, None, false);
        mgr.clear_filters();
        assert!(mgr.get_filters().is_empty());
    }

    #[test]
    fn test_get_marked_lines() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "line zero").unwrap();
        writeln!(f, "line one").unwrap();
        writeln!(f, "line two").unwrap();
        let reader = FileReader::new(f.path().to_str().unwrap()).unwrap();

        let mut mgr = make_manager();
        mgr.toggle_mark(0);
        mgr.toggle_mark(2);

        let lines = mgr.get_marked_lines(&reader);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], b"line zero");
        assert_eq!(lines[1], b"line two");
    }
}
