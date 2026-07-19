use crate::{
    config::Keybindings,
    ingestion::{ArchiveTree, CheckState, NodeId, NodeKind},
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    ui::{KeyResult, TabState},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashSet;

/// What kind of row this is, for the widget to pick a glyph without needing
/// to know anything about `NodeKind`/`ArchiveTree`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    File,
    /// `collapsed` mirrors `ArchiveNode::collapsed` — whether this
    /// container's children are currently folded out of the row list.
    Container {
        collapsed: bool,
    },
    /// A nested archive not yet read — see `NodeKind::LazyContainer`.
    Lazy,
    Error,
}

/// A single rendered row of the archive picker popup: enough to draw one
/// line (name, indentation, checkbox state) without the widget needing to
/// know anything about the underlying tree structure.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveRow {
    pub name: String,
    pub depth: usize,
    pub kind: RowKind,
    pub check_state: CheckState,
    /// Independent of `check_state` — whether this row is marked to be
    /// merged into one timestamp-sorted tab rather than opened on its own.
    pub merge_check_state: CheckState,
}

/// Precompiled form of the search query — built once when the query text
/// changes rather than once per node checked. `visible_rows()` is called
/// several times per keystroke (scroll clamping, then `render_state()`)
/// and an archive can have thousands of entries, so recompiling a `Regex`
/// per node per call (as a plain `regex_search_match` per node would) made
/// every keystroke — and every idle redraw tick — visibly slow to type
/// once a search was active on a large archive. Mirrors
/// `regex_search_match`'s own fallback: a pattern that fails to compile as
/// a regex still works as a plain case-insensitive substring match.
#[derive(Debug)]
enum SearchMatcher {
    MatchAll,
    Regex(regex::Regex),
    Literal(String),
}

impl SearchMatcher {
    fn compile(query: &str) -> Self {
        if query.is_empty() {
            return Self::MatchAll;
        }
        match regex::RegexBuilder::new(query)
            .case_insensitive(true)
            .build()
        {
            Ok(re) => Self::Regex(re),
            Err(_) => Self::Literal(query.to_lowercase()),
        }
    }

    fn is_match(&self, haystack: &str) -> bool {
        match self {
            Self::MatchAll => true,
            Self::Regex(re) => re.is_match(haystack),
            Self::Literal(needle) => haystack.to_lowercase().contains(needle.as_str()),
        }
    }
}

#[derive(Debug)]
pub struct ArchivePickerMode {
    pub tree: ArchiveTree,
    /// Full preorder row list. Recomputing it on every keystroke would be
    /// wasted work when nothing changed, so it's cached here and refreshed
    /// via [`Self::refresh_all_ids`] only after something that actually
    /// changes it: an expand or collapse.
    all_ids: Vec<NodeId>,
    pub selected: usize,
    pub source_path: String,
    /// Live typeahead query; non-empty narrows [`Self::visible_rows`] to
    /// matching files and the containers that hold them.
    pub search: String,
    /// Kept in sync with `search` at every mutation site — see
    /// [`SearchMatcher`].
    search_matcher: SearchMatcher,
    /// True while capturing search input — gates every other bound key
    /// (toggle/all/none) into the query buffer instead, since the picker's
    /// action keys ('a', 'n', ' ') would otherwise collide with likely
    /// search text (e.g. a filename starting with "a").
    pub searching: bool,
    /// Full-list index to restore if search is cancelled.
    pre_search_selected: Option<usize>,
    /// Pending count prefix (e.g. the `4` in `4j`), mirrors
    /// `FilterManagementMode.count`/`NormalMode.count`.
    pub count: Option<usize>,
}

impl ArchivePickerMode {
    pub fn new(tree: ArchiveTree, source_path: String) -> Self {
        let all_ids = tree.visible_rows();
        Self {
            tree,
            all_ids,
            selected: 0,
            source_path,
            search: String::new(),
            search_matcher: SearchMatcher::MatchAll,
            searching: false,
            pre_search_selected: None,
            count: None,
        }
    }

    /// Rows currently shown: every row when `search` is empty, otherwise
    /// only rows that match the query regex plus any ancestor containers
    /// needed to keep a matching file's location visible.
    pub fn visible_rows(&self) -> Vec<NodeId> {
        if self.search.is_empty() {
            return self.all_ids.clone();
        }
        let mut keep: HashSet<NodeId> = HashSet::new();
        for &id in &self.all_ids {
            if self.search_matcher.is_match(&self.tree.nodes[id].name) {
                let mut cur = Some(id);
                while let Some(nid) = cur {
                    keep.insert(nid);
                    cur = self.tree.nodes[nid].parent;
                }
            }
        }
        self.all_ids
            .iter()
            .copied()
            .filter(|id| keep.contains(id))
            .collect()
    }

