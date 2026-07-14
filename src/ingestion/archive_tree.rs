use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::sync::Arc;

use crate::ingestion::archive::{decompress_to_temp, detect_archive_type, stem};
use crate::ingestion::{ArchiveExtractionProgress, ArchiveType, ExtractedFile};

/// Maximum total entries (across all nesting levels) a single listing pass
/// will walk, to bound pathological/zip-bomb-style archives. Archives-
/// within-archives are descended into with no nesting-depth limit of their
/// own — this entry cap is what bounds a listing pass, regardless of how
/// deeply nested the entries that hit it are.
///
/// Real-world archives (dozens of top-level zips each containing hundreds
/// of entries, many of which are themselves further-nested zips of
/// compressed logs) can easily total tens of thousands of entries once
/// fully expanded. Once the shared budget runs out mid-listing, every
/// *remaining* sibling of whichever container hit the cap — not just its
/// own descendants — is silently swallowed into one generic "entry limit
/// reached" marker (see `entry_budget_exhausted`), so a too-low cap can
/// make specific nested files vanish from the listing entirely with no
/// indication of where. 10,000 was hit by ordinary real-world archives of
/// this shape; this is deliberately generous.
pub const MAX_TOTAL_ENTRIES: usize = 250_000;
/// Maximum cumulative bytes of streaming-source (TarGz/TarBz2/TarXz) entry
/// content retained in memory during listing for reuse at extraction time.
pub const MAX_CACHED_BYTES: u64 = 256 * 1024 * 1024;
/// Deepest nesting level a listing pass recurses into automatically — depth
/// 0 (the opened file's own entries) and depth 1 (one layer of nested-archive
/// contents). A nested archive found any deeper becomes a [`NodeKind::LazyContainer`]
/// placeholder instead of being eagerly decompressed and parsed: still fully
/// reachable and lossless (unlike the old fixed recursion cap this replaced,
/// which showed an [`NodeKind::UnreadableContainer`] dead end), just deferred
/// until the user expands it.
pub const AUTO_EXPAND_DEPTH: usize = 1;

#[derive(Debug, Clone, Copy)]
pub struct ListLimits {
    pub max_entries: usize,
    pub max_cached_bytes: u64,
    pub auto_expand_depth: usize,
}

impl Default for ListLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_TOTAL_ENTRIES,
            max_cached_bytes: MAX_CACHED_BYTES,
            auto_expand_depth: AUTO_EXPAND_DEPTH,
        }
    }
}

/// Index into [`ArchiveTree::nodes`].
pub type NodeId = usize;

#[derive(Debug, Clone)]
pub struct ArchiveNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    /// Basename, for display.
    pub name: String,
    /// Path within its containing archive, e.g. `"logs/app.log"`.
    pub full_path: String,
    /// 0 = top-level entry of the opened file; +1 per nested archive layer.
    pub depth: usize,
    pub kind: NodeKind,
    /// Meaningful for `File` nodes; a container's checkbox state is derived
    /// from its descendants, not stored here.
    pub selected: bool,
    /// Independent of `selected` — marks this file to be extracted and
    /// merged into one timestamp-sorted tab, rather than opened on its own.
    /// A file can be `selected`, `merge_marked`, both, or neither.
    pub merge_marked: bool,
    /// Populated only when this entry's bytes were already buffered while
    /// listing a streaming (TarGz/TarBz2/TarXz) source, so extraction can
    /// reuse them instead of decompressing the parent stream a second time.
    pub cached_bytes: Option<Arc<Vec<u8>>>,
    /// Meaningful only for `Container` — whether its children are folded out
    /// of [`ArchiveTree::visible_rows`]. A `LazyContainer` needs no such flag
    /// of its own: having no children yet already keeps it out of the row
    /// list until it's expanded.
    pub collapsed: bool,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    File,
    Container {
        children: Vec<NodeId>,
        archive_type: ArchiveType,
    },
    /// A nested archive found past [`AUTO_EXPAND_DEPTH`] whose contents
    /// haven't been read yet — not an error state, just deferred. Becomes a
    /// `Container` once [`ArchiveTree::expand_lazy_node`] is called on it.
    LazyContainer {
        archive_type: ArchiveType,
    },
    /// A nested archive that failed to parse, or whose bytes couldn't even
    /// be read — shown as a non-expandable row with an error marker.
    /// Listing (or expanding) one bad nested entry must never abort the
    /// whole tree.
    UnreadableContainer {
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Checked,
    Unchecked,
    Partial,
}

/// Selects which of `ArchiveNode`'s two independent per-file flags a
/// tree-walking method operates on — lets `container_check_state`/
/// `check_states`/`toggle_subtree` share one implementation each between
/// `selected` (extraction) and `merge_marked` (merge) instead of being
/// duplicated wholesale for the second flag.
#[derive(Debug, Clone, Copy)]
enum MarkField {
    Selected,
    MergeMarked,
}

impl MarkField {
    fn get(self, node: &ArchiveNode) -> bool {
        match self {
            MarkField::Selected => node.selected,
            MarkField::MergeMarked => node.merge_marked,
        }
    }

    fn set(self, node: &mut ArchiveNode, value: bool) {
        match self {
            MarkField::Selected => node.selected = value,
            MarkField::MergeMarked => node.merge_marked = value,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ArchiveTree {
    pub nodes: Vec<ArchiveNode>,
    pub roots: Vec<NodeId>,
}

impl ArchiveTree {
    /// Rows in the order they should be rendered: a pre-order depth-first
    /// walk of `roots`, skipping a `Container`'s children while it's
    /// `collapsed`. A `LazyContainer` naturally contributes no rows beyond
    /// itself, since it has no children to descend into until expanded.
    pub fn visible_rows(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for &root in &self.roots {
            self.push_preorder(root, &mut out);
        }
        out
    }

    fn push_preorder(&self, id: NodeId, out: &mut Vec<NodeId>) {
        out.push(id);
        if let NodeKind::Container { children, .. } = &self.nodes[id].kind
            && !self.nodes[id].collapsed
        {
            for &child in children {
                self.push_preorder(child, out);
            }
        }
    }

    /// Every `File` descendant of `id` (including `id` itself if it is a
    /// `File`). `UnreadableContainer`/`LazyContainer` subtrees contribute no
    /// files, since they have no known children to select.
    fn descendant_files(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_descendant_files(id, &mut out);
        out
    }

    fn collect_descendant_files(&self, id: NodeId, out: &mut Vec<NodeId>) {
        match &self.nodes[id].kind {
            NodeKind::File => out.push(id),
            NodeKind::Container { children, .. } => {
                for &child in children {
                    self.collect_descendant_files(child, out);
                }
            }
            NodeKind::LazyContainer { .. } | NodeKind::UnreadableContainer { .. } => {}
        }
    }

    /// A container's checkbox state, derived from its descendant files —
    /// never stored, always computed fresh so it can't drift out of sync.
    fn check_state_for(&self, id: NodeId, field: MarkField) -> CheckState {
        let files = self.descendant_files(id);
        if files.is_empty() {
            return CheckState::Unchecked;
        }
        let all_set = files.iter().all(|&fid| field.get(&self.nodes[fid]));
        if all_set {
            return CheckState::Checked;
        }
        let any_set = files.iter().any(|&fid| field.get(&self.nodes[fid]));
        if any_set {
            CheckState::Partial
        } else {
            CheckState::Unchecked
        }
    }

    /// Bulk equivalent of calling [`Self::check_state_for`] once per node —
    /// for rendering a full row list, that naive approach re-walks each
    /// container's descendants from scratch, so nested containers pay for
    /// their shared descendants over and over (worst case `O(n * depth)`
    /// for a chain of containers each wrapping the same underlying files).
    /// This computes every node's state in a single `O(n)` pass instead, by
    /// relying on the arena invariant that a node's id is always smaller
    /// than any of its descendants' ids (parents are pushed before their
    /// children while listing) — iterating ids from highest to lowest is
    /// therefore a valid bottom-up (children-before-parents) order without
    /// needing recursion.
    fn check_states_for(&self, field: MarkField) -> Vec<CheckState> {
        let mut total_files = vec![0usize; self.nodes.len()];
        let mut set_files = vec![0usize; self.nodes.len()];
        for id in (0..self.nodes.len()).rev() {
            match &self.nodes[id].kind {
                NodeKind::File => {
                    total_files[id] = 1;
                    if field.get(&self.nodes[id]) {
                        set_files[id] = 1;
                    }
                }
                NodeKind::Container { children, .. } => {
                    for &child in children {
                        total_files[id] += total_files[child];
                        set_files[id] += set_files[child];
                    }
                }
                NodeKind::LazyContainer { .. } | NodeKind::UnreadableContainer { .. } => {}
            }
        }
        (0..self.nodes.len())
            .map(|id| {
                if total_files[id] == 0 {
                    CheckState::Unchecked
                } else if set_files[id] == total_files[id] {
                    CheckState::Checked
                } else if set_files[id] == 0 {
                    CheckState::Unchecked
                } else {
                    CheckState::Partial
                }
            })
            .collect()
    }

    /// Toggling a `File` row flips just that node. Toggling a `Container`
    /// row is a "select all in this subtree" shortcut: if every descendant
    /// file is already set, unset them all; otherwise set them all.
    /// `LazyContainer`/`UnreadableContainer` rows have nothing to toggle —
    /// a lazy one must be expanded first before its files can be selected.
    fn toggle_subtree_for(&mut self, id: NodeId, field: MarkField) {
        match &self.nodes[id].kind {
            NodeKind::File => {
                let new_value = !field.get(&self.nodes[id]);
                field.set(&mut self.nodes[id], new_value);
            }
            NodeKind::Container { .. } => {
                let target = self.check_state_for(id, field) != CheckState::Checked;
                for fid in self.descendant_files(id) {
                    field.set(&mut self.nodes[fid], target);
                }
            }
            NodeKind::LazyContainer { .. } | NodeKind::UnreadableContainer { .. } => {}
        }
    }

    pub fn container_check_state(&self, id: NodeId) -> CheckState {
        self.check_state_for(id, MarkField::Selected)
    }

    pub fn check_states(&self) -> Vec<CheckState> {
        self.check_states_for(MarkField::Selected)
    }

    pub fn toggle_subtree(&mut self, id: NodeId) {
        self.toggle_subtree_for(id, MarkField::Selected);
    }

    pub fn merge_container_check_state(&self, id: NodeId) -> CheckState {
        self.check_state_for(id, MarkField::MergeMarked)
    }

    pub fn merge_check_states(&self) -> Vec<CheckState> {
        self.check_states_for(MarkField::MergeMarked)
    }

    pub fn toggle_merge_subtree(&mut self, id: NodeId) {
        self.toggle_subtree_for(id, MarkField::MergeMarked);
    }

    pub fn any_file_selected(&self) -> bool {
        self.nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::File) && n.selected)
    }

    pub fn any_file_merge_marked(&self) -> bool {
        self.nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::File) && n.merge_marked)
    }

    /// Parses `bytes` (the lazy node's own raw archive bytes, fetched via
    /// [`resolve_node_bytes`]) into real children, turning `node_id` from a
    /// `LazyContainer` into a `Container` — exactly what listing would have
    /// done for it eagerly, had it not been deferred past `AUTO_EXPAND_DEPTH`.
    /// Falls back to a plain `File` if `bytes` doesn't actually parse as the
    /// claimed archive type (mirrors the eager path's "named like an archive
    /// but isn't one" fallback). A no-op if `node_id` isn't currently a
    /// `LazyContainer` (a stale/duplicate expand request).
    pub fn expand_lazy_node(&mut self, node_id: NodeId, bytes: Vec<u8>) {
        let (archive_type, depth) = match &self.nodes[node_id].kind {
            NodeKind::LazyContainer { archive_type } => {
                (archive_type.clone(), self.nodes[node_id].depth)
            }
            _ => return,
        };
        let mut state = ListingState::new(ListLimits::default());
        self.nodes[node_id].kind = match list_nested(
            &archive_type,
            bytes,
            Some(node_id),
            depth + 1,
            &mut self.nodes,
            &mut state,
        ) {
            Ok(children) => NodeKind::Container {
                children,
                archive_type,
            },
            Err(_) => NodeKind::File,
        };
    }

    /// Marks `node_id` as failed to even read (as opposed to
    /// [`Self::expand_lazy_node`]'s parse-failure fallback to `File`) — used
    /// when the background fetch behind a manual expand can't get the node's
    /// bytes at all, mirroring the eager listing path's own read-failure
    /// handling.
    pub fn mark_unreadable(&mut self, node_id: NodeId, error: String) {
        self.nodes[node_id].kind = NodeKind::UnreadableContainer { error };
    }

    /// Folds (`true`) or reveals (`false`) a `Container`'s children in
    /// [`Self::visible_rows`], without discarding them. No-op on any other
    /// node kind.
    pub fn set_collapsed(&mut self, node_id: NodeId, collapsed: bool) {
        if matches!(self.nodes[node_id].kind, NodeKind::Container { .. }) {
            self.nodes[node_id].collapsed = collapsed;
        }
    }
}

