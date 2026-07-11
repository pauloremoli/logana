use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::sync::Arc;

use crate::ingestion::archive::{decompress_to_temp, detect_archive_type, stem};
use crate::ingestion::{ArchiveExtractionProgress, ArchiveType, ExtractedFile};

/// Maximum nesting depth of archives-within-archives before listing stops
/// descending further (a nested entry past this depth is still shown, as a
/// non-expandable row, just not parsed).
pub const MAX_RECURSION_DEPTH: usize = 20;
/// Maximum total entries (across all nesting levels) a single listing pass
/// will walk, to bound pathological/zip-bomb-style archives.
pub const MAX_TOTAL_ENTRIES: usize = 10_000;
/// Maximum cumulative bytes of streaming-source (TarGz/TarBz2/TarXz) entry
/// content retained in memory during listing for reuse at extraction time.
pub const MAX_CACHED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct ListLimits {
    pub max_depth: usize,
    pub max_entries: usize,
    pub max_cached_bytes: u64,
}

impl Default for ListLimits {
    fn default() -> Self {
        Self {
            max_depth: MAX_RECURSION_DEPTH,
            max_entries: MAX_TOTAL_ENTRIES,
            max_cached_bytes: MAX_CACHED_BYTES,
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
    /// Populated only when this entry's bytes were already buffered while
    /// listing a streaming (TarGz/TarBz2/TarXz) source, so extraction can
    /// reuse them instead of decompressing the parent stream a second time.
    pub cached_bytes: Option<Arc<Vec<u8>>>,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    File,
    Container {
        children: Vec<NodeId>,
        archive_type: ArchiveType,
    },
    /// A nested archive that failed to parse, or one that hit the depth/entry
    /// cap — shown as a non-expandable row with an error marker. Listing one
    /// bad nested entry must never abort the whole tree.
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

#[derive(Debug, Clone, Default)]
pub struct ArchiveTree {
    pub nodes: Vec<ArchiveNode>,
    pub roots: Vec<NodeId>,
}

impl ArchiveTree {
    /// Rows in the order they should be rendered: a pre-order depth-first
    /// walk of `roots`. There is no expand/collapse — the tree is always
    /// shown in full.
    pub fn visible_rows(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for &root in &self.roots {
            self.push_preorder(root, &mut out);
        }
        out
    }

    fn push_preorder(&self, id: NodeId, out: &mut Vec<NodeId>) {
        out.push(id);
        if let NodeKind::Container { children, .. } = &self.nodes[id].kind {
            for &child in children {
                self.push_preorder(child, out);
            }
        }
    }

    /// Every `File` descendant of `id` (including `id` itself if it is a
    /// `File`). `UnreadableContainer` subtrees contribute no files, since
    /// they have no children to select.
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
            NodeKind::UnreadableContainer { .. } => {}
        }
    }

    /// A container's checkbox state, derived from its descendant files —
    /// never stored, always computed fresh so it can't drift out of sync.
    pub fn container_check_state(&self, id: NodeId) -> CheckState {
        let files = self.descendant_files(id);
        if files.is_empty() {
            return CheckState::Unchecked;
        }
        let all_selected = files.iter().all(|&fid| self.nodes[fid].selected);
        if all_selected {
            return CheckState::Checked;
        }
        let any_selected = files.iter().any(|&fid| self.nodes[fid].selected);
        if any_selected {
            CheckState::Partial
        } else {
            CheckState::Unchecked
        }
    }

    /// Toggling a `File` row flips just that node. Toggling a `Container`
    /// row is a "select all in this subtree" shortcut: if every descendant
    /// file is already selected, deselect them all; otherwise select them
    /// all. `UnreadableContainer` rows have nothing to toggle.
    pub fn toggle_subtree(&mut self, id: NodeId) {
        match &self.nodes[id].kind {
            NodeKind::File => {
                self.nodes[id].selected = !self.nodes[id].selected;
            }
            NodeKind::Container { .. } => {
                let target = self.container_check_state(id) != CheckState::Checked;
                for fid in self.descendant_files(id) {
                    self.nodes[fid].selected = target;
                }
            }
            NodeKind::UnreadableContainer { .. } => {}
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
        cached_bytes: None,
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
        cached_bytes: None,
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
        cached_bytes: None,
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
            Some(nested_type) if depth < state.limits.max_depth => {
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
            Some(_) => {
                let id = nodes.len();
                state.entry_count += 1;
                nodes.push(unreadable_node(
                    id,
                    parent,
                    name,
                    full_path,
                    depth,
                    "max nesting depth reached",
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
            Some(nested_type) if depth < state.limits.max_depth => {
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
            Some(_) => {
                let id = nodes.len();
                state.entry_count += 1;
                nodes.push(unreadable_node(
                    id,
                    parent,
                    name,
                    full_path,
                    depth,
                    "max nesting depth reached",
                ));
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
pub fn extract_selected(
    path: &str,
    tree: &ArchiveTree,
    progress_tx: tokio::sync::watch::Sender<ArchiveExtractionProgress>,
) -> Result<Vec<ExtractedFile>, String> {
    let selected: Vec<NodeId> = tree
        .nodes
        .iter()
        .filter(|n| n.selected && matches!(n.kind, NodeKind::File))
        .map(|n| n.id)
        .collect();

    let mut used_names: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(selected.len());
    let total = selected.len().max(1);
    for (i, &node_id) in selected.iter().enumerate() {
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

/// A selected file's display name: its basename, or — only once a later
/// selected file collides with an earlier one's basename — the basename
/// suffixed with its immediate containing archive's name, to keep the
/// common case (no collisions) free of visual noise.
fn disambiguated_name(
    tree: &ArchiveTree,
    node_id: NodeId,
    used: &mut HashMap<String, usize>,
) -> String {
    let node = &tree.nodes[node_id];
    let seen_before = used.entry(node.name.clone()).or_insert(0);
    *seen_before += 1;
    if *seen_before == 1 {
        return node.name.clone();
    }
    match node.parent {
        Some(parent_id) => format!("{} ({})", node.name, tree.nodes[parent_id].name),
        None => node.name.clone(),
    }
}

/// Resolves a node's own raw bytes as stored inside its immediate parent —
/// via `cached_bytes` when available, otherwise by walking down from the
/// root, re-opening/re-decoding one archive layer at a time.
fn resolve_node_bytes(tree: &ArchiveTree, node_id: NodeId, path: &str) -> Result<Vec<u8>, String> {
    let node = &tree.nodes[node_id];
    if let Some(cached) = &node.cached_bytes {
        return Ok((**cached).clone());
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
            read_entry_bytes_from_slice(&parent_bytes, parent_archive_type, &node.full_path)
        }
    }
}

fn resolve_root_entry_bytes(path: &str, full_path: &str) -> Result<Vec<u8>, String> {
    let archive_type = detect_archive_type(path)
        .ok_or_else(|| format!("'{}' is not a recognised archive format", path))?;
    match archive_type {
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
    }
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
        make_tar, make_tar_bz2, make_tar_gz, make_tar_xz, make_zip,
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
            cached_bytes: None,
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
            cached_bytes: None,
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
    fn test_depth_cap_truncates_recursion() {
        fn wrap_in_zip(entry_name: &str, bytes: Vec<u8>) -> Vec<u8> {
            let tmp = make_zip(&[(entry_name, bytes.as_slice())]);
            std::fs::read(tmp.path()).unwrap()
        }

        let mut bytes = b"leaf".to_vec();
        for i in (2..=5).rev() {
            bytes = wrap_in_zip(&format!("archive{i}.zip"), bytes);
        }
        let outer_tmp = make_zip(&[("archive1.zip", bytes.as_slice())]);
        let path = path_with_ext(&outer_tmp, ".zip");

        let limits = ListLimits {
            max_depth: 3,
            ..ListLimits::default()
        };
        let tree = list_archive_tree_with_limits(&path, &limits).unwrap();
        std::fs::remove_file(&path).unwrap();

        let archive1 = find_node(&tree, "archive1.zip");
        assert_eq!(archive1.depth, 0);
        assert!(matches!(archive1.kind, NodeKind::Container { .. }));

        let archive4 = find_node(&tree, "archive4.zip");
        assert_eq!(archive4.depth, 3);
        assert!(
            matches!(archive4.kind, NodeKind::UnreadableContainer { .. }),
            "expected archive4.zip (depth 3, at the cap) to be truncated, got {:?}",
            archive4.kind
        );

        assert!(
            tree.nodes.iter().all(|n| n.full_path != "archive5.zip"),
            "listing must not descend past the depth cap"
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
        // `extract_selected` to re-open/re-decode from `path` on demand.
        let mut tree = list_archive_tree(&path).unwrap();
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
}