    fn clamp_selected(&mut self) {
        let count = self.visible_rows().len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    fn scroll_up(&mut self, count: usize) {
        self.selected = self.selected.saturating_sub(count);
    }

    fn scroll_down(&mut self, count: usize) {
        let num_rows = self.visible_rows().len();
        if num_rows > 0 {
            self.selected = (self.selected + count).min(num_rows - 1);
        }
    }

    /// Jumps to a specific row: `Some(n)` (a `1`-based count prefix, as in
    /// `5G`/`5gg`) clamped to the last row, or `None` for the natural
    /// "no count given" target (`fallback`, `G`'s last row or `gg`'s first).
    fn goto(&mut self, count: Option<usize>, fallback: usize) {
        let num_rows = self.visible_rows().len();
        self.selected = match count {
            Some(n) => (n.saturating_sub(1)).min(num_rows.saturating_sub(1)),
            None => fallback,
        };
    }

    /// `states`/`merge_states` are the whole tree's precomputed check
    /// states (see [`ArchiveTree::check_states`]/[`ArchiveTree::merge_check_states`])
    /// — passing them in rather than calling `container_check_state(id)`
    /// here keeps a full row list build at `O(n)` instead of re-walking
    /// each container's descendants per row.
    fn row_for(
        &self,
        id: NodeId,
        states: &[CheckState],
        merge_states: &[CheckState],
    ) -> ArchiveRow {
        let node = &self.tree.nodes[id];
        match &node.kind {
            NodeKind::File => ArchiveRow {
                name: node.name.clone(),
                depth: node.depth,
                kind: RowKind::File,
                check_state: states[id],
                merge_check_state: merge_states[id],
            },
            NodeKind::Container { .. } => ArchiveRow {
                name: node.name.clone(),
                depth: node.depth,
                kind: RowKind::Container {
                    collapsed: node.collapsed,
                },
                check_state: states[id],
                merge_check_state: merge_states[id],
            },
            NodeKind::LazyContainer { .. } => ArchiveRow {
                name: node.name.clone(),
                depth: node.depth,
                kind: RowKind::Lazy,
                check_state: CheckState::Unchecked,
                merge_check_state: CheckState::Unchecked,
            },
            NodeKind::UnreadableContainer { error } => ArchiveRow {
                name: format!("{} ({error})", node.name),
                depth: node.depth,
                kind: RowKind::Error,
                check_state: CheckState::Unchecked,
                merge_check_state: CheckState::Unchecked,
            },
        }
    }

    fn set_all_files_selected(&mut self, selected: bool) {
        for node in &mut self.tree.nodes {
            if matches!(node.kind, NodeKind::File) {
                node.selected = selected;
            }
        }
    }

    /// Recomputes `all_ids` from the tree and re-clamps `selected` — call
    /// after any tree mutation that can change which rows exist (expanding
    /// a lazy node, collapsing/uncollapsing a container). `pub(crate)` so
    /// `App::poll_archive_expand` (a different module) can call it after
    /// applying a background expand to the live tree.
    pub(crate) fn refresh_all_ids(&mut self) {
        self.all_ids = self.tree.visible_rows();
        self.clamp_selected();
    }
}

impl ArchivePickerMode {
    /// Handles input while [`Self::searching`] is set — a deliberately
    /// minimal key set (confirm/cancel/backspace/chars/single-step j/k),
    /// gating every other bound action out until search is confirmed or
    /// cancelled.
    fn handle_search_key(
        mut self: Box<Self>,
        kb: &Keybindings,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if kb.search.confirm.matches(key, modifiers) {
            let full_idx = self
                .visible_rows()
                .get(self.selected)
                .and_then(|&id| self.all_ids.iter().position(|&aid| aid == id))
                .unwrap_or(0);
            self.selected = full_idx;
            self.search.clear();
            self.search_matcher = SearchMatcher::MatchAll;
            self.searching = false;
            self.pre_search_selected = None;
            return (self, KeyResult::Handled);
        }
        if kb.search.cancel.matches(key, modifiers) {
            self.selected = self.pre_search_selected.take().unwrap_or(0);
            self.search.clear();
            self.search_matcher = SearchMatcher::MatchAll;
            self.searching = false;
            return (self, KeyResult::Handled);
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            let count = self.visible_rows().len();
            if count > 0 {
                self.selected = (self.selected + 1).min(count - 1);
            }
            return (self, KeyResult::Handled);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
            return (self, KeyResult::Handled);
        }
        match key {
            KeyCode::Backspace => {
                self.search.pop();
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(c);
            }
            _ => return (self, KeyResult::Ignored),
        }
        self.search_matcher = SearchMatcher::compile(&self.search);
        self.selected = 0;
        self.clamp_selected();
        (self, KeyResult::Handled)
    }
}

#[async_trait]
impl Mode for ArchivePickerMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.interaction.keybindings;

        if self.searching {
            return self.handle_search_key(kb, key, modifiers);
        }