/// Returns true for archive types that contain multiple independently
/// selectable entries — only these trigger recursion when found nested
/// inside another archive. A lone `.gz`/`.bz2`/`.xz` entry wraps exactly one
/// file, so it's shown as an ordinary leaf rather than an expandable node.
fn is_multi_entry_archive(t: &ArchiveType) -> bool {
    matches!(
        t,
        ArchiveType::Zip
            | ArchiveType::Tar
            | ArchiveType::TarGz
            | ArchiveType::TarBz2
            | ArchiveType::TarXz
    )
}

fn basename(full_path: &str) -> String {
    full_path
        .rsplit('/')
        .next()
        .unwrap_or(full_path)
        .to_string()
}

/// Reserves `nodes[id]` as a `Container` with no children yet — the caller
/// fills in `children`/`archive_type` (or replaces the whole `kind` with
/// `UnreadableContainer`) once recursion into it finishes. Needed because a
/// node's `id` must equal its final index in `nodes`, but its children (with
/// `parent: Some(id)`) are appended *after* this reservation.
fn placeholder_container_node(
    id: NodeId,
    parent: Option<NodeId>,
    name: String,
    full_path: String,
    depth: usize,
) -> ArchiveNode {
    ArchiveNode {
        id,
        parent,
        name,
        full_path,
        depth,
        kind: NodeKind::Container {
            children: Vec::new(),
            archive_type: ArchiveType::Zip,
        },
        selected: false,
        merge_marked: false,
        cached_bytes: None,
        collapsed: false,
    }
}

fn unreadable_node(
    id: NodeId,
    parent: Option<NodeId>,
    name: String,
    full_path: String,
    depth: usize,
    error: &str,
) -> ArchiveNode {
    ArchiveNode {
        id,
        parent,
        name,
        full_path,
        depth,
        kind: NodeKind::UnreadableContainer {
            error: error.to_string(),
        },
        selected: false,
        merge_marked: false,
        cached_bytes: None,
        collapsed: false,
    }
}

/// A nested archive found past `AUTO_EXPAND_DEPTH` — not read yet, see
/// [`NodeKind::LazyContainer`].
fn lazy_container_node(
    id: NodeId,
    parent: Option<NodeId>,
    name: String,
    full_path: String,
    depth: usize,
    archive_type: ArchiveType,
) -> ArchiveNode {
    ArchiveNode {
        id,
        parent,
        name,
        full_path,
        depth,
        kind: NodeKind::LazyContainer { archive_type },
        selected: false,
        merge_marked: false,
        cached_bytes: None,
        collapsed: false,
    }
}

fn file_node(
    id: NodeId,
    parent: Option<NodeId>,
    name: String,
    full_path: String,
    depth: usize,
) -> ArchiveNode {
    ArchiveNode {
        id,
        parent,
        name,
        full_path,
        depth,
        kind: NodeKind::File,
        selected: false,
        merge_marked: false,
        cached_bytes: None,
        collapsed: false,
    }
}

/// Appends a row marking that listing stopped early because [`ListLimits::max_entries`]
/// was reached. Does not count against the entry budget itself.
fn push_truncated_marker(
    parent: Option<NodeId>,
    depth: usize,
    nodes: &mut Vec<ArchiveNode>,
) -> NodeId {
    let id = nodes.len();
    nodes.push(unreadable_node(
        id,
        parent,
        "…".to_string(),
        "…".to_string(),
        depth,
        "entry limit reached",
    ));
    id
}

/// Mutable bookkeeping threaded through a whole listing pass: how many
/// entries have been walked (against [`ListLimits::max_entries`]) and how
/// many bytes have been retained in [`ArchiveNode::cached_bytes`] so far
/// (against [`ListLimits::max_cached_bytes`]).
struct ListingState {
    entry_count: usize,
    cached_bytes_used: u64,
    limits: ListLimits,
}

impl ListingState {
    fn new(limits: ListLimits) -> Self {
        Self {
            entry_count: 0,
            cached_bytes_used: 0,
            limits,
        }
    }

    fn entry_budget_exhausted(&self) -> bool {
        self.entry_count >= self.limits.max_entries
    }

    /// Retains `bytes` for later reuse at extraction time, unless doing so
    /// would exceed the cumulative cache budget — in which case the caller
    /// falls back to re-reading from the source archive during extraction.
    fn cache_bytes(&mut self, bytes: Vec<u8>) -> Option<Arc<Vec<u8>>> {
        let len = bytes.len() as u64;
        if self.cached_bytes_used + len > self.limits.max_cached_bytes {
            return None;
        }
        self.cached_bytes_used += len;
        Some(Arc::new(bytes))
    }
}

pub fn list_archive_tree(path: &str) -> Result<ArchiveTree, String> {
    list_archive_tree_with_limits(path, &ListLimits::default())
}

pub fn list_archive_tree_with_limits(
    path: &str,
    limits: &ListLimits,
) -> Result<ArchiveTree, String> {
    let archive_type = detect_archive_type(path)
        .ok_or_else(|| format!("'{}' is not a recognised archive format", path))?;
    let mut nodes = Vec::new();
    let mut state = ListingState::new(*limits);
    let roots = list_top_level(path, &archive_type, &mut nodes, &mut state)?;
    Ok(ArchiveTree { nodes, roots })
}

fn list_top_level(
    path: &str,
    archive_type: &ArchiveType,
    nodes: &mut Vec<ArchiveNode>,
    state: &mut ListingState,
) -> Result<Vec<NodeId>, String> {
    match archive_type {
        ArchiveType::Zip => {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            list_zip_entries(file, None, 0, nodes, state)
        }
        ArchiveType::Tar => {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            list_tar_entries(file, None, 0, nodes, state, false)
        }
        ArchiveType::TarGz => {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            list_tar_entries(
                flate2::read::GzDecoder::new(file),
                None,
                0,
                nodes,
                state,
                true,
            )
        }
        ArchiveType::TarBz2 => {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            list_tar_entries(
                bzip2::read::BzDecoder::new(file),
                None,
                0,
                nodes,
                state,
                true,
            )
        }
        ArchiveType::TarXz => {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            list_tar_entries(xz2::read::XzDecoder::new(file), None, 0, nodes, state, true)
        }
        ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz => {
            let name = stem(&stem(path));
            let id = nodes.len();
            nodes.push(file_node(id, None, name.clone(), name, 0));
            state.entry_count += 1;
            Ok(vec![id])
        }
    }
}

/// Lists entries of a nested archive whose full bytes have already been
/// buffered (`buf`) — used for anything found *inside* another archive,
/// regardless of what format the outer archive was.
fn list_nested(
    archive_type: &ArchiveType,
    buf: Vec<u8>,
    parent: Option<NodeId>,
    depth: usize,
    nodes: &mut Vec<ArchiveNode>,
    state: &mut ListingState,
) -> Result<Vec<NodeId>, String> {
    match archive_type {
        ArchiveType::Zip => list_zip_entries(Cursor::new(buf), parent, depth, nodes, state),
        ArchiveType::Tar => list_tar_entries(Cursor::new(buf), parent, depth, nodes, state, false),
        ArchiveType::TarGz => list_tar_entries(
            flate2::read::GzDecoder::new(Cursor::new(buf)),
            parent,
            depth,
            nodes,
            state,
            true,
        ),
        ArchiveType::TarBz2 => list_tar_entries(
            bzip2::read::BzDecoder::new(Cursor::new(buf)),
            parent,
            depth,
            nodes,
            state,
            true,
        ),
        ArchiveType::TarXz => list_tar_entries(
            xz2::read::XzDecoder::new(Cursor::new(buf)),
            parent,
            depth,
            nodes,
            state,
            true,
        ),
        ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz => {
            unreachable!("callers only recurse into is_multi_entry_archive() types")
        }
    }
}

