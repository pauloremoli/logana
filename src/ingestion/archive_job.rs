//! Bounded, cooperative archive I/O for interactive GUI operations.
use super::{
    ArchiveType,
    archive_tree::{ArchiveTree, NodeId, NodeKind},
};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub struct Progress {
    pub phase: &'static str,
    pub entry: String,
    pub bytes: u64,
    pub total_bytes: Option<u64>,
    pub fraction: Option<f64>,
    pub completed: usize,
    pub total_files: usize,
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            phase: "Queued",
            entry: String::new(),
            bytes: 0,
            total_bytes: None,
            fraction: None,
            completed: 0,
            total_files: 0,
        }
    }
}
#[derive(Clone)]
pub struct Control {
    pub cancel: Arc<AtomicBool>,
    pub progress: watch::Sender<Progress>,
}
impl Control {
    pub fn new() -> (Self, watch::Receiver<Progress>) {
        let (progress, rx) = watch::channel(Progress::default());
        (
            Self {
                cancel: Arc::new(AtomicBool::new(false)),
                progress,
            },
            rx,
        )
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn check(&self) -> io::Result<()> {
        // io::copy/read_exact retry Interrupted, so cancellation uses Other.
        if self.cancelled() {
            Err(io::Error::other("Operation canceled"))
        } else {
            Ok(())
        }
    }
    pub fn phase(&self, phase: &'static str, entry: String, total_bytes: Option<u64>) {
        if self.cancelled() && phase != "Canceling" {
            return;
        }
        self.progress.send_modify(|p| {
            p.phase = phase;
            p.entry = entry;
            p.bytes = 0;
            p.total_bytes = total_bytes;
            p.fraction = None;
        });
    }
    pub fn bytes(&self, bytes: u64) {
        self.progress.send_modify(|p| {
            p.bytes = bytes;
            p.fraction = p
                .total_bytes
                .filter(|&n| n > 0)
                .map(|n| (bytes as f64 / n as f64).min(1.));
        });
    }
}
/// Caps each read, including tar's implicit skipping of unselected entries.
pub(crate) struct CheckedReader<R> {
    inner: R,
    control: Option<Control>,
    bytes: u64,
}
impl<R> CheckedReader<R> {
    pub fn new(inner: R, control: Option<Control>) -> Self {
        Self {
            inner,
            control,
            bytes: 0,
        }
    }
}
impl<R: Read> Read for CheckedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(c) = &self.control {
            c.check()?;
        }
        let len = buf.len().min(64 * 1024);
        let n = self.inner.read(&mut buf[..len])?;
        self.bytes += n as u64;
        if let Some(c) = &self.control {
            c.bytes(self.bytes);
            c.check()?;
        }
        Ok(n)
    }
}
impl<R: Seek> Seek for CheckedReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if let Some(c) = &self.control {
            c.check()?;
        }
        self.inner.seek(pos)
    }
}
fn copy(reader: impl Read, output: &mut File, control: &Control) -> io::Result<()> {
    io::copy(
        &mut CheckedReader::new(reader, Some(control.clone())),
        output,
    )?;
    control.check()
}
fn archive_suffix(name: &str) -> &'static str {
    match super::detect_archive_type(name) {
        Some(ArchiveType::Zip) => ".zip",
        Some(ArchiveType::Tar) => ".tar",
        Some(ArchiveType::TarGz) => ".tar.gz",
        Some(ArchiveType::TarBz2) => ".tar.bz2",
        Some(ArchiveType::TarXz) => ".tar.xz",
        Some(ArchiveType::Gz) => ".gz",
        Some(ArchiveType::Bz2) => ".bz2",
        Some(ArchiveType::Xz) => ".xz",
        None => ".log",
    }
}
fn unpack(
    source: &Path,
    kind: &ArchiveType,
    entry_name: &str,
    control: &Control,
) -> Result<tempfile::NamedTempFile, String> {
    control.check().map_err(|e| e.to_string())?;
    let mut output = tempfile::Builder::new()
        .suffix(archive_suffix(entry_name))
        .tempfile()
        .map_err(|e| e.to_string())?;
    let file = CheckedReader::new(
        File::open(source).map_err(|e| e.to_string())?,
        Some(control.clone()),
    );
    let result = match kind {
        ArchiveType::Zip => {
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            let entry = zip.by_name(entry_name).map_err(|e| e.to_string())?;
            copy(entry, output.as_file_mut(), control)
        }
        ArchiveType::Gz => copy(
            flate2::read::GzDecoder::new(file),
            output.as_file_mut(),
            control,
        ),
        ArchiveType::Bz2 => copy(
            bzip2::read::BzDecoder::new(file),
            output.as_file_mut(),
            control,
        ),
        ArchiveType::Xz => copy(
            xz2::read::XzDecoder::new(file),
            output.as_file_mut(),
            control,
        ),
        _ => {
            let reader: Box<dyn Read> = match kind {
                ArchiveType::Tar => Box::new(file),
                ArchiveType::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
                ArchiveType::TarBz2 => Box::new(bzip2::read::BzDecoder::new(file)),
                ArchiveType::TarXz => Box::new(xz2::read::XzDecoder::new(file)),
                _ => unreachable!(),
            };
            let mut archive = tar::Archive::new(CheckedReader::new(reader, Some(control.clone())));
            let mut found = false;
            for entry in archive.entries().map_err(|e| e.to_string())? {
                control.check().map_err(|e| e.to_string())?;
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.header().entry_type() == tar::EntryType::Regular
                    && entry.path().map_err(|e| e.to_string())?.to_string_lossy() == entry_name
                {
                    copy(entry, output.as_file_mut(), control).map_err(|e| e.to_string())?;
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(format!("Archive entry not found: {entry_name}"));
            }
            Ok(())
        }
    };
    result.map_err(|e| e.to_string())?;
    Ok(output)
}
/// Spool nested containers to disk instead of materializing multi-gigabyte entries.
pub fn extract_node(
    source: &Path,
    tree: &ArchiveTree,
    id: NodeId,
    control: &Control,
) -> Result<tempfile::NamedTempFile, String> {
    let mut chain = vec![id];
    while let Some(parent) = tree.nodes[*chain.last().unwrap()].parent {
        chain.push(parent);
    }
    chain.reverse();
    let mut kind =
        super::detect_archive_type(&source.to_string_lossy()).ok_or("Unsupported archive")?;
    let mut current = source.to_path_buf();
    let mut spool = None;
    // Used only for lazy expansion; selected extraction below walks each container once.
    for node_id in chain {
        let node = &tree.nodes[node_id];
        let next = unpack(&current, &kind, &node.full_path, control)?;
        current = next.path().to_path_buf();
        spool = Some(next);
        if let NodeKind::Container {
            archive_type: Some(t),
            ..
        }
        | NodeKind::LazyContainer { archive_type: t } = &node.kind
        {
            kind = t.clone();
        }
    }
    let mut result = spool.ok_or("Missing entry")?;
    if let Some(kind @ (ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz)) =
        super::detect_archive_type(&tree.nodes[id].name)
    {
        // A lone compressed root was already decoded by unpack.
        if tree.nodes[id].parent.is_some()
            || !matches!(
                super::detect_archive_type(&source.to_string_lossy()),
                Some(ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz)
            )
        {
            result = unpack(result.path(), &kind, "", control)?;
        }
    }
    Ok(result)
}
pub fn expand(
    source: &Path,
    mut tree: ArchiveTree,
    id: NodeId,
    control: Control,
) -> Result<ArchiveTree, String> {
    let kind = match &tree.nodes[id].kind {
        NodeKind::LazyContainer { archive_type } => archive_type.clone(),
        _ => return Ok(tree),
    };
    let offset = tree.nodes.len();
    let nested = (|| {
        let spool = extract_node(source, &tree, id, &control)?;
        let nested = super::archive_tree::list_archive_tree_controlled(
            &spool.path().to_string_lossy(),
            control.clone(),
        )?;
        if offset + nested.nodes.len() > super::archive_tree::MAX_TOTAL_ENTRIES {
            return Err("Archive entry limit reached".to_string());
        }
        Ok(nested)
    })();
    control.check().map_err(|e| e.to_string())?;
    let mut nested = match nested {
        Ok(tree) => tree,
        Err(error) => {
            tree.mark_unreadable(id, error);
            return Ok(tree);
        }
    };
    let depth = tree.nodes[id].depth + 1;
    for node in &mut nested.nodes {
        node.id += offset;
        node.parent = Some(node.parent.map(|p| p + offset).unwrap_or(id));
        node.depth += depth;
        if let NodeKind::Container { children, .. } = &mut node.kind {
            for child in children {
                *child += offset;
            }
        }
    }
    tree.nodes[id].kind = NodeKind::Container {
        children: nested.roots.iter().map(|p| p + offset).collect(),
        archive_type: Some(kind),
    };
    tree.nodes[id].collapsed = false;
    tree.nodes.extend(nested.nodes);
    Ok(tree)
}

/// Walk selected containers once, emitting each completed entry before continuing.
/// The consumer can apply backpressure while it indexes and publishes that entry.
pub fn extract_selection(
    source: &Path,
    tree: &ArchiveTree,
    control: &Control,
    emit: &mut dyn FnMut(Result<(NodeId, tempfile::NamedTempFile), String>) -> Result<(), String>,
) -> Result<(), String> {
    let mut wanted = vec![false; tree.nodes.len()];
    for node in tree.nodes.iter().rev() {
        wanted[node.id] |= node.selected && matches!(node.kind, NodeKind::File);
        if wanted[node.id]
            && let Some(parent) = node.parent
        {
            wanted[parent] = true;
        }
    }
    let kind =
        super::detect_archive_type(&source.to_string_lossy()).ok_or("Unsupported archive")?;
    walk_selection(source, &kind, &tree.roots, tree, &wanted, control, emit)
}
fn walk_selection(
    source: &Path,
    kind: &ArchiveType,
    children: &[NodeId],
    tree: &ArchiveTree,
    wanted: &[bool],
    control: &Control,
    emit: &mut dyn FnMut(Result<(NodeId, tempfile::NamedTempFile), String>) -> Result<(), String>,
) -> Result<(), String> {
    control.check().map_err(|e| e.to_string())?;
    let selected: Vec<_> = children.iter().copied().filter(|&id| wanted[id]).collect();
    if selected.is_empty() {
        return Ok(());
    }
    if matches!(kind, ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz) {
        let id = selected[0];
        control.phase("Extracting", tree.nodes[id].name.clone(), None);
        return emit(unpack(source, kind, "", control).map(|file| (id, file)));
    }
    if matches!(kind, ArchiveType::Zip) {
        let file = CheckedReader::new(
            File::open(source).map_err(|e| e.to_string())?,
            Some(control.clone()),
        );
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        for id in selected {
            control.check().map_err(|e| e.to_string())?;
            control.phase("Extracting", tree.nodes[id].full_path.clone(), None);
            let result = (|| {
                let entry = archive
                    .by_name(&tree.nodes[id].full_path)
                    .map_err(|e| e.to_string())?;
                let mut output = tempfile::Builder::new()
                    .suffix(archive_suffix(&tree.nodes[id].name))
                    .tempfile()
                    .map_err(|e| e.to_string())?;
                copy(entry, output.as_file_mut(), control).map_err(|e| e.to_string())?;
                Ok::<_, String>(output)
            })();
            match result {
                Ok(file) => finish_entry(file, id, tree, wanted, control, emit)?,
                Err(error) => {
                    control.check().map_err(|e| e.to_string())?;
                    emit(Err(format!("{}: {error}", tree.nodes[id].full_path)))?;
                }
            }
        }
        return Ok(());
    }
    let by_name: std::collections::HashMap<_, _> = selected
        .iter()
        .map(|&id| (tree.nodes[id].full_path.as_str(), id))
        .collect();
    let file = CheckedReader::new(
        File::open(source).map_err(|e| e.to_string())?,
        Some(control.clone()),
    );
    let reader: Box<dyn Read> = match kind {
        ArchiveType::Tar => Box::new(file),
        ArchiveType::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
        ArchiveType::TarBz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        ArchiveType::TarXz => Box::new(xz2::read::XzDecoder::new(file)),
        _ => unreachable!(),
    };
    let mut archive = tar::Archive::new(CheckedReader::new(reader, Some(control.clone())));
    let mut found = std::collections::HashSet::new();
    for entry in archive.entries().map_err(|e| e.to_string())? {
        control.check().map_err(|e| e.to_string())?;
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let Some(&id) = by_name.get(name.as_str()) else {
            continue;
        };
        if !found.insert(id) {
            continue;
        }
        control.phase("Extracting", name, None);
        let mut output = tempfile::Builder::new()
            .suffix(archive_suffix(&tree.nodes[id].name))
            .tempfile()
            .map_err(|e| e.to_string())?;
        match copy(entry, output.as_file_mut(), control) {
            Ok(()) => finish_entry(output, id, tree, wanted, control, emit)?,
            Err(error) => {
                control.check().map_err(|e| e.to_string())?;
                emit(Err(format!("{}: {error}", tree.nodes[id].full_path)))?;
            }
        }
        if found.len() == by_name.len() {
            break;
        }
    }
    for id in selected {
        if !found.contains(&id) {
            emit(Err(format!(
                "Archive entry not found: {}",
                tree.nodes[id].full_path
            )))?;
        }
    }
    Ok(())
}
fn finish_entry(
    mut file: tempfile::NamedTempFile,
    id: NodeId,
    tree: &ArchiveTree,
    wanted: &[bool],
    control: &Control,
    emit: &mut dyn FnMut(Result<(NodeId, tempfile::NamedTempFile), String>) -> Result<(), String>,
) -> Result<(), String> {
    if let NodeKind::Container {
        children,
        archive_type: Some(kind),
    } = &tree.nodes[id].kind
    {
        if let Err(error) = walk_selection(file.path(), kind, children, tree, wanted, control, emit)
        {
            control.check().map_err(|e| e.to_string())?;
            emit(Err(format!("{}: {error}", tree.nodes[id].full_path)))?;
        }
        return Ok(());
    }
    if let Some(kind @ (ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz)) =
        super::detect_archive_type(&tree.nodes[id].name)
    {
        match unpack(file.path(), &kind, "", control) {
            Ok(decoded) => file = decoded,
            Err(error) => {
                control.check().map_err(|e| e.to_string())?;
                return emit(Err(format!("{}: {error}", tree.nodes[id].full_path)));
            }
        }
    }
    emit(Ok((id, file)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::archive::test_helpers::{
        make_bz2, make_gz, make_tar, make_tar_bz2, make_tar_gz, make_tar_xz, make_xz, make_zip,
    };
    fn archive(bytes: &tempfile::NamedTempFile, suffix: &str) -> tempfile::NamedTempFile {
        let file = tempfile::Builder::new().suffix(suffix).tempfile().unwrap();
        std::fs::copy(bytes.path(), file.path()).unwrap();
        file
    }
    #[test]
    fn selected_entries_stream_for_every_supported_format() {
        let content = b"first\nsecond\n";
        let zip = archive(
            &make_zip(&[("logs/a.log", content), ("skip.log", b"skip")]),
            ".zip",
        );
        let tar = archive(
            &make_tar(&[("logs/a.log", content), ("skip.log", b"skip")]),
            ".tar",
        );
        let tar_gz = archive(
            &make_tar_gz(&[("logs/a.log", content), ("skip.log", b"skip")]),
            ".tar.gz",
        );
        let gz = archive(&make_gz(content), ".log.gz");
        let bz2 = archive(&make_bz2(content), ".log.bz2");
        let xz = archive(&make_xz(content), ".log.xz");
        let tar_bz2 = archive(
            &make_tar_bz2(&[("logs/a.log", content), ("skip.log", b"skip")]),
            ".tar.bz2",
        );
        let tar_xz = archive(
            &make_tar_xz(&[("logs/a.log", content), ("skip.log", b"skip")]),
            ".tar.xz",
        );
        for file in [&zip, &tar, &tar_gz, &gz, &bz2, &xz, &tar_bz2, &tar_xz] {
            let (control, rx) = Control::new();
            let mut tree = super::super::archive_tree::list_archive_tree_controlled(
                &file.path().to_string_lossy(),
                control.clone(),
            )
            .unwrap();
            assert!(tree.nodes.iter().all(|n| n.cached_bytes.is_none()));
            tree.nodes[0].selected = true;
            let mut outputs = Vec::new();
            extract_selection(file.path(), &tree, &control, &mut |result| {
                outputs.push(result.unwrap().1);
                Ok(())
            })
            .unwrap();
            assert_eq!(outputs.len(), 1);
            assert_eq!(std::fs::read(outputs[0].path()).unwrap(), content);
            assert!(rx.borrow().bytes > 0);
        }
    }
    #[test]
    fn nested_archives_expand_on_disk_and_selected_leaves_decode() {
        let compressed = make_gz(b"nested data\n");
        let nested = make_zip(&[("a.log.gz", &std::fs::read(compressed.path()).unwrap())]);
        let root = archive(
            &make_tar_gz(&[("inside.zip", &std::fs::read(nested.path()).unwrap())]),
            ".tar.gz",
        );
        let (control, _) = Control::new();
        let tree = super::super::archive_tree::list_archive_tree_controlled(
            &root.path().to_string_lossy(),
            control.clone(),
        )
        .unwrap();
        assert!(matches!(tree.nodes[0].kind, NodeKind::LazyContainer { .. }));
        let mut tree = expand(root.path(), tree, 0, control.clone()).unwrap();
        let leaf = tree
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::File))
            .unwrap()
            .id;
        tree.nodes[leaf].selected = true;
        let mut output = None;
        extract_selection(root.path(), &tree, &control, &mut |result| {
            output = Some(result.unwrap().1);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::read(output.unwrap().path()).unwrap(),
            b"nested data\n"
        );
    }
    #[test]
    fn failed_nested_expansion_preserves_sibling_selection() {
        let root = archive(
            &make_zip(&[("bad.zip", b"broken"), ("good.log", b"good")]),
            ".zip",
        );
        let (control, _) = Control::new();
        let mut tree = super::super::archive_tree::list_archive_tree_controlled(
            &root.path().to_string_lossy(),
            control.clone(),
        )
        .unwrap();
        let good = tree.nodes.iter().find(|n| n.name == "good.log").unwrap().id;
        let bad = tree.nodes.iter().find(|n| n.name == "bad.zip").unwrap().id;
        tree.nodes[good].selected = true;
        let tree = expand(root.path(), tree, bad, control).unwrap();
        assert!(tree.nodes[good].selected);
        assert!(matches!(
            tree.nodes[bad].kind,
            NodeKind::UnreadableContainer { .. }
        ));
    }
    #[test]
    fn cancellation_keeps_completed_output_and_stops_remaining_entries() {
        let root = archive(
            &make_tar_gz(&[("a.log", b"complete\n"), ("b.log", b"unopened\n")]),
            ".tar.gz",
        );
        let before = std::fs::read(root.path()).unwrap();
        let (control, _) = Control::new();
        let mut tree = super::super::archive_tree::list_archive_tree_controlled(
            &root.path().to_string_lossy(),
            control.clone(),
        )
        .unwrap();
        tree.set_all_files_selected(true);
        let mut outputs = Vec::new();
        let result = extract_selection(root.path(), &tree, &control, &mut |result| {
            outputs.push(result.unwrap().1);
            control.cancel.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(outputs.len(), 1);
        assert_eq!(std::fs::read(outputs[0].path()).unwrap(), b"complete\n");
        assert_eq!(std::fs::read(root.path()).unwrap(), before);
        let path = outputs[0].path().to_path_buf();
        drop(outputs);
        assert!(!path.exists());
    }
    #[test]
    fn listing_and_reads_observe_cancellation_within_a_bounded_chunk() {
        let (control, _) = Control::new();
        struct CancelOnRead(Control);
        impl Read for CancelOnRead {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                assert!(output.len() <= 64 * 1024);
                output.fill(1);
                self.0.cancel.store(true, Ordering::Relaxed);
                Ok(output.len())
            }
        }
        let mut reader = CheckedReader::new(CancelOnRead(control.clone()), Some(control.clone()));
        let error = reader.read(&mut vec![0; 1_000_000]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(
            super::super::archive_tree::list_archive_tree_controlled("missing.zip", control)
                .unwrap_err()
                .contains("canceled")
        );
    }
    #[test]
    fn corrupt_selected_entry_reports_failure_and_continues() {
        let root = archive(
            &make_zip(&[("broken.log.gz", b"invalid gzip"), ("good.log", b"valid\n")]),
            ".zip",
        );
        let (control, _) = Control::new();
        let mut tree = super::super::archive_tree::list_archive_tree_controlled(
            &root.path().to_string_lossy(),
            control.clone(),
        )
        .unwrap();
        tree.set_all_files_selected(true);
        let mut errors = Vec::new();
        let mut outputs = Vec::new();
        extract_selection(root.path(), &tree, &control, &mut |result| {
            match result {
                Ok((_, file)) => outputs.push(file),
                Err(e) => errors.push(e),
            };
            Ok(())
        })
        .unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(outputs.len(), 1);
        assert_eq!(std::fs::read(outputs[0].path()).unwrap(), b"valid\n");
    }
}