        if let KeyCode::Char(c @ '1'..='9') = key
            && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT)
        {
            let digit = (c as u32 - '0' as u32) as usize;
            let n = self
                .count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit);
            self.count = Some(n.min(999_999));
            return (self, KeyResult::Handled);
        }
        if let KeyCode::Char('0') = key
            && self.count.is_some()
            && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT)
        {
            self.count = Some(self.count.unwrap().saturating_mul(10).min(999_999));
            return (self, KeyResult::Handled);
        }

        if kb.archive_picker.apply.matches(key, modifiers) {
            self.count = None;
            tab.interaction.g_key_pressed = false;
            if !self.tree.any_file_selected() && !self.tree.any_file_merge_marked() {
                tab.interaction.command_error =
                    Some("Select or merge-mark at least 1 file".to_string());
                return (self, KeyResult::Handled);
            }
            return (
                Box::new(NormalMode::default()),
                KeyResult::ApplyArchivePicker {
                    source_path: self.source_path.clone(),
                    tree: self.tree.clone(),
                },
            );
        }

        if kb.archive_picker.cancel.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        if kb.archive_picker.search.matches(key, modifiers) {
            self.pre_search_selected = Some(self.selected);
            self.search.clear();
            self.search_matcher = SearchMatcher::MatchAll;
            self.searching = true;
            self.count = None;
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.scroll_up.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.interaction.g_key_pressed = false;
            self.scroll_up(count);
            return (self, KeyResult::Handled);
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.interaction.g_key_pressed = false;
            self.scroll_down(count);
            return (self, KeyResult::Handled);
        }
        if kb.navigation.half_page_up.matches(key, modifiers) {
            let half = (tab.interaction.archive_picker_visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.interaction.g_key_pressed = false;
            self.scroll_up(half.saturating_mul(count));
            return (self, KeyResult::Handled);
        }
        if kb.navigation.half_page_down.matches(key, modifiers) {
            let half = (tab.interaction.archive_picker_visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.interaction.g_key_pressed = false;
            self.scroll_down(half.saturating_mul(count));
            return (self, KeyResult::Handled);
        }
        if kb.navigation.page_up.matches(key, modifiers) {
            let page = tab.interaction.archive_picker_visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.interaction.g_key_pressed = false;
            self.scroll_up(page.saturating_mul(count));
            return (self, KeyResult::Handled);
        }
        if kb.navigation.page_down.matches(key, modifiers) {
            let page = tab.interaction.archive_picker_visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.interaction.g_key_pressed = false;
            self.scroll_down(page.saturating_mul(count));
            return (self, KeyResult::Handled);
        }
        if kb.normal.go_to_bottom.matches(key, modifiers) {
            let num_rows = self.visible_rows().len();
            let count = self.count.take();
            tab.interaction.g_key_pressed = false;
            self.goto(count, num_rows.saturating_sub(1));
            return (self, KeyResult::Handled);
        }
        if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.interaction.g_key_pressed {
                let count = self.count.take();
                tab.interaction.g_key_pressed = false;
                self.goto(count, 0);
            } else {
                tab.interaction.g_key_pressed = true;
            }
            return (self, KeyResult::Handled);
        }

        self.count = None;
        tab.interaction.g_key_pressed = false;

        if kb.archive_picker.toggle.matches(key, modifiers) {
            if let Some(&id) = self.visible_rows().get(self.selected) {
                self.tree.toggle_subtree(id);
            }
            return (self, KeyResult::Handled);
        }
        if kb.archive_picker.merge_toggle.matches(key, modifiers) {
            if let Some(&id) = self.visible_rows().get(self.selected) {
                self.tree.toggle_merge_subtree(id);
            }
            return (self, KeyResult::Handled);
        }
        if kb.archive_picker.expand.matches(key, modifiers) {
            if let Some(&id) = self.visible_rows().get(self.selected) {
                let is_lazy = matches!(self.tree.nodes[id].kind, NodeKind::LazyContainer { .. });
                let is_collapsed_container =
                    matches!(self.tree.nodes[id].kind, NodeKind::Container { .. })
                        && self.tree.nodes[id].collapsed;
                if is_lazy {
                    return (self, KeyResult::ExpandArchiveNode { node_id: id });
                }
                if is_collapsed_container {
                    self.tree.set_collapsed(id, false);
                    self.refresh_all_ids();
                }
            }
            return (self, KeyResult::Handled);
        }
        if kb.archive_picker.collapse.matches(key, modifiers) {
            if let Some(&id) = self.visible_rows().get(self.selected) {
                let is_expanded_container =
                    matches!(self.tree.nodes[id].kind, NodeKind::Container { .. })
                        && !self.tree.nodes[id].collapsed;
                if is_expanded_container {
                    self.tree.set_collapsed(id, true);
                    self.refresh_all_ids();
                }
            }
            return (self, KeyResult::Handled);
        }
        if kb.archive_picker.all.matches(key, modifiers) {
            self.set_all_files_selected(true);
            return (self, KeyResult::Handled);
        }
        if kb.archive_picker.none.matches(key, modifiers) {
            self.set_all_files_selected(false);
            return (self, KeyResult::Handled);
        }

        (self, KeyResult::Ignored)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[ARCHIVE]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(
            &mut spans,
            kb.archive_picker.toggle.display(),
            "toggle",
            theme,
        );
        status_entry(
            &mut spans,
            kb.archive_picker.merge_toggle.display(),
            "merge-mark",
            theme,
        );
        status_entry(
            &mut spans,
            kb.archive_picker.expand.display(),
            "expand",
            theme,
        );
        status_entry(
            &mut spans,
            kb.archive_picker.collapse.display(),
            "collapse",
            theme,
        );
        status_entry(
            &mut spans,
            kb.archive_picker.apply.display(),
            "extract",
            theme,
        );
        status_entry(
            &mut spans,
            kb.archive_picker.cancel.display(),
            "cancel",
            theme,
        );
        status_entry(&mut spans, kb.archive_picker.all.display(), "all", theme);
        status_entry(&mut spans, kb.archive_picker.none.display(), "none", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        let states = self.tree.check_states();
        let merge_states = self.tree.merge_check_states();
        ModeRenderState::ArchivePicker {
            rows: self
                .visible_rows()
                .iter()
                .map(|&id| self.row_for(id, &states, &merge_states))
                .collect(),
            selected: self.selected,
            source_path: self.source_path.clone(),
            search: self.search.clone(),
            searching: self.searching,
        }
    }

    fn as_archive_picker_mut(&mut self) -> Option<&mut ArchivePickerMode> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager};
    use crate::ingestion::{ArchiveNode, FileReader};
    use crate::mode::app_mode::ModeRenderState;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let reader = FileReader::from_bytes(b"line1\nline2\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        TabState::new(reader, lm, "test".to_string())
    }

    fn file_node(id: NodeId, parent: Option<NodeId>, name: &str, depth: usize) -> ArchiveNode {
        ArchiveNode {
            id,
            parent,
            name: name.to_string(),
            full_path: name.to_string(),
            depth,
            kind: NodeKind::File,
            selected: false,
            merge_marked: false,
            cached_bytes: None,
            collapsed: false,
            disk_path: false,
        }
    }

    fn container_node(
        id: NodeId,
        parent: Option<NodeId>,
        name: &str,
        depth: usize,
        children: Vec<NodeId>,
    ) -> ArchiveNode {
        ArchiveNode {
            id,
            parent,
            name: name.to_string(),
            full_path: name.to_string(),
            depth,
            kind: NodeKind::Container {
                children,
                archive_type: Some(crate::ingestion::ArchiveType::Zip),
            },
            selected: false,
            merge_marked: false,
            cached_bytes: None,
            collapsed: false,
            disk_path: false,
        }
    }

    fn lazy_node(id: NodeId, parent: Option<NodeId>, name: &str, depth: usize) -> ArchiveNode {
        ArchiveNode {
            id,
            parent,
            name: name.to_string(),
            full_path: name.to_string(),
            depth,
            kind: NodeKind::LazyContainer {
                archive_type: crate::ingestion::ArchiveType::Zip,
            },
            selected: false,
            merge_marked: false,
            cached_bytes: None,
            collapsed: false,
            disk_path: false,
        }
    }

    /// Builds:
    ///   0: File "a.log"                (root)
    ///   1: Container "bundle.zip"      (root) -> [2, 3]
    ///     2: File "inner1.log"
    ///     3: File "inner2.log"
    ///   4: LazyContainer "lazy.zip"    (root)
    fn build_tree_with_lazy_node() -> ArchiveTree {
        let nodes = vec![
            file_node(0, None, "a.log", 0),
            container_node(1, None, "bundle.zip", 0, vec![2, 3]),
            file_node(2, Some(1), "inner1.log", 1),
            file_node(3, Some(1), "inner2.log", 1),
            lazy_node(4, None, "lazy.zip", 0),
        ];
        ArchiveTree {
            nodes,
            roots: vec![0, 1, 4],
        }
    }

    fn mode_with_lazy_node() -> ArchivePickerMode {
        ArchivePickerMode::new(build_tree_with_lazy_node(), "archive.zip".to_string())
    }

    /// Builds:
    ///   0: File "a.log"                (root)
    ///   1: Container "bundle.zip"      (root) -> [2, 3]
    ///     2: File "inner1.log"
    ///     3: File "inner2.log"
    fn build_test_tree() -> ArchiveTree {
        let nodes = vec![
            file_node(0, None, "a.log", 0),
            container_node(1, None, "bundle.zip", 0, vec![2, 3]),
            file_node(2, Some(1), "inner1.log", 1),
            file_node(3, Some(1), "inner2.log", 1),
        ];
        ArchiveTree {
            nodes,
            roots: vec![0, 1],
        }
    }

    fn mode() -> ArchivePickerMode {
        ArchivePickerMode::new(build_test_tree(), "archive.zip".to_string())
    }

    /// A flat tree of `n` top-level files — large enough to exercise
    /// count-prefixed motions, half-page/page scrolling, and `gg`/`G`
    /// meaningfully (unlike `build_test_tree`'s 4 rows).
    fn build_flat_tree(n: usize) -> ArchiveTree {
        let nodes: Vec<ArchiveNode> = (0..n)
            .map(|i| file_node(i, None, &format!("file{i}.log"), 0))
            .collect();
        let roots = (0..n).collect();
        ArchiveTree { nodes, roots }
    }

    fn mode_with_n_files(n: usize) -> ArchivePickerMode {
        ArchivePickerMode::new(build_flat_tree(n), "archive.zip".to_string())
    }

    async fn press(
        mode: ArchivePickerMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    fn extract_state(state: ModeRenderState) -> (Vec<ArchiveRow>, usize, String) {
        match state {
            ModeRenderState::ArchivePicker {
                rows,
                selected,
                source_path,
                ..
            } => (rows, selected, source_path),
            other => panic!("expected ArchivePicker, got {:?}", other),
        }
    }

    fn extract_search(state: ModeRenderState) -> String {
        match state {
            ModeRenderState::ArchivePicker { search, .. } => search,
            other => panic!("expected ArchivePicker, got {:?}", other),
        }
    }

    fn extract_searching(state: ModeRenderState) -> bool {
        match state {
            ModeRenderState::ArchivePicker { searching, .. } => searching,
            other => panic!("expected ArchivePicker, got {:?}", other),
        }
    }

    #[test]
    fn test_new_flattens_visible_rows_in_preorder() {
        let m = mode();
        assert_eq!(m.visible_rows(), vec![0, 1, 2, 3]);
        assert_eq!(m.selected, 0);
    }

    #[test]
    fn test_render_state_returns_archive_picker() {
        let m = mode();
        let (rows, selected, source_path) = extract_state(m.render_state());
        assert_eq!(rows.len(), 4);
        assert_eq!(selected, 0);
        assert_eq!(source_path, "archive.zip");
        assert_eq!(rows[0].name, "a.log");
        assert_eq!(rows[0].kind, RowKind::File);
        assert_eq!(rows[1].name, "bundle.zip");
        assert_eq!(rows[1].kind, RowKind::Container { collapsed: false });
        assert_eq!(rows[2].depth, 1);
    }

    #[tokio::test]
    async fn test_scroll_down_moves_cursor() {
        let mut tab = make_tab().await;
        let (mode2, result) = press(mode(), &mut tab, KeyCode::Char('j')).await;
        assert!(
            matches!(result, KeyResult::Handled),
            "a recognized navigation key must not also fall through to the global key handler"
        );
        let (_, selected, _) = extract_state(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_scroll_down_clamped_at_last() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 3;
        let (mode2, _) = press(m, &mut tab, KeyCode::Char('j')).await;
        let (_, selected, _) = extract_state(mode2.render_state());
        assert_eq!(selected, 3);
    }

    #[tokio::test]
    async fn test_scroll_up_clamped_at_zero() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(mode(), &mut tab, KeyCode::Char('k')).await;
        let (_, selected, _) = extract_state(mode2.render_state());
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_count_prefixed_j_moves_down_by_count() {
        let mut tab = make_tab().await;
        let (m, result) = press_keys(
            mode_with_n_files(10),
            &mut tab,
            &[KeyCode::Char('4'), KeyCode::Char('j')],
        )
        .await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 4);
    }

    #[tokio::test]
    async fn test_count_prefixed_k_moves_up_by_count_clamped() {
        let mut tab = make_tab().await;
        let mut m = mode_with_n_files(10);
        m.selected = 5;
        let (m, _) = press_keys(m, &mut tab, &[KeyCode::Char('9'), KeyCode::Char('k')]).await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_digit_accumulation_then_j_moves_down_by_typed_count() {
        let mut tab = make_tab().await;
        // "12j" — two digit keypresses accumulate into count 12.
        let (m, _) = press_keys(
            mode_with_n_files(20),
            &mut tab,
            &[KeyCode::Char('1'), KeyCode::Char('2'), KeyCode::Char('j')],
        )
        .await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 12);
    }

    #[tokio::test]
    async fn test_count_resets_after_being_consumed() {
        let mut tab = make_tab().await;
        let (m, _) = press_keys(
            mode_with_n_files(20),
            &mut tab,
            &[KeyCode::Char('4'), KeyCode::Char('j')],
        )
        .await;
        // Second 'j' with no new count should move by 1, not reuse the old count of 4.
        let (m, _) = m
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 5);
    }

    #[tokio::test]
    async fn test_count_resets_on_unrecognized_key() {
        let mut tab = make_tab().await;
        let (m, _) = press_keys(
            mode_with_n_files(20),
            &mut tab,
            &[KeyCode::Char('4'), KeyCode::F(5)],
        )
        .await;
        let (m, _) = m
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(
            selected, 1,
            "stale count from before an unrecognized key must not apply"
        );
    }

    #[tokio::test]
    async fn test_half_page_down_moves_by_half_visible_height() {
        let mut tab = make_tab().await;
        tab.interaction.archive_picker_visible_height = 10;
        let (m, result) = Box::new(mode_with_n_files(30))
            .handle_key(&mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL)
            .await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 5);
    }

    #[tokio::test]
    async fn test_half_page_up_moves_by_half_visible_height() {
        let mut tab = make_tab().await;
        tab.interaction.archive_picker_visible_height = 10;
        let mut m = mode_with_n_files(30);
        m.selected = 20;
        let (m2, _) = Box::new(m)
            .handle_key(&mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL)
            .await;
        let (_, selected, _) = extract_state(m2.render_state());
        assert_eq!(selected, 15);
    }

    #[tokio::test]
    async fn test_page_down_moves_by_full_visible_height() {
        let mut tab = make_tab().await;
        tab.interaction.archive_picker_visible_height = 10;
        let (m, _) = press(mode_with_n_files(30), &mut tab, KeyCode::PageDown).await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 10);
    }

    #[tokio::test]
    async fn test_page_up_moves_by_full_visible_height() {
        let mut tab = make_tab().await;
        tab.interaction.archive_picker_visible_height = 10;
        let mut m = mode_with_n_files(30);
        m.selected = 25;
        let (m2, _) = press(m, &mut tab, KeyCode::PageUp).await;
        let (_, selected, _) = extract_state(m2.render_state());
        assert_eq!(selected, 15);
    }

    #[tokio::test]
    async fn test_capital_g_goes_to_last_row() {
        let mut tab = make_tab().await;
        let (m, result) = press(mode_with_n_files(30), &mut tab, KeyCode::Char('G')).await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 29);
    }

    #[tokio::test]
    async fn test_capital_g_with_count_goes_to_specific_row() {
        let mut tab = make_tab().await;
        let (m, _) = press_keys(
            mode_with_n_files(30),
            &mut tab,
            &[KeyCode::Char('5'), KeyCode::Char('G')],
        )
        .await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 4);
    }

    #[tokio::test]
    async fn test_gg_chord_goes_to_first_row() {
        let mut tab = make_tab().await;
        let mut m = mode_with_n_files(30);
        m.selected = 20;
        let (m, _) = press_keys(m, &mut tab, &[KeyCode::Char('g'), KeyCode::Char('g')]).await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_gg_chord_with_count_goes_to_specific_row() {
        let mut tab = make_tab().await;
        let (m, _) = press_keys(
            mode_with_n_files(30),
            &mut tab,
            &[KeyCode::Char('5'), KeyCode::Char('g'), KeyCode::Char('g')],
        )
        .await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 4);
    }

    #[tokio::test]
    async fn test_non_g_key_clears_g_key_pressed() {
        let mut tab = make_tab().await;
        let mut m = mode_with_n_files(30);
        m.selected = 20;
        // First 'g' starts the chord, 'j' interrupts it, second 'g' must
        // not complete a stale chord.
        let (m, _) = press_keys(
            m,
            &mut tab,
            &[KeyCode::Char('g'), KeyCode::Char('j'), KeyCode::Char('g')],
        )
        .await;
        let (_, selected, _) = extract_state(m.render_state());
        assert_eq!(
            selected, 21,
            "interrupted gg chord must not jump to the top"
        );
    }

    #[tokio::test]
    async fn test_toggle_file_row_selects_only_itself() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(mode(), &mut tab, KeyCode::Char(' ')).await;
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows[0].check_state, CheckState::Checked);
        assert_eq!(rows[1].check_state, CheckState::Unchecked);
    }

    #[tokio::test]
    async fn test_toggle_container_row_selects_all_descendants() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 1; // "bundle.zip"
        let (mode2, _) = press(m, &mut tab, KeyCode::Char(' ')).await;
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows[1].check_state, CheckState::Checked);
        assert_eq!(rows[2].check_state, CheckState::Checked);
        assert_eq!(rows[3].check_state, CheckState::Checked);
    }

    #[tokio::test]
    async fn test_merge_toggle_key_marks_file_row() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(mode(), &mut tab, KeyCode::Char('m')).await;
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows[0].merge_check_state, CheckState::Checked);
        assert_eq!(rows[1].merge_check_state, CheckState::Unchecked);
        // Independent from the extraction checkbox.
        assert_eq!(rows[0].check_state, CheckState::Unchecked);
    }

    #[tokio::test]
    async fn test_merge_toggle_on_container_marks_all_descendants() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 1; // "bundle.zip"
        let (mode2, _) = press(m, &mut tab, KeyCode::Char('m')).await;
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows[1].merge_check_state, CheckState::Checked);
        assert_eq!(rows[2].merge_check_state, CheckState::Checked);
        assert_eq!(rows[3].merge_check_state, CheckState::Checked);
        assert!(
            rows.iter().all(|r| r.check_state == CheckState::Unchecked),
            "merge-marking must not affect the extraction checkbox"
        );
    }

    #[tokio::test]
    async fn test_expand_key_on_lazy_row_returns_expand_archive_node_and_keeps_mode_alive() {
        let mut tab = make_tab().await;
        let mut m = mode_with_lazy_node();
        m.selected = 4; // "lazy.zip"
        let (mode2, result) = press(m, &mut tab, KeyCode::Right).await;
        match result {
            KeyResult::ExpandArchiveNode { node_id } => assert_eq!(node_id, 4),
            other => panic!("expected ExpandArchiveNode, got {:?}", other),
        }
        // The mode itself is unchanged/still alive (not replaced with
        // NormalMode) — the fetch happens in the background.
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows[4].kind, RowKind::Lazy);
    }

    #[tokio::test]
    async fn test_expand_key_on_collapsed_container_reveals_children_synchronously() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.tree.set_collapsed(1, true);
        m.selected = 1; // "bundle.zip", now collapsed
        let (mode2, result) = press(m, &mut tab, KeyCode::Right).await;
        assert!(matches!(result, KeyResult::Handled));
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(
            rows.len(),
            4,
            "children must be visible again, no fetch needed"
        );
        assert_eq!(rows[1].kind, RowKind::Container { collapsed: false });
    }

    #[tokio::test]
    async fn test_expand_key_on_already_expanded_container_is_a_no_op() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 1; // "bundle.zip", already expanded
        let (mode2, result) = press(m, &mut tab, KeyCode::Right).await;
        assert!(matches!(result, KeyResult::Handled));
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows.len(), 4);
    }

    #[tokio::test]
    async fn test_expand_key_on_file_row_is_a_no_op() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 0; // "a.log"
        let (_, result) = press(m, &mut tab, KeyCode::Right).await;
        assert!(matches!(result, KeyResult::Handled));
    }

    #[tokio::test]
    async fn test_collapse_key_on_expanded_container_hides_children() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 1; // "bundle.zip"
        let (mode2, result) = press(m, &mut tab, KeyCode::Left).await;
        assert!(matches!(result, KeyResult::Handled));
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows.len(), 2, "inner1.log/inner2.log must be hidden");
        assert_eq!(rows[1].kind, RowKind::Container { collapsed: true });
    }

    #[tokio::test]
    async fn test_collapse_key_on_already_collapsed_container_is_a_no_op() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.tree.set_collapsed(1, true);
        m.refresh_all_ids();
        m.selected = 1;
        let (mode2, result) = press(m, &mut tab, KeyCode::Left).await;
        assert!(matches!(result, KeyResult::Handled));
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn test_collapse_key_on_file_row_is_a_no_op() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 0; // "a.log"
        let (mode2, result) = press(m, &mut tab, KeyCode::Left).await;
        assert!(matches!(result, KeyResult::Handled));
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows.len(), 4);
    }

    #[tokio::test]
    async fn test_mode_bar_shows_expand_and_collapse_entries() {
        let m = mode();
        let kb = Keybindings::default();
        let theme = crate::theme::Theme::default();
        let line = m.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("expand"));
        assert!(text.contains("collapse"));
    }

    #[tokio::test]
    async fn test_toggle_descendant_does_not_affect_sibling_row() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 2; // "inner1.log"
        let (mode2, _) = press(m, &mut tab, KeyCode::Char(' ')).await;
        let (rows, _, _) = extract_state(mode2.render_state());
        assert_eq!(rows[2].check_state, CheckState::Checked);
        assert_eq!(rows[3].check_state, CheckState::Unchecked);
        assert_eq!(rows[1].check_state, CheckState::Partial);
    }

    #[tokio::test]
    async fn test_select_all_selects_every_file_not_just_visible_containers() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(mode(), &mut tab, KeyCode::Char('a')).await;
        let (rows, _, _) = extract_state(mode2.render_state());
        assert!(rows.iter().all(|r| r.check_state == CheckState::Checked));
    }

    #[tokio::test]
    async fn test_select_none_deselects_every_file() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.tree.toggle_subtree(1);
        let (mode2, _) = press(m, &mut tab, KeyCode::Char('n')).await;
        let (rows, _, _) = extract_state(mode2.render_state());
        assert!(rows.iter().all(|r| r.check_state == CheckState::Unchecked));
    }

    #[tokio::test]
    async fn test_apply_with_zero_selected_shows_error_and_stays_in_mode() {
        let mut tab = make_tab().await;
        let (_, result) = press(mode(), &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(tab.interaction.command_error.is_some());
    }

    #[tokio::test]
    async fn test_apply_with_selection_returns_apply_archive_picker_key_result() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.tree.nodes[0].selected = true;
        let (_, result) = press(m, &mut tab, KeyCode::Enter).await;
        match result {
            KeyResult::ApplyArchivePicker { source_path, tree } => {
                assert_eq!(source_path, "archive.zip");
                assert!(tree.nodes[0].selected);
            }
            other => panic!("expected ApplyArchivePicker, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_apply_with_only_merge_marked_no_selected_succeeds() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.tree.nodes[0].merge_marked = true;
        let (_, result) = press(m, &mut tab, KeyCode::Enter).await;
        match result {
            KeyResult::ApplyArchivePicker { tree, .. } => {
                assert!(tree.nodes[0].merge_marked);
                assert!(!tree.nodes[0].selected);
            }
            other => panic!("expected ApplyArchivePicker, got {:?}", other),
        }
        assert!(tab.interaction.command_error.is_none());
    }

    #[tokio::test]
    async fn test_cancel_returns_to_normal_mode() {
        let mut tab = make_tab().await;
        let (_, result) = press(mode(), &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
    }

    #[tokio::test]
    async fn test_unknown_key_returns_ignored() {
        let mut tab = make_tab().await;
        let (_, result) = press(mode(), &mut tab, KeyCode::F(5)).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_mode_bar_content_contains_archive_label() {
        let m = mode();
        let kb = Keybindings::default();
        let theme = crate::theme::Theme::default();
        let line = m.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("ARCHIVE"));
    }

    #[test]
    fn test_mode_bar_content_contains_merge_mark_entry() {
        let m = mode();
        let kb = Keybindings::default();
        let theme = crate::theme::Theme::default();
        let line = m.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("merge-mark"));
    }

    #[test]
    fn test_row_for_unreadable_container_marks_is_error() {
        let mut tree = build_test_tree();
        tree.nodes[1].kind = NodeKind::UnreadableContainer {
            error: "bad zip".to_string(),
        };
        let m = ArchivePickerMode::new(tree, "archive.zip".to_string());
        let (rows, _, _) = extract_state(m.render_state());
        assert_eq!(rows[1].kind, RowKind::Error);
        assert!(rows[1].name.contains("bad zip"));
    }

    /// Presses `/` to enter search, then types `text` one character at a
    /// time via `handle_key`, threading the opaque `Box<dyn Mode>` through
    /// each keystroke the same way real input dispatch does.
    async fn press_keys(
        m: ArchivePickerMode,
        tab: &mut TabState,
        keys: &[KeyCode],
    ) -> (Box<dyn Mode>, KeyResult) {
        let (mut m, mut result) = press(m, tab, keys[0]).await;
        for &k in &keys[1..] {
            let (m2, r) = m.handle_key(tab, k, KeyModifiers::NONE).await;
            m = m2;
            result = r;
        }
        (m, result)
    }

    async fn enter_search_and_type(
        m: ArchivePickerMode,
        tab: &mut TabState,
        text: &str,
    ) -> (Box<dyn Mode>, KeyResult) {
        let (mut m, mut result) = press(m, tab, KeyCode::Char('/')).await;
        for c in text.chars() {
            let (m2, r) = m
                .handle_key(tab, KeyCode::Char(c), KeyModifiers::NONE)
                .await;
            m = m2;
            result = r;
        }
        (m, result)
    }

    #[test]
    fn test_search_starts_empty_and_not_searching() {
        let m = mode();
        assert_eq!(extract_search(m.render_state()), "");
        assert!(!extract_searching(m.render_state()));
    }

    #[tokio::test]
    async fn test_slash_enters_search_mode_with_empty_query() {
        let mut tab = make_tab().await;
        let (m, result) = press(mode(), &mut tab, KeyCode::Char('/')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(extract_searching(m.render_state()));
        assert_eq!(extract_search(m.render_state()), "");
    }

    #[tokio::test]
    async fn test_typing_while_searching_appends_to_query() {
        let mut tab = make_tab().await;
        let (m, _) = enter_search_and_type(mode(), &mut tab, "in").await;
        assert_eq!(extract_search(m.render_state()), "in");
        assert!(extract_searching(m.render_state()));
    }

    #[tokio::test]
    async fn test_search_narrows_to_matching_files_and_keeps_ancestor_container() {
        let mut tab = make_tab().await;
        let (m, _) = enter_search_and_type(mode(), &mut tab, "inner1").await;
        let (rows, _, _) = extract_state(m.render_state());
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        // "a.log" doesn't match and has no matching descendant, so it's hidden;
        // "bundle.zip" doesn't match itself but is kept because its child does;
        // "inner2.log" doesn't match, so it's hidden even though its sibling does.
        assert_eq!(names, vec!["bundle.zip", "inner1.log"]);
    }

    #[tokio::test]
    async fn test_search_supports_regex_alternation() {
        let mut tab = make_tab().await;
        let (m, _) = enter_search_and_type(mode(), &mut tab, "a.log|inner2").await;
        let (rows, _, _) = extract_state(m.render_state());
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        // "a.log" matches directly; "inner2.log" matches via the alternation
        // and pulls in its ancestor "bundle.zip" for context; "inner1.log"
        // matches neither branch, so it stays hidden.
        assert_eq!(names, vec!["a.log", "bundle.zip", "inner2.log"]);
    }

    #[tokio::test]
    async fn test_search_invalid_regex_does_not_panic_and_falls_back_to_literal() {
        let mut tab = make_tab().await;
        // "(inner" is an unclosed group — invalid regex mid-composition,
        // likely typed on the way to a real pattern like "(inner1|inner2)".
        // Must not panic, and must fall back to a literal substring search
        // rather than silently matching nothing — no node name contains the
        // literal text "(inner", so the narrowed list is empty either way,
        // but the important thing is this doesn't crash while typing.
        let (m, _) = enter_search_and_type(mode(), &mut tab, "(inner").await;
        let (rows, _, _) = extract_state(m.render_state());
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn test_backspace_removes_last_search_char() {
        let mut tab = make_tab().await;
        let (m, _) = enter_search_and_type(mode(), &mut tab, "in").await;
        let (m2, _) = m
            .handle_key(&mut tab, KeyCode::Backspace, KeyModifiers::NONE)
            .await;
        assert_eq!(extract_search(m2.render_state()), "i");
    }

    #[tokio::test]
    async fn test_action_letter_goes_to_query_while_searching_not_triggered() {
        let mut tab = make_tab().await;
        // 'a' is normally bound to "select all" — while searching it must be
        // captured as query text instead, so a filename search for "app.log"
        // (or anything starting with 'a') actually works.
        let (m, _) = enter_search_and_type(mode(), &mut tab, "a").await;
        assert_eq!(extract_search(m.render_state()), "a");
        let (rows, _, _) = extract_state(m.render_state());
        assert!(rows.iter().all(|r| r.check_state == CheckState::Unchecked));
    }

    #[tokio::test]
    async fn test_none_letter_goes_to_query_while_searching_not_triggered() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.tree.toggle_subtree(1);
        assert!(m.tree.any_file_selected());
        let (m, _) = enter_search_and_type(m, &mut tab, "n").await;
        assert_eq!(extract_search(m.render_state()), "n");
        let (rows, _, _) = extract_state(m.render_state());
        assert!(rows.iter().any(|r| r.check_state == CheckState::Checked));
    }

    #[tokio::test]
    async fn test_space_goes_to_query_while_searching_not_toggled() {
        let mut tab = make_tab().await;
        let (m, _) = enter_search_and_type(mode(), &mut tab, "a b").await;
        assert_eq!(extract_search(m.render_state()), "a b");
    }

    #[tokio::test]
    async fn test_j_navigates_within_narrowed_list_while_searching() {
        let mut tab = make_tab().await;
        let (m, _) = enter_search_and_type(mode(), &mut tab, "inner").await;
        // Narrowed to ["bundle.zip", "inner1.log", "inner2.log"].
        let (rows, selected, _) = extract_state(m.render_state());
        assert_eq!(rows.len(), 3);
        assert_eq!(selected, 0);
        let (m, _) = m
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        let (rows, selected, _) = extract_state(m.render_state());
        assert_eq!(selected, 1);
        assert_eq!(rows[1].name, "inner1.log");
    }

    #[tokio::test]
    async fn test_typing_a_character_resets_selection_to_top_of_narrowed_list() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 3;
        let (m, _) = enter_search_and_type(m, &mut tab, "inner1").await;
        // Only "bundle.zip" and "inner1.log" match "inner1" -> 2 rows, selection
        // must have been reclamped down from wherever it started.
        let (rows, selected, _) = extract_state(m.render_state());
        assert_eq!(rows.len(), 2);
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_enter_confirms_search_and_translates_to_full_list_index() {
        let mut tab = make_tab().await;
        let (m, _) = enter_search_and_type(mode(), &mut tab, "inner1").await;
        // Narrowed selection is on "bundle.zip" (index 0 of the narrowed list).
        let (m, result) = m
            .handle_key(&mut tab, KeyCode::Enter, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!extract_searching(m.render_state()));
        assert_eq!(extract_search(m.render_state()), "");
        let (rows, selected, _) = extract_state(m.render_state());
        // Search cleared -> full unfiltered list is shown again.
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[selected].name, "bundle.zip");
    }

    #[tokio::test]
    async fn test_esc_cancels_search_and_restores_original_selection() {
        let mut tab = make_tab().await;
        let mut m = mode();
        m.selected = 2; // "inner1.log"
        let (m, _) = enter_search_and_type(m, &mut tab, "bundle").await;
        let (m, result) = m
            .handle_key(&mut tab, KeyCode::Esc, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!extract_searching(m.render_state()));
        assert_eq!(extract_search(m.render_state()), "");
        let (rows, selected, _) = extract_state(m.render_state());
        assert_eq!(rows.len(), 4);
        assert_eq!(selected, 2);
        assert_eq!(rows[selected].name, "inner1.log");
    }

    #[tokio::test]
    async fn test_esc_exits_mode_when_not_searching() {
        let mut tab = make_tab().await;
        let (mode2, result) = press(mode(), &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ArchivePicker { .. }
        ));
    }

    /// A directory-sourced tree's subdirectory (see
    /// `crate::ingestion::list_directory_tree`) must get the exact same
    /// picker treatment an archive's containers do: rendered as an
    /// expandable/collapsible `Container` row, and ticking it checks every
    /// file nested inside — this already falls out of the generic
    /// `Container { .. }` handling everywhere in this file and in
    /// `ArchiveTree`, since a directory container is just a `Container`
    /// with `archive_type: None`.
    #[tokio::test]
    async fn test_directory_sourced_subdirectory_renders_as_container_and_marks_nested_files() {
        let mut tab = make_tab().await;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        std::fs::write(tmp.path().join("subdir/a.log"), b"one").unwrap();
        std::fs::write(tmp.path().join("subdir/b.log"), b"two").unwrap();
        let tree = crate::ingestion::list_directory_tree(tmp.path().to_str().unwrap()).unwrap();

        let mut m = ArchivePickerMode::new(tree, tmp.path().to_str().unwrap().to_string());
        let (rows, _, _) = extract_state(m.render_state());
        assert_eq!(
            rows.len(),
            3,
            "subdir starts expanded, showing both nested files"
        );
        let subdir_row = rows.iter().find(|r| r.name == "subdir").unwrap();
        assert_eq!(subdir_row.kind, RowKind::Container { collapsed: false });
        assert_eq!(subdir_row.check_state, CheckState::Unchecked);

        // Ticking the subdirectory row must tick every file nested inside it.
        let subdir_idx = rows.iter().position(|r| r.name == "subdir").unwrap();
        m.selected = subdir_idx;
        let (mode2, _) = press(m, &mut tab, KeyCode::Char(' ')).await;
        let (rows, _, _) = extract_state(mode2.render_state());
        assert!(
            rows.iter()
                .filter(|r| r.kind == RowKind::File)
                .all(|r| r.check_state == CheckState::Checked),
            "ticking the subdirectory container must tick every file inside it"
        );
    }
}