fn list_zip_entries<R: Read + Seek>(
    reader: R,
    parent: Option<NodeId>,
    depth: usize,
    nodes: &mut Vec<ArchiveNode>,
    state: &mut ListingState,
) -> Result<Vec<NodeId>, String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    let mut children = Vec::new();
    for i in 0..archive.len() {
        if state.entry_budget_exhausted() {
            children.push(push_truncated_marker(parent, depth, nodes));
            break;
        }
        let (is_dir, full_path) = {
            let entry = archive.by_index_raw(i).map_err(|e| e.to_string())?;
            let is_dir = entry.is_dir();
            let full_path = entry
                .enclosed_name()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| format!("file_{i}"));
            (is_dir, full_path)
        };
        if is_dir {
            continue;
        }
        let name = basename(&full_path);

        // Zip supports cheap by-index re-reads, so nothing here is ever
        // cached for reuse at extraction time (unlike the streaming tar
        // formats handled in `list_tar_entries`).
        match detect_archive_type(&name).filter(is_multi_entry_archive) {
            Some(nested_type) if depth < state.limits.auto_expand_depth => {
                let mut buf = Vec::new();
                let read_result = archive
                    .by_index(i)
                    .map_err(|e| e.to_string())
                    .and_then(|mut zf| zf.read_to_end(&mut buf).map_err(|e| e.to_string()));
                let id = nodes.len();
                state.entry_count += 1;
                match read_result {
                    Ok(_) => {
                        nodes.push(placeholder_container_node(
                            id, parent, name, full_path, depth,
                        ));
                        nodes[id].kind =
                            match list_nested(&nested_type, buf, Some(id), depth + 1, nodes, state)
                            {
                                Ok(nested_children) => NodeKind::Container {
                                    children: nested_children,
                                    archive_type: nested_type,
                                },
                                // Named like an archive but didn't actually parse as one
                                // (garbage bytes, or an empty placeholder file) — fall back
                                // to a plain, selectable file rather than a dead-end row.
                                Err(_) => NodeKind::File,
                            };
                    }
                    Err(e) => nodes.push(unreadable_node(id, parent, name, full_path, depth, &e)),
                }
                children.push(id);
            }
            // Past the auto-expand depth — deferred rather than eagerly
            // decompressed. Zip's cheap by-index re-reads mean the entry's
            // bytes don't even need to be read here; `expand_lazy_node` reads
            // them later, on demand, via `resolve_node_bytes`.
            Some(nested_type) => {
                let id = nodes.len();
                state.entry_count += 1;
                nodes.push(lazy_container_node(
                    id,
                    parent,
                    name,
                    full_path,
                    depth,
                    nested_type,
                ));
                children.push(id);
            }
            None => {
                let id = nodes.len();
                state.entry_count += 1;
                nodes.push(file_node(id, parent, name, full_path, depth));
                children.push(id);
            }
        }
    }
    Ok(children)
}

/// Lists entries of a tar stream. `should_cache` is true only for the
/// streaming formats (TarGz/TarBz2/TarXz) where a second listing/extraction
/// pass would mean decompressing the whole stream again from the start —
/// there, every entry's bytes are read once here and retained (budget
/// permitting) on the node for extraction to reuse directly.
fn list_tar_entries<R: Read>(
    reader: R,
    parent: Option<NodeId>,
    depth: usize,
    nodes: &mut Vec<ArchiveNode>,
    state: &mut ListingState,
    should_cache: bool,
) -> Result<Vec<NodeId>, String> {
    let mut archive = tar::Archive::new(reader);
    let mut children = Vec::new();
    let entries = archive.entries().map_err(|e| e.to_string())?;
    for entry in entries {
        if state.entry_budget_exhausted() {
            children.push(push_truncated_marker(parent, depth, nodes));
            break;
        }
        let mut entry = entry.map_err(|e| e.to_string())?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let full_path = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let name = basename(&full_path);

        match detect_archive_type(&name).filter(is_multi_entry_archive) {
            Some(nested_type) if depth < state.limits.auto_expand_depth => {
                let mut buf = Vec::new();
                let read_result = entry.read_to_end(&mut buf).map_err(|e| e.to_string());
                let id = nodes.len();
                state.entry_count += 1;
                match read_result {
                    Ok(_) => {
                        nodes.push(placeholder_container_node(
                            id, parent, name, full_path, depth,
                        ));
                        let cached_bytes = if should_cache {
                            state.cache_bytes(buf.clone())
                        } else {
                            None
                        };
                        nodes[id].cached_bytes = cached_bytes;
                        nodes[id].kind =
                            match list_nested(&nested_type, buf, Some(id), depth + 1, nodes, state)
                            {
                                Ok(nested_children) => NodeKind::Container {
                                    children: nested_children,
                                    archive_type: nested_type,
                                },
                                // Named like an archive but didn't actually parse as one
                                // (garbage bytes, or an empty placeholder file) — fall back
                                // to a plain, selectable file rather than a dead-end row.
                                Err(_) => NodeKind::File,
                            };
                    }
                    Err(e) => nodes.push(unreadable_node(id, parent, name, full_path, depth, &e)),
                }
                children.push(id);
            }
            // Past the auto-expand depth. Unlike zip, a tar stream is
            // sequential — the entry's bytes must still be consumed to reach
            // whatever follows it — but the recursive parse into its
            // contents is skipped, deferred to `expand_lazy_node`. Cached
            // (budget permitting) so a later expand doesn't need to
            // re-decompress the outer stream from scratch.
            Some(nested_type) => {
                let mut buf = Vec::new();
                let read_result = entry.read_to_end(&mut buf).map_err(|e| e.to_string());
                let id = nodes.len();
                state.entry_count += 1;
                match read_result {
                    Ok(_) => {
                        let mut node =
                            lazy_container_node(id, parent, name, full_path, depth, nested_type);
                        if should_cache {
                            node.cached_bytes = state.cache_bytes(buf);
                        }
                        nodes.push(node);
                    }
                    Err(e) => nodes.push(unreadable_node(id, parent, name, full_path, depth, &e)),
                }
                children.push(id);
            }
            None => {
                let id = nodes.len();
                state.entry_count += 1;
                let mut node = file_node(id, parent, name, full_path, depth);
                if should_cache {
                    let mut buf = Vec::new();
                    if entry.read_to_end(&mut buf).is_ok() {
                        node.cached_bytes = state.cache_bytes(buf);
                    }
                }
                nodes.push(node);
                children.push(id);
            }
        }
    }
    Ok(children)
}

/// Extracts every `selected` file in `tree` to its own temp file, skipping
/// everything unselected (including whole nested-archive subtrees with no
/// selected descendants — those are never even opened).
fn extract_by_flag(
    path: &str,
    tree: &ArchiveTree,
    field: MarkField,
    progress_tx: tokio::sync::watch::Sender<ArchiveExtractionProgress>,
) -> Result<Vec<ExtractedFile>, String> {
    let matched: Vec<NodeId> = tree
        .nodes
        .iter()
        .filter(|n| field.get(n) && matches!(n.kind, NodeKind::File))
        .map(|n| n.id)
        .collect();

    let mut used_names: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(matched.len());
    let total = matched.len().max(1);
    for (i, &node_id) in matched.iter().enumerate() {
        let bytes = resolve_node_bytes(tree, node_id, path)?;
        let name = disambiguated_name(tree, node_id, &mut used_names);
        let extracted = decompress_to_temp(&mut Cursor::new(bytes), name)?;
        out.push(extracted);
        let _ = progress_tx.send(ArchiveExtractionProgress {
            file_index: i,
            fraction: (i + 1) as f64 / total as f64,
        });
    }
    Ok(out)
}

pub fn extract_selected(
    path: &str,
    tree: &ArchiveTree,
    progress_tx: tokio::sync::watch::Sender<ArchiveExtractionProgress>,
) -> Result<Vec<ExtractedFile>, String> {
    extract_by_flag(path, tree, MarkField::Selected, progress_tx)
}

/// A merge-marked file's extracted, format-detected form — ready to feed
/// directly into building a merged tab without needing its own `TabState`/
/// `LogManager`/DB row (only the final merged tab needs one of those).
pub struct MergeMarkedSource {
    /// Same disambiguated display name `extract_selected` produces.
    pub label: String,
    pub reader: crate::ingestion::FileReader,
    pub detected: crate::ingestion::format_detect::DetectedFormat,
}

/// Extracts every `merge_marked` file in `tree`, exactly like
/// `extract_selected` extracts `selected` ones, but additionally loads each
/// extracted file into a `FileReader` and runs format detection on it —
/// letting the caller decide, before building any tab, whether every
/// merge-marked file's format was recognized.
pub fn extract_and_detect_merge_marked(
    path: &str,
    tree: &ArchiveTree,
    progress_tx: tokio::sync::watch::Sender<ArchiveExtractionProgress>,
) -> Result<Vec<MergeMarkedSource>, String> {
    let extracted = extract_by_flag(path, tree, MarkField::MergeMarked, progress_tx)?;
    Ok(extracted
        .into_iter()
        .map(|f| {
            let path_str = f.temp_file.path().to_string_lossy().to_string();
            let reader = crate::ingestion::FileReader::new(&path_str)
                .unwrap_or_else(|_| crate::ingestion::FileReader::from_bytes(vec![]));
            let detected = crate::ingestion::format_detect::detect_format_for_reader(&reader);
            MergeMarkedSource {
                label: f.name,
                reader,
                detected,
            }
        })
        .collect())
}

/// A selected file's display name: its basename (with any lone-compression
/// suffix stripped, see [`display_name_for_extraction`]), or — only once a
/// later selected file collides with an earlier one's basename — the
/// basename suffixed with its immediate containing archive's name, to keep
/// the common case (no collisions) free of visual noise.
fn disambiguated_name(
    tree: &ArchiveTree,
    node_id: NodeId,
    used: &mut HashMap<String, usize>,
) -> String {
    let node = &tree.nodes[node_id];
    let display_name = display_name_for_extraction(&node.name);
    let seen_before = used.entry(display_name.clone()).or_insert(0);
    *seen_before += 1;
    if *seen_before == 1 {
        return display_name;
    }
    match node.parent {
        Some(parent_id) => format!("{} ({})", display_name, tree.nodes[parent_id].name),
        None => display_name,
    }
}

/// The name to show for an extracted file: for a nested lone-compressed
/// entry (e.g. "app.log.gz"), the compression suffix is stripped since
/// [`resolve_node_bytes`] already decompresses it — the extracted content is
/// the same as if the entry had never been compressed. Any other name
/// (including root-level entries, already stripped during listing, and
/// entries merely named like a multi-entry archive) is returned unchanged.
fn display_name_for_extraction(name: &str) -> String {
    match detect_archive_type(name) {
        Some(ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz) => stem(name),
        _ => name.to_string(),
    }
}

/// Resolves a node's own *final* bytes — decompressed, if it's a nested lone
/// Gz/Bz2/Xz entry (see [`decompress_if_lone_compressed`]) — as stored
/// inside its immediate parent: via `cached_bytes` when available, otherwise
/// by walking down from the root, re-opening/re-decoding one archive layer
/// at a time. Also used to fetch a `LazyContainer`'s own raw archive bytes
/// ahead of [`ArchiveTree::expand_lazy_node`] — for a multi-entry archive
/// name, `decompress_if_lone_compressed` is a no-op, so this returns exactly
/// the nested archive's own bytes.
pub(crate) fn resolve_node_bytes(
    tree: &ArchiveTree,
    node_id: NodeId,
    path: &str,
) -> Result<Vec<u8>, String> {
    let node = &tree.nodes[node_id];
    if let Some(cached) = &node.cached_bytes {
        return decompress_if_lone_compressed(&node.name, (**cached).clone());
    }
    match node.parent {
        None => resolve_root_entry_bytes(path, &node.full_path),
        Some(parent_id) => {
            let parent_bytes = resolve_node_bytes(tree, parent_id, path)?;
            let parent_archive_type = match &tree.nodes[parent_id].kind {
                NodeKind::Container { archive_type, .. } => archive_type,
                other => {
                    return Err(format!(
                        "parent of a nested entry must be a container, got {other:?}"
                    ));
                }
            };
            let raw =
                read_entry_bytes_from_slice(&parent_bytes, parent_archive_type, &node.full_path)?;
            decompress_if_lone_compressed(&node.name, raw)
        }
    }
}

/// If `name` indicates a single-file compressed format (Gz/Bz2/Xz) — the
/// shape `list_zip_entries`/`list_tar_entries` leave as a plain `File` leaf
/// rather than expanding into a `Container`, since it wraps exactly one file
/// — decompresses `raw` to that file's actual content. Root-level entries of
/// this shape are already decompressed by [`resolve_root_entry_bytes`]
/// before reaching here, so this only ever fires for nested entries. Any
/// other name (including one merely *named* like a multi-entry archive that
/// failed to parse as one during listing) is returned unchanged.
fn decompress_if_lone_compressed(name: &str, raw: Vec<u8>) -> Result<Vec<u8>, String> {
    match detect_archive_type(name) {
        Some(ArchiveType::Gz) => read_to_end(flate2::read::GzDecoder::new(Cursor::new(raw))),
        Some(ArchiveType::Bz2) => read_to_end(bzip2::read::BzDecoder::new(Cursor::new(raw))),
        Some(ArchiveType::Xz) => read_to_end(xz2::read::XzDecoder::new(Cursor::new(raw))),
        _ => Ok(raw),
    }
}

/// Resolves `full_path`'s bytes from the top-level archive at `path`. For a
/// multi-entry container (Zip/Tar/...), that entry may itself be a lone
/// Gz/Bz2/Xz-compressed file (see [`decompress_if_lone_compressed`]) — e.g.
/// `path` is a `.zip` whose sole top-level entry is `app.log.gz` — so the
/// raw entry bytes get the same follow-up decompression check applied. For a
/// lone-compressed `path` itself, `full_path` was already stripped of its
/// compression suffix by `list_top_level`, so the check is a harmless no-op.
fn resolve_root_entry_bytes(path: &str, full_path: &str) -> Result<Vec<u8>, String> {
    let archive_type = detect_archive_type(path)
        .ok_or_else(|| format!("'{}' is not a recognised archive format", path))?;
    let raw = match archive_type {
        ArchiveType::Gz => read_to_end(flate2::read::GzDecoder::new(open(path)?)),
        ArchiveType::Bz2 => read_to_end(bzip2::read::BzDecoder::new(open(path)?)),
        ArchiveType::Xz => read_to_end(xz2::read::XzDecoder::new(open(path)?)),
        ArchiveType::Zip => read_zip_entry_bytes(open(path)?, full_path),
        ArchiveType::Tar => read_tar_entry_bytes(open(path)?, full_path),
        ArchiveType::TarGz => {
            read_tar_entry_bytes(flate2::read::GzDecoder::new(open(path)?), full_path)
        }
        ArchiveType::TarBz2 => {
            read_tar_entry_bytes(bzip2::read::BzDecoder::new(open(path)?), full_path)
        }
        ArchiveType::TarXz => {
            read_tar_entry_bytes(xz2::read::XzDecoder::new(open(path)?), full_path)
        }
    }?;
    decompress_if_lone_compressed(full_path, raw)
}

fn open(path: &str) -> Result<File, String> {
    File::open(path).map_err(|e| e.to_string())
}

fn read_to_end<R: Read>(mut reader: R) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Locates `full_path` inside an already-buffered nested archive (a
/// container's own bytes) and returns just that one entry's bytes.
fn read_entry_bytes_from_slice(
    bytes: &[u8],
    archive_type: &ArchiveType,
    full_path: &str,
) -> Result<Vec<u8>, String> {
    match archive_type {
        ArchiveType::Zip => read_zip_entry_bytes(Cursor::new(bytes), full_path),
        ArchiveType::Tar => read_tar_entry_bytes(Cursor::new(bytes), full_path),
        ArchiveType::TarGz => {
            read_tar_entry_bytes(flate2::read::GzDecoder::new(Cursor::new(bytes)), full_path)
        }
        ArchiveType::TarBz2 => {
            read_tar_entry_bytes(bzip2::read::BzDecoder::new(Cursor::new(bytes)), full_path)
        }
        ArchiveType::TarXz => {
            read_tar_entry_bytes(xz2::read::XzDecoder::new(Cursor::new(bytes)), full_path)
        }
        ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz => {
            unreachable!("callers only recurse into is_multi_entry_archive() types")
        }
    }
}

fn read_zip_entry_bytes<R: Read + Seek>(reader: R, full_path: &str) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    let mut zf = archive.by_name(full_path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    zf.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn read_tar_entry_bytes<R: Read>(reader: R, full_path: &str) -> Result<Vec<u8>, String> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|e| e.to_string())?;
    for entry in entries {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let entry_path = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        if entry_path == full_path {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            return Ok(buf);
        }
    }
    Err(format!("entry '{full_path}' not found in archive"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::archive::test_helpers::{
        make_bz2, make_gz, make_tar, make_tar_bz2, make_tar_gz, make_tar_xz, make_xz, make_zip,
    };
    use std::io::SeekFrom;

    fn path_with_ext(tmp: &tempfile::NamedTempFile, ext: &str) -> String {
        let path = tmp.path().to_str().unwrap().to_string() + ext;
        std::fs::copy(tmp.path(), &path).unwrap();
        path
    }

    fn read_extracted(file: &mut ExtractedFile) -> String {
        let mut content = String::new();
        file.temp_file.seek(SeekFrom::Start(0)).unwrap();
        file.temp_file.read_to_string(&mut content).unwrap();
        content
    }

    fn no_progress() -> tokio::sync::watch::Sender<ArchiveExtractionProgress> {
        tokio::sync::watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        })
        .0
    }

    fn select_by_full_path(tree: &mut ArchiveTree, full_path: &str) -> NodeId {
        let id = tree
            .nodes
            .iter()
            .find(|n| n.full_path == full_path)
            .unwrap_or_else(|| panic!("no node with full_path {full_path}"))
            .id;
        tree.nodes[id].selected = true;
        id
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
                archive_type: ArchiveType::Zip,
            },
            selected: false,
            merge_marked: false,
            cached_bytes: None,
            collapsed: false,
        }
    }

    /// Builds:
    ///   0: File "a.log"                (root)
    ///   1: Container "archive.zip"     (root) -> [2, 3]
    ///     2: File "inner1.log"
    ///     3: File "inner2.log"
    ///   4: Container "other.zip"       (root) -> [5, 6]
    ///     5: File "x.log"
    ///     6: File "y.log"
    fn build_test_tree() -> ArchiveTree {
        let nodes = vec![
            file_node(0, None, "a.log", 0),
            container_node(1, None, "archive.zip", 0, vec![2, 3]),
            file_node(2, Some(1), "inner1.log", 1),
            file_node(3, Some(1), "inner2.log", 1),
            container_node(4, None, "other.zip", 0, vec![5, 6]),
            file_node(5, Some(4), "x.log", 1),
            file_node(6, Some(4), "y.log", 1),
        ];
        ArchiveTree {
            nodes,
            roots: vec![0, 1, 4],
        }
    }

    #[test]
    fn test_list_zip_flat_files_full_path() {
        let tmp = make_zip(&[("logs/app.log", b"a"), ("logs/debug.log", b"b")]);
        let path = path_with_ext(&tmp, ".zip");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(tree.roots.len(), 2);
        let mut full_paths: Vec<&str> = tree.nodes.iter().map(|n| n.full_path.as_str()).collect();
        full_paths.sort();
        assert_eq!(full_paths, vec!["logs/app.log", "logs/debug.log"]);
        for node in &tree.nodes {
            assert!(matches!(node.kind, NodeKind::File));
            assert_eq!(node.depth, 0);
            assert!(node.parent.is_none());
        }
    }

    #[test]
    fn test_list_tar_flat_files_full_path() {
        let tmp = make_tar(&[("logs/app.log", b"a"), ("logs/debug.log", b"b")]);
        let path = path_with_ext(&tmp, ".tar");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(tree.roots.len(), 2);
        let mut full_paths: Vec<&str> = tree.nodes.iter().map(|n| n.full_path.as_str()).collect();
        full_paths.sort();
        assert_eq!(full_paths, vec!["logs/app.log", "logs/debug.log"]);
        for node in &tree.nodes {
            assert!(matches!(node.kind, NodeKind::File));
            assert_eq!(node.depth, 0);
        }
    }

    #[test]
    fn test_list_zip_names_use_basename() {
        let tmp = make_zip(&[("logs/app.log", b"a")]);
        let path = path_with_ext(&tmp, ".zip");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(tree.nodes[0].name, "app.log");
        assert_eq!(tree.nodes[0].full_path, "logs/app.log");
    }

    fn find_node<'a>(tree: &'a ArchiveTree, full_path: &str) -> &'a ArchiveNode {
        tree.nodes
            .iter()
            .find(|n| n.full_path == full_path)
            .unwrap_or_else(|| panic!("no node with full_path {full_path}"))
    }

    #[test]
    fn test_list_zip_in_zip_nests_entries() {
        let inner = make_zip(&[("a.log", b"inner-a"), ("b.log", b"inner-b")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_zip(&[
            ("top.log", b"top"),
            ("nested/inner.zip", inner_bytes.as_slice()),
        ]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let container = find_node(&tree, "nested/inner.zip");
        assert_eq!(container.depth, 0);
        let (children, archive_type) = match &container.kind {
            NodeKind::Container {
                children,
                archive_type,
            } => (children, archive_type),
            other => panic!("expected Container, got {other:?}"),
        };
        assert_eq!(*archive_type, ArchiveType::Zip);
        let mut names: Vec<&str> = children
            .iter()
            .map(|&id| tree.nodes[id].full_path.as_str())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.log", "b.log"]);
        for &id in children {
            assert_eq!(tree.nodes[id].depth, 1);
            assert_eq!(tree.nodes[id].parent, Some(container.id));
        }
    }

    #[test]
    fn test_list_tar_in_tar_nests_entries() {
        let inner = make_tar(&[("a.log", b"inner-a"), ("b.log", b"inner-b")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_tar(&[
            ("top.log", b"top"),
            ("nested/inner.tar", inner_bytes.as_slice()),
        ]);
        let path = path_with_ext(&outer_tmp, ".tar");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let container = find_node(&tree, "nested/inner.tar");
        assert_eq!(container.depth, 0);
        let (children, archive_type) = match &container.kind {
            NodeKind::Container {
                children,
                archive_type,
            } => (children, archive_type),
            other => panic!("expected Container, got {other:?}"),
        };
        assert_eq!(*archive_type, ArchiveType::Tar);
        let mut names: Vec<&str> = children
            .iter()
            .map(|&id| tree.nodes[id].full_path.as_str())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.log", "b.log"]);
        for &id in children {
            assert_eq!(tree.nodes[id].depth, 1);
            assert_eq!(tree.nodes[id].parent, Some(container.id));
        }
    }

    #[test]
    fn test_list_zip_in_tar_nests_entries() {
        let inner = make_zip(&[("a.log", b"inner-a")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_tar(&[("nested/inner.zip", inner_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".tar");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let container = find_node(&tree, "nested/inner.zip");
        match &container.kind {
            NodeKind::Container {
                children,
                archive_type,
            } => {
                assert_eq!(*archive_type, ArchiveType::Zip);
                assert_eq!(children.len(), 1);
                assert_eq!(tree.nodes[children[0]].full_path, "a.log");
                assert_eq!(tree.nodes[children[0]].depth, 1);
            }
            other => panic!("expected Container, got {other:?}"),
        }
    }

    #[test]
    fn test_list_tar_in_zip_nests_entries() {
        let inner = make_tar(&[("a.log", b"inner-a")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_zip(&[("nested/inner.tar", inner_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let container = find_node(&tree, "nested/inner.tar");
        match &container.kind {
            NodeKind::Container {
                children,
                archive_type,
            } => {
                assert_eq!(*archive_type, ArchiveType::Tar);
                assert_eq!(children.len(), 1);
                assert_eq!(tree.nodes[children[0]].full_path, "a.log");
                assert_eq!(tree.nodes[children[0]].depth, 1);
            }
            other => panic!("expected Container, got {other:?}"),
        }
    }

    #[test]
    fn test_corrupt_nested_archive_falls_back_to_plain_file() {
        // An entry named like an archive but whose content doesn't actually parse as
        // one (garbage bytes, or an empty/placeholder file some log-packaging script
        // accidentally created) must still be a selectable, extractable File — not a
        // dead-end row nobody can toggle. Only genuine I/O failures (can't even read
        // the entry's bytes) or policy limits (depth/entry caps) become UnreadableContainer.
        let outer_tmp = make_zip(&[
            ("bad.zip", b"this is not a valid zip file".as_slice()),
            ("empty.tar.gz", b"".as_slice()),
            ("good.log", b"fine".as_slice()),
        ]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let bad = find_node(&tree, "bad.zip");
        assert!(
            matches!(bad.kind, NodeKind::File),
            "expected File, got {:?}",
            bad.kind
        );

        let empty = find_node(&tree, "empty.tar.gz");
        assert!(
            matches!(empty.kind, NodeKind::File),
            "expected File, got {:?}",
            empty.kind
        );

        let good = find_node(&tree, "good.log");
        assert!(matches!(good.kind, NodeKind::File));
    }

    #[test]
    fn test_corrupt_nested_archive_entry_is_togglable_and_extractable() {
        let outer_tmp = make_zip(&[("bad.tar.gz", b"not actually a tar.gz".as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();

        let bad_id = select_by_full_path(&mut tree, "bad.tar.gz");
        assert!(
            tree.nodes[bad_id].selected,
            "toggling a fallback File node must actually flip its selection"
        );

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(extracted.len(), 1);
        assert_eq!(read_extracted(&mut extracted[0]), "not actually a tar.gz");
    }

    #[test]
    fn test_deeply_nested_archives_become_lazy_past_auto_expand_depth_not_truncated() {
        // 25 levels of zip-in-zip nesting. Under the default `auto_expand_depth`
        // (1), listing stops eagerly recursing after depth 0/1 — but unlike the
        // old fixed recursion cap this replaced, nothing deeper is lost or
        // shown as an error: it's simply not read yet (`LazyContainer`),
        // reachable on demand via `expand_lazy_node` (see the dedicated
        // `test_expand_lazy_node_*` tests below).
        fn wrap_in_zip(entry_name: &str, bytes: Vec<u8>) -> Vec<u8> {
            let tmp = make_zip(&[(entry_name, bytes.as_slice())]);
            std::fs::read(tmp.path()).unwrap()
        }

        const DEPTH: usize = 25;
        let mut bytes = {
            let tmp = make_zip(&[("leaf.log", b"leaf content".as_slice())]);
            std::fs::read(tmp.path()).unwrap()
        };
        for i in (1..DEPTH).rev() {
            bytes = wrap_in_zip(&format!("archive{i}.zip"), bytes);
        }
        let outer_tmp = make_zip(&[("archive0.zip", bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");

        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert!(
            !tree
                .nodes
                .iter()
                .any(|n| matches!(n.kind, NodeKind::UnreadableContainer { .. })),
            "nothing should be truncated as an error just for being deeply nested"
        );

        let archive0 = find_node(&tree, "archive0.zip");
        assert_eq!(archive0.depth, 0);
        assert!(matches!(archive0.kind, NodeKind::Container { .. }));

        let archive1 = find_node(&tree, "archive1.zip");
        assert_eq!(archive1.depth, 1);
        assert!(
            matches!(archive1.kind, NodeKind::LazyContainer { .. }),
            "past auto_expand_depth, archive1.zip must be lazy, got {:?}",
            archive1.kind
        );

        // Nothing past the lazy node was ever read — not present at all
        // (not even as an error row), since laziness is a "not yet" not a
        // "lost".
        assert!(
            tree.nodes.iter().all(|n| n.full_path != "archive2.zip"),
            "archive1.zip's own contents must not be read until expanded"
        );
        assert!(tree.nodes.iter().all(|n| n.full_path != "leaf.log"));
    }

    #[test]
    fn test_lazy_zip_entry_content_is_never_read_at_listing_time() {
        // A depth-1 entry named like a zip but containing garbage bytes: if
        // it had been eagerly read+parsed (as it would be at depth 0), the
        // "named like an archive but isn't one" fallback would turn it into
        // a plain `File`. It staying `LazyContainer` after listing is the
        // proof its bytes were never even opened yet — the actual "limit
        // automatic decompression" win for zip (which supports free by-index
        // re-reads, so nothing needs to be pre-buffered for later expansion).
        let middle = make_zip(&[("archive2.zip", b"not a valid zip".as_slice())]);
        let middle_bytes = std::fs::read(middle.path()).unwrap();
        let outer_tmp = make_zip(&[("archive1.zip", middle_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");

        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let inner = find_node(&tree, "archive2.zip");
        assert_eq!(inner.depth, 1);
        assert!(
            matches!(inner.kind, NodeKind::LazyContainer { .. }),
            "garbage content must not have been read/parsed yet, got {:?}",
            inner.kind
        );
    }

    #[test]
    fn test_lazy_tar_entry_still_consumes_stream_but_does_not_recurse() {
        // Unlike zip, a tar stream is sequential: a lazy entry's bytes must
        // still be consumed to reach whatever follows it in the same
        // stream. This proves the sibling *after* a lazy entry is still
        // discovered (stream position advanced correctly), while the lazy
        // entry itself doesn't get its own contents recursively parsed.
        let middle = make_tar(&[("archive2.tar", b"nested content".as_slice())]);
        let middle_bytes = std::fs::read(middle.path()).unwrap();
        let outer_tmp = make_tar(&[
            ("archive1.tar", middle_bytes.as_slice()),
            ("after.log", b"sibling after the lazy entry".as_slice()),
        ]);
        let path = path_with_ext(&outer_tmp, ".tar");

        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let lazy_child = find_node(&tree, "archive2.tar");
        assert_eq!(lazy_child.depth, 1);
        assert!(matches!(lazy_child.kind, NodeKind::LazyContainer { .. }));

        let sibling = find_node(&tree, "after.log");
        assert_eq!(sibling.depth, 0);
        assert!(matches!(sibling.kind, NodeKind::File));
    }

    #[test]
    fn test_expand_lazy_node_zip_populates_real_children_at_correct_depth() {
        let inner = make_zip(&[("a.log", b"one"), ("b.log", b"two")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let middle = make_zip(&[("inner.zip", inner_bytes.as_slice())]);
        let middle_bytes = std::fs::read(middle.path()).unwrap();
        let outer_tmp = make_zip(&[("middle.zip", middle_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");

        let mut tree = list_archive_tree(&path).unwrap();
        let lazy_id = find_node(&tree, "inner.zip").id;
        assert!(matches!(
            tree.nodes[lazy_id].kind,
            NodeKind::LazyContainer { .. }
        ));

        let bytes = resolve_node_bytes(&tree, lazy_id, &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        tree.expand_lazy_node(lazy_id, bytes);

        match &tree.nodes[lazy_id].kind {
            NodeKind::Container {
                children,
                archive_type,
            } => {
                assert_eq!(*archive_type, ArchiveType::Zip);
                let mut names: Vec<&str> = children
                    .iter()
                    .map(|&id| tree.nodes[id].full_path.as_str())
                    .collect();
                names.sort();
                assert_eq!(names, vec!["a.log", "b.log"]);
                for &id in children {
                    assert_eq!(tree.nodes[id].depth, 2);
                    assert_eq!(tree.nodes[id].parent, Some(lazy_id));
                }
            }
            other => panic!("expected Container after expand, got {other:?}"),
        }
    }

    #[test]
    fn test_expand_lazy_node_reveals_further_nesting_as_lazy_too() {
        // Expanding a lazy node always reveals exactly one level: a further
        // nested archive among its newly-revealed children is itself lazy,
        // not eagerly recursed into (the auto-expand depth check is
        // absolute, not "one more level from wherever you clicked").
        let innermost = make_zip(&[("leaf.log", b"leaf".as_slice())]);
        let innermost_bytes = std::fs::read(innermost.path()).unwrap();
        let inner = make_zip(&[("deeper.zip", innermost_bytes.as_slice())]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let middle = make_zip(&[("inner.zip", inner_bytes.as_slice())]);
        let middle_bytes = std::fs::read(middle.path()).unwrap();
        let outer_tmp = make_zip(&[("middle.zip", middle_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");

        let mut tree = list_archive_tree(&path).unwrap();
        let lazy_id = find_node(&tree, "inner.zip").id;
        let bytes = resolve_node_bytes(&tree, lazy_id, &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        tree.expand_lazy_node(lazy_id, bytes);

        let deeper = find_node(&tree, "deeper.zip");
        assert_eq!(deeper.depth, 2);
        assert!(
            matches!(deeper.kind, NodeKind::LazyContainer { .. }),
            "one expand reveals one level — deeper.zip must still be lazy, got {:?}",
            deeper.kind
        );
        assert!(
            tree.nodes.iter().all(|n| n.full_path != "leaf.log"),
            "content past the newly-revealed lazy node must not have been read"
        );
    }

    #[test]
    fn test_expand_lazy_node_falls_back_to_file_on_corrupt_content() {
        let middle = make_zip(&[("archive2.zip", b"not a valid zip".as_slice())]);
        let middle_bytes = std::fs::read(middle.path()).unwrap();
        let outer_tmp = make_zip(&[("archive1.zip", middle_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");

        let mut tree = list_archive_tree(&path).unwrap();
        let lazy_id = find_node(&tree, "archive2.zip").id;
        let bytes = resolve_node_bytes(&tree, lazy_id, &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(bytes, b"not a valid zip");
        tree.expand_lazy_node(lazy_id, bytes);

        assert!(matches!(tree.nodes[lazy_id].kind, NodeKind::File));
    }

    #[test]
    fn test_expand_lazy_node_is_a_no_op_on_a_non_lazy_node() {
        let mut tree = build_test_tree();
        let before_len = tree.nodes.len();
        tree.expand_lazy_node(1, vec![1, 2, 3]); // node 1 is already a real Container
        assert_eq!(tree.nodes.len(), before_len);
        assert!(matches!(tree.nodes[1].kind, NodeKind::Container { .. }));
    }

    #[test]
    fn test_mark_unreadable_sets_error_kind() {
        let mut tree = build_test_tree();
        tree.mark_unreadable(1, "boom".to_string());
        match &tree.nodes[1].kind {
            NodeKind::UnreadableContainer { error } => assert_eq!(error, "boom"),
            other => panic!("expected UnreadableContainer, got {other:?}"),
        }
    }

    #[test]
    fn test_set_collapsed_hides_descendants_from_visible_rows_but_keeps_them_in_nodes() {
        let mut tree = build_test_tree();
        let before = tree.visible_rows();
        assert!(before.contains(&2) && before.contains(&3));

        tree.set_collapsed(1, true);
        let collapsed_rows = tree.visible_rows();
        assert!(
            collapsed_rows.contains(&1),
            "the container row itself stays visible"
        );
        assert!(!collapsed_rows.contains(&2));
        assert!(!collapsed_rows.contains(&3));
        assert_eq!(tree.nodes.len(), 7, "collapsing must not discard nodes");

        tree.set_collapsed(1, false);
        assert_eq!(tree.visible_rows(), before);
    }

    #[test]
    fn test_set_collapsed_is_a_no_op_on_non_container_nodes() {
        let mut tree = build_test_tree();
        tree.set_collapsed(0, true); // node 0 is a File
        assert!(!tree.nodes[0].collapsed);
    }

    #[test]
    fn test_lazy_container_contributes_no_rows_beyond_itself() {
        let inner = make_zip(&[("a.log", b"one"), ("b.log", b"two")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let middle = make_zip(&[("inner.zip", inner_bytes.as_slice())]);
        let middle_bytes = std::fs::read(middle.path()).unwrap();
        let outer_tmp = make_zip(&[("middle.zip", middle_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");

        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let lazy = find_node(&tree, "inner.zip");
        assert!(matches!(lazy.kind, NodeKind::LazyContainer { .. }));
        let rows = tree.visible_rows();
        assert!(rows.contains(&lazy.id));
        assert!(
            rows.iter()
                .all(|&id| tree.nodes[id].full_path != "a.log"
                    && tree.nodes[id].full_path != "b.log"),
            "a lazy node's not-yet-fetched contents must not appear in visible_rows"
        );
    }

    #[test]
    fn test_entry_budget_exhausted_by_earlier_sibling_truncates_a_later_sibling_entirely() {
        // A large "big_sibling.zip" is listed first and alone consumes the
        // whole (small, test-only) entry budget. "target.zip" — a LATER
        // sibling in the same outer zip — never even gets its own node:
        // the outer zip's loop hits the exhausted-budget check on its very
        // next iteration and swallows every remaining sibling into one
        // generic truncation marker. This is the exact mechanism behind a
        // reported real-world case: a large nested archive (tens of
        // thousands of entries total) silently dropped a `.xz` file nested
        // a few levels down, once an earlier-listed sibling used up the
        // (too small, at the time — see `MAX_TOTAL_ENTRIES`) shared budget.
        let big_entries: Vec<(String, Vec<u8>)> = (0..10)
            .map(|i| (format!("f{i}.log"), b"x".to_vec()))
            .collect();
        let big_refs: Vec<(&str, &[u8])> = big_entries
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let big_sibling = make_zip(&big_refs);
        let big_sibling_bytes = std::fs::read(big_sibling.path()).unwrap();

        let xz = make_xz(b"target content");
        let xz_bytes = std::fs::read(xz.path()).unwrap();
        let target_inner = make_zip(&[("target.xz", xz_bytes.as_slice())]);
        let target_inner_bytes = std::fs::read(target_inner.path()).unwrap();

        let outer_tmp = make_zip(&[
            ("big_sibling.zip", big_sibling_bytes.as_slice()),
            ("target.zip", target_inner_bytes.as_slice()),
        ]);
        let path = path_with_ext(&outer_tmp, ".zip");

        let limits = ListLimits {
            max_entries: 5,
            ..ListLimits::default()
        };
        let tree = list_archive_tree_with_limits(&path, &limits).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert!(
            !tree.nodes.iter().any(|n| n.full_path == "target.xz"),
            "documents current (tiny-budget) truncation behavior: a later \
             sibling's contents are dropped once the shared entry budget \
             is exhausted by an earlier one"
        );
        assert!(
            tree.nodes
                .iter()
                .any(|n| matches!(n.kind, NodeKind::UnreadableContainer { .. })),
            "the outer zip must show a truncation marker for its swallowed remaining siblings"
        );
    }

    #[test]
    fn test_entry_count_cap_truncates_listing() {
        let entries: Vec<(String, Vec<u8>)> = (0..15)
            .map(|i| (format!("file{i}.log"), b"x".to_vec()))
            .collect();
        let entry_refs: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let tmp = make_zip(&entry_refs);
        let path = path_with_ext(&tmp, ".zip");

        let limits = ListLimits {
            max_entries: 5,
            ..ListLimits::default()
        };
        let tree = list_archive_tree_with_limits(&path, &limits).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            tree.nodes.len(),
            6,
            "5 real entries plus 1 truncation marker"
        );
        let marker = tree.nodes.last().unwrap();
        assert!(matches!(marker.kind, NodeKind::UnreadableContainer { .. }));
    }

    #[test]
    fn test_default_entry_budget_is_generous_enough_for_large_real_world_archives() {
        // Locks in the raised cap (see `MAX_TOTAL_ENTRIES`'s doc comment)
        // so a future accidental revert back toward the old 10,000 doesn't
        // silently reintroduce the "later sibling's contents vanish"
        // truncation for realistically large nested archives.
        assert_eq!(ListLimits::default().max_entries, 250_000);
    }

    #[test]
    fn test_list_tar_gz_flat_files_full_path() {
        let tmp = make_tar_gz(&[("logs/app.log", b"a"), ("logs/debug.log", b"b")]);
        let path = path_with_ext(&tmp, ".tar.gz");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(tree.roots.len(), 2);
        let mut full_paths: Vec<&str> = tree.nodes.iter().map(|n| n.full_path.as_str()).collect();
        full_paths.sort();
        assert_eq!(full_paths, vec!["logs/app.log", "logs/debug.log"]);
    }

    #[test]
    fn test_list_tar_bz2_flat_files_full_path() {
        let tmp = make_tar_bz2(&[("a.log", b"a")]);
        let path = path_with_ext(&tmp, ".tar.bz2");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.nodes[0].full_path, "a.log");
    }

    #[test]
    fn test_list_tar_xz_flat_files_full_path() {
        let tmp = make_tar_xz(&[("a.log", b"a")]);
        let path = path_with_ext(&tmp, ".tar.xz");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.nodes[0].full_path, "a.log");
    }

    #[test]
    fn test_lone_xz_three_levels_deep_in_zip_in_zip_is_reachable_via_visible_rows() {
        // zip (outer) > zip (inner) > app.log.xz — three levels, the lone
        // .xz at the bottom, listed and its ancestor's `visible_rows()`
        // reachability both checked (not just presence in `tree.nodes`).
        let xz = make_xz(b"third level content");
        let xz_bytes = std::fs::read(xz.path()).unwrap();
        let inner = make_zip(&[("app.log.xz", xz_bytes.as_slice())]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_zip(&[("inner.zip", inner_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let visible = tree.visible_rows();
        let xz_id = tree
            .nodes
            .iter()
            .find(|n| n.full_path == "app.log.xz")
            .expect("app.log.xz must exist in tree.nodes")
            .id;
        assert!(
            visible.contains(&xz_id),
            "app.log.xz (id {xz_id}) must be reachable via visible_rows(): {visible:?}"
        );
    }

    #[test]
    fn test_list_tar_gz_with_nested_zip() {
        let inner = make_zip(&[("a.log", b"inner-a")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_tar_gz(&[("nested/inner.zip", inner_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".tar.gz");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let container = find_node(&tree, "nested/inner.zip");
        match &container.kind {
            NodeKind::Container {
                children,
                archive_type,
            } => {
                assert_eq!(*archive_type, ArchiveType::Zip);
                assert_eq!(children.len(), 1);
                assert_eq!(tree.nodes[children[0]].full_path, "a.log");
                assert_eq!(tree.nodes[children[0]].depth, 1);
            }
            other => panic!("expected Container, got {other:?}"),
        }
    }

    #[test]
    fn test_streaming_tar_source_caches_entry_bytes_under_default_cap() {
        let tmp = make_tar_gz(&[("a.log", b"hello world")]);
        let path = path_with_ext(&tmp, ".tar.gz");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let node = find_node(&tree, "a.log");
        let cached = node
            .cached_bytes
            .as_ref()
            .expect("small entries from a streaming source should be cached by default");
        assert_eq!(cached.as_slice(), b"hello world");
    }

    #[test]
    fn test_streaming_tar_source_skips_cache_past_byte_budget() {
        let tmp = make_tar_gz(&[("a.log", b"hello world")]);
        let path = path_with_ext(&tmp, ".tar.gz");

        let limits = ListLimits {
            max_cached_bytes: 1,
            ..ListLimits::default()
        };
        let tree = list_archive_tree_with_limits(&path, &limits).unwrap();
        std::fs::remove_file(&path).unwrap();

        let node = find_node(&tree, "a.log");
        assert!(
            node.cached_bytes.is_none(),
            "an entry larger than the cache budget must not be cached"
        );
    }

    #[test]
    fn test_plain_tar_source_never_caches_entry_bytes() {
        let tmp = make_tar(&[("a.log", b"hello world")]);
        let path = path_with_ext(&tmp, ".tar");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let node = find_node(&tree, "a.log");
        assert!(
            node.cached_bytes.is_none(),
            "plain tar supports cheap re-reads, so nothing should be cached"
        );
    }

    #[test]
    fn test_zip_source_never_caches_entry_bytes() {
        let tmp = make_zip(&[("a.log", b"hello world")]);
        let path = path_with_ext(&tmp, ".zip");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let node = find_node(&tree, "a.log");
        assert!(
            node.cached_bytes.is_none(),
            "zip supports cheap by-index re-reads, so nothing should be cached"
        );
    }

    #[test]
    fn test_visible_rows_preorder_depth_first() {
        let tree = build_test_tree();
        assert_eq!(tree.visible_rows(), vec![0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_visible_rows_depths_match_nesting() {
        let tree = build_test_tree();
        let depths: Vec<usize> = tree
            .visible_rows()
            .iter()
            .map(|&id| tree.nodes[id].depth)
            .collect();
        assert_eq!(depths, vec![0, 0, 1, 1, 0, 1, 1]);
    }

    #[test]
    fn test_container_check_state_unchecked_when_no_children_selected() {
        let tree = build_test_tree();
        assert_eq!(tree.container_check_state(1), CheckState::Unchecked);
    }

    #[test]
    fn test_container_check_state_checked_when_all_children_selected() {
        let mut tree = build_test_tree();
        tree.nodes[2].selected = true;
        tree.nodes[3].selected = true;
        assert_eq!(tree.container_check_state(1), CheckState::Checked);
    }

    #[test]
    fn test_container_check_state_partial_when_some_children_selected() {
        let mut tree = build_test_tree();
        tree.nodes[2].selected = true;
        assert_eq!(tree.container_check_state(1), CheckState::Partial);
    }

    #[test]
    fn test_check_states_matches_container_check_state_for_every_node() {
        // `check_states()` is a bulk O(n) alternative to calling
        // `container_check_state(id)` once per node — must agree with it
        // for every node in the tree, in several selection states.
        for setup in [Vec::<usize>::new(), vec![2], vec![2, 3], vec![2, 3, 5, 6]] {
            let mut tree = build_test_tree();
            for &idx in &setup {
                tree.nodes[idx].selected = true;
            }
            let states = tree.check_states();
            assert_eq!(states.len(), tree.nodes.len());
            for (id, &state) in states.iter().enumerate() {
                assert_eq!(
                    state,
                    tree.container_check_state(id),
                    "node {id} mismatched for selection {setup:?}"
                );
            }
        }
    }

    #[test]
    fn test_check_states_matches_container_check_state_for_deeply_nested_tree() {
        // A chain of containers, each nesting the next, so a naive
        // top-down per-container walk would redo work at every level —
        // `check_states()` must still agree with `container_check_state`.
        let mut bytes = std::fs::read(make_zip(&[("leaf.log", b"leaf")]).path()).unwrap();
        for _ in 0..5 {
            let wrapper = make_zip(&[("wrapped.zip", bytes.as_slice())]);
            bytes = std::fs::read(wrapper.path()).unwrap();
        }
        let outer_tmp = make_zip(&[("wrapped.zip", bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let tree = list_archive_tree(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let states = tree.check_states();
        assert_eq!(states.len(), tree.nodes.len());
        for (id, &state) in states.iter().enumerate() {
            assert_eq!(state, tree.container_check_state(id), "node {id}");
        }
    }

    #[test]
    fn test_toggle_subtree_file_flips_only_itself() {
        let mut tree = build_test_tree();
        tree.toggle_subtree(0);
        assert!(tree.nodes[0].selected);
        tree.toggle_subtree(0);
        assert!(!tree.nodes[0].selected);
    }

    #[test]
    fn test_toggle_subtree_container_selects_all_descendants() {
        let mut tree = build_test_tree();
        tree.toggle_subtree(1);
        assert!(tree.nodes[2].selected);
        assert!(tree.nodes[3].selected);
    }

    #[test]
    fn test_toggle_subtree_container_deselects_all_when_fully_selected() {
        let mut tree = build_test_tree();
        tree.nodes[2].selected = true;
        tree.nodes[3].selected = true;
        tree.toggle_subtree(1);
        assert!(!tree.nodes[2].selected);
        assert!(!tree.nodes[3].selected);
    }

    #[test]
    fn test_toggle_subtree_partial_container_selects_all_descendants() {
        // Toggling a `[~]` (partial) container should select everything,
        // not deselect — only a fully-`[x]` container deselects on toggle.
        let mut tree = build_test_tree();
        tree.nodes[2].selected = true;
        tree.toggle_subtree(1);
        assert!(tree.nodes[2].selected);
        assert!(tree.nodes[3].selected);
    }

    #[test]
    fn test_toggle_subtree_descendant_does_not_affect_sibling_subtree() {
        let mut tree = build_test_tree();
        tree.toggle_subtree(2); // select "inner1.log" under container 1
        assert!(tree.nodes[2].selected);
        assert!(
            !tree.nodes[3].selected,
            "sibling under the same container must be untouched"
        );
        assert!(
            !tree.nodes[5].selected,
            "unrelated container's children must be untouched"
        );
        assert!(
            !tree.nodes[6].selected,
            "unrelated container's children must be untouched"
        );
        assert_eq!(tree.container_check_state(4), CheckState::Unchecked);
    }

    #[test]
    fn test_toggle_merge_subtree_file_marks_only_itself() {
        let mut tree = build_test_tree();
        tree.toggle_merge_subtree(0);
        assert!(tree.nodes[0].merge_marked);
        tree.toggle_merge_subtree(0);
        assert!(!tree.nodes[0].merge_marked);
    }

    #[test]
    fn test_toggle_merge_subtree_container_marks_all_descendants() {
        let mut tree = build_test_tree();
        tree.toggle_merge_subtree(1);
        assert!(tree.nodes[2].merge_marked);
        assert!(tree.nodes[3].merge_marked);
    }

    #[test]
    fn test_merge_container_check_state_partial_when_some_descendants_marked() {
        let mut tree = build_test_tree();
        tree.nodes[2].merge_marked = true;
        assert_eq!(tree.merge_container_check_state(1), CheckState::Partial);
    }

    #[test]
    fn test_merge_check_states_matches_merge_container_check_state_for_every_node() {
        for setup in [Vec::<usize>::new(), vec![2], vec![2, 3], vec![2, 3, 5, 6]] {
            let mut tree = build_test_tree();
            for &idx in &setup {
                tree.nodes[idx].merge_marked = true;
            }
            let states = tree.merge_check_states();
            assert_eq!(states.len(), tree.nodes.len());
            for (id, &state) in states.iter().enumerate() {
                assert_eq!(
                    state,
                    tree.merge_container_check_state(id),
                    "node {id} mismatched for selection {setup:?}"
                );
            }
        }
    }

    #[test]
    fn test_selected_and_merge_marked_are_independent_flags() {
        let mut tree = build_test_tree();
        tree.toggle_subtree(0);
        assert!(tree.nodes[0].selected);
        assert!(
            !tree.nodes[0].merge_marked,
            "toggling selected must not affect merge_marked"
        );

        tree.toggle_merge_subtree(2);
        assert!(tree.nodes[2].merge_marked);
        assert!(
            !tree.nodes[2].selected,
            "toggling merge_marked must not affect selected"
        );
    }

    #[test]
    fn test_any_file_selected_true_only_when_a_file_is_selected() {
        let mut tree = build_test_tree();
        assert!(!tree.any_file_selected());
        tree.nodes[0].selected = true;
        assert!(tree.any_file_selected());
    }

    #[test]
    fn test_any_file_merge_marked_true_only_when_a_file_is_marked() {
        let mut tree = build_test_tree();
        assert!(!tree.any_file_merge_marked());
        tree.nodes[2].merge_marked = true;
        assert!(tree.any_file_merge_marked());
    }

    #[test]
    fn test_extract_selected_ignores_merge_marked_only_files() {
        let tmp = make_zip(&[("a.log", b"hello"), ("b.log", b"world")]);
        let path = path_with_ext(&tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        // Mark "a.log" for merge only — not selected for extraction.
        let a_id = tree
            .nodes
            .iter()
            .find(|n| n.full_path == "a.log")
            .unwrap()
            .id;
        tree.nodes[a_id].merge_marked = true;

        let extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert!(
            extracted.is_empty(),
            "a merge-marked-only file must not be extracted by extract_selected"
        );
    }

    #[test]
    fn test_extract_and_detect_merge_marked_ignores_selected_only_files() {
        let tmp = make_zip(&[("a.log", b"hello"), ("b.log", b"world")]);
        let path = path_with_ext(&tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        let a_id = tree
            .nodes
            .iter()
            .find(|n| n.full_path == "a.log")
            .unwrap()
            .id;
        tree.nodes[a_id].selected = true;

        let merge_sources = extract_and_detect_merge_marked(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert!(
            merge_sources.is_empty(),
            "a selected-only file must not be extracted by extract_and_detect_merge_marked"
        );
    }

    #[test]
    fn test_extract_and_detect_merge_marked_returns_detected_format_per_file() {
        let recognized =
            b"{\"timestamp\":\"2024-01-01T00:00:00Z\",\"level\":\"INFO\",\"msg\":\"hello\"}\n"
                .to_vec();
        let unrecognized = b"just some random bytes with no structure\n".to_vec();
        let tmp = make_zip(&[
            ("recognized.log", recognized.as_slice()),
            ("unrecognized.log", unrecognized.as_slice()),
        ]);
        let path = path_with_ext(&tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        for node in &mut tree.nodes {
            if matches!(node.kind, NodeKind::File) {
                node.merge_marked = true;
            }
        }

        let sources = extract_and_detect_merge_marked(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(sources.len(), 2);
        let recognized_src = sources
            .iter()
            .find(|s| s.label.contains("recognized") && !s.label.contains("unrecognized"))
            .unwrap();
        assert!(recognized_src.detected.format.is_some());
        let unrecognized_src = sources
            .iter()
            .find(|s| s.label.contains("unrecognized"))
            .unwrap();
        assert!(unrecognized_src.detected.format.is_none());
    }

    #[test]
    fn test_extract_selected_single_top_level_file() {
        let tmp = make_zip(&[("a.log", b"hello"), ("b.log", b"world")]);
        let path = path_with_ext(&tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        select_by_full_path(&mut tree, "a.log");

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "a.log");
        assert_eq!(read_extracted(&mut extracted[0]), "hello");
    }

    #[test]
    fn test_extract_selected_excludes_unselected_siblings() {
        let tmp = make_zip(&[("a.log", b"hello"), ("b.log", b"world"), ("c.log", b"!")]);
        let path = path_with_ext(&tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        select_by_full_path(&mut tree, "a.log");
        select_by_full_path(&mut tree, "c.log");

        let extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        let mut names: Vec<&str> = extracted.iter().map(|f| f.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["a.log", "c.log"]);
    }

    #[test]
    fn test_extract_selected_two_levels_deep_uses_cached_bytes() {
        // TarGz caches each of its own entries' raw bytes during listing —
        // for a nested zip entry, that means the *container's* bytes are
        // cached (the leaf inside it is then resolved from those cached
        // bytes via a cheap zip lookup, not a second tar.gz decompression).
        let inner = make_zip(&[("a.log", b"inner-a")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_tar_gz(&[("nested/inner.zip", inner_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".tar.gz");
        let mut tree = list_archive_tree(&path).unwrap();

        let leaf_id = select_by_full_path(&mut tree, "a.log");
        assert!(
            tree.nodes[leaf_id].cached_bytes.is_none(),
            "the leaf itself isn't cached — only its containing tar.gz entry is"
        );
        let container_id = tree.nodes[leaf_id].parent.unwrap();
        assert!(
            tree.nodes[container_id].cached_bytes.is_some(),
            "precondition: TarGz entries (including nested-archive containers) are cached by default"
        );

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(extracted.len(), 1);
        assert_eq!(read_extracted(&mut extracted[0]), "inner-a");
    }

    #[test]
    fn test_extract_selected_three_levels_deep_without_cache_falls_back_to_redecode() {
        let innermost = make_zip(&[("a.log", b"deep-a")]);
        let innermost_bytes = std::fs::read(innermost.path()).unwrap();
        let middle = make_zip(&[("mid.zip", innermost_bytes.as_slice())]);
        let middle_bytes = std::fs::read(middle.path()).unwrap();
        let outer_tmp = make_zip(&[("outer.zip", middle_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");

        // Zip never populates cached_bytes (see test_zip_source_never_caches_entry_bytes),
        // so listing this three-level-deep zip-in-zip-in-zip forces every level of
        // `extract_selected` to re-open/re-decode from `path` on demand. Uses a
        // raised `auto_expand_depth` to keep this fully eager (unrelated to
        // laziness — this test is about `resolve_node_bytes`'s multi-level walk).
        let limits = ListLimits {
            auto_expand_depth: 2,
            ..ListLimits::default()
        };
        let mut tree = list_archive_tree_with_limits(&path, &limits).unwrap();
        let leaf_id = select_by_full_path(&mut tree, "a.log");
        assert!(tree.nodes[leaf_id].cached_bytes.is_none());
        assert_eq!(tree.nodes[leaf_id].depth, 2);

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(extracted.len(), 1);
        assert_eq!(read_extracted(&mut extracted[0]), "deep-a");
    }

    #[test]
    fn test_extract_selected_container_toggle_produces_every_descendant_file() {
        let inner = make_zip(&[("a.log", b"one"), ("b.log", b"two")]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_zip(&[("bundle.zip", inner_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();

        let bundle_id = find_node(&tree, "bundle.zip").id;
        tree.toggle_subtree(bundle_id);

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();
        extracted.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].name, "a.log");
        assert_eq!(read_extracted(&mut extracted[0]), "one");
        assert_eq!(extracted[1].name, "b.log");
        assert_eq!(read_extracted(&mut extracted[1]), "two");
    }

    #[test]
    fn test_extract_selected_disambiguates_colliding_basenames() {
        let inner_a = make_zip(&[("app.log", b"from-a")]);
        let inner_a_bytes = std::fs::read(inner_a.path()).unwrap();
        let inner_b = make_zip(&[("app.log", b"from-b")]);
        let inner_b_bytes = std::fs::read(inner_b.path()).unwrap();
        let outer_tmp = make_zip(&[
            ("a.zip", inner_a_bytes.as_slice()),
            ("b.zip", inner_b_bytes.as_slice()),
        ]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        for id in tree
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::File))
            .map(|n| n.id)
            .collect::<Vec<_>>()
        {
            tree.nodes[id].selected = true;
        }

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();
        extracted.sort_by(|a, b| a.name.cmp(&b.name));

        let mut names: Vec<&str> = extracted.iter().map(|f| f.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["app.log", "app.log (b.zip)"]);
    }

    #[test]
    fn test_extract_selected_lone_gz_at_zip_top_level_is_decompressed() {
        // "app.log.gz" inside the zip is a lone single-file compressed entry
        // (not a multi-entry archive), so listing shows it as a plain File
        // leaf rather than expanding it — but its bytes as stored in the zip
        // are still gzip-compressed. Extracting it must yield the original
        // decompressed text, the same as if it had never been compressed.
        let gz = make_gz(b"decompressed content");
        let gz_bytes = std::fs::read(gz.path()).unwrap();
        let outer_tmp = make_zip(&[("app.log.gz", gz_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        select_by_full_path(&mut tree, "app.log.gz");

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(extracted.len(), 1);
        assert_eq!(read_extracted(&mut extracted[0]), "decompressed content");
    }

    #[test]
    fn test_extract_selected_lone_gz_two_levels_deep_is_decompressed() {
        // Same shape as above, but "app.log.gz" is inside an inner zip that's
        // itself nested inside an outer zip — exercising the resolve path
        // that walks down through a real `Container` ancestor (`Some(parent_id)`
        // in `resolve_node_bytes`), not just the top-level-archive path.
        let gz = make_gz(b"deeply nested content");
        let gz_bytes = std::fs::read(gz.path()).unwrap();
        let inner = make_zip(&[("app.log.gz", gz_bytes.as_slice())]);
        let inner_bytes = std::fs::read(inner.path()).unwrap();
        let outer_tmp = make_zip(&[("inner.zip", inner_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        let leaf_id = select_by_full_path(&mut tree, "app.log.gz");
        assert!(
            tree.nodes[leaf_id].parent.is_some(),
            "precondition: the leaf must have a real Container ancestor"
        );

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "app.log");
        assert_eq!(read_extracted(&mut extracted[0]), "deeply nested content");
    }

    #[test]
    fn test_extract_selected_nested_lone_gz_strips_extension_from_name() {
        let gz = make_gz(b"hello");
        let gz_bytes = std::fs::read(gz.path()).unwrap();
        let outer_tmp = make_zip(&[("app.log.gz", gz_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        select_by_full_path(&mut tree, "app.log.gz");

        let extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(extracted[0].name, "app.log");
    }

    #[test]
    fn test_extract_selected_nested_lone_bz2_and_xz_are_decompressed() {
        let bz2 = make_bz2(b"bz2 content");
        let bz2_bytes = std::fs::read(bz2.path()).unwrap();
        let xz = make_xz(b"xz content");
        let xz_bytes = std::fs::read(xz.path()).unwrap();
        let outer_tmp = make_zip(&[
            ("a.log.bz2", bz2_bytes.as_slice()),
            ("b.log.xz", xz_bytes.as_slice()),
        ]);
        let path = path_with_ext(&outer_tmp, ".zip");
        let mut tree = list_archive_tree(&path).unwrap();
        select_by_full_path(&mut tree, "a.log.bz2");
        select_by_full_path(&mut tree, "b.log.xz");

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();
        extracted.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(extracted[0].name, "a.log");
        assert_eq!(read_extracted(&mut extracted[0]), "bz2 content");
        assert_eq!(extracted[1].name, "b.log");
        assert_eq!(read_extracted(&mut extracted[1]), "xz content");
    }

    #[test]
    fn test_extract_selected_nested_lone_gz_via_cached_bytes_is_decompressed() {
        // Nested inside a TarGz stream, "app.log.gz"'s raw (still-compressed)
        // bytes get cached during listing for reuse at extraction time — the
        // decompression must still apply on that fast path too, not just the
        // fresh-read path exercised by the zip-based tests above.
        let gz = make_gz(b"cached path content");
        let gz_bytes = std::fs::read(gz.path()).unwrap();
        let outer_tmp = make_tar_gz(&[("app.log.gz", gz_bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".tar.gz");
        let mut tree = list_archive_tree(&path).unwrap();
        let leaf_id = select_by_full_path(&mut tree, "app.log.gz");
        assert!(
            tree.nodes[leaf_id].cached_bytes.is_some(),
            "precondition: TarGz entries are cached by default"
        );

        let mut extracted = extract_selected(&path, &tree, no_progress()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "app.log");
        assert_eq!(read_extracted(&mut extracted[0]), "cached path content");
    }
}
