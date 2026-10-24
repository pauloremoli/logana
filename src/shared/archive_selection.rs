use crate::ingestion::archive_tree::{ArchiveTree, NodeId, NodeKind};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct RotationGroup {
    pub representative: NodeId,
    pub relative: String,
    pub members: Vec<NodeId>,
}

pub(crate) fn entry_path(tree: &ArchiveTree, id: NodeId) -> String {
    let mut parts = vec![tree.nodes[id].full_path.as_str()];
    let mut parent = tree.nodes[id].parent;
    while let Some(id) = parent {
        parts.push(&tree.nodes[id].full_path);
        parent = tree.nodes[id].parent;
    }
    parts.reverse();
    parts.join("/")
}

pub(crate) fn archive_output_name(name: &str) -> String {
    let Some(kind) = crate::ingestion::detect_archive_type(name) else {
        return name.to_string();
    };
    let path = std::path::Path::new(name);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let stem = if matches!(
        kind,
        crate::ingestion::ArchiveType::TarGz
            | crate::ingestion::ArchiveType::TarBz2
            | crate::ingestion::ArchiveType::TarXz
    ) && stem.to_ascii_lowercase().ends_with(".tar")
    {
        &stem[..stem.len() - 4]
    } else {
        &stem
    };
    path.with_file_name(stem).to_string_lossy().into_owned()
}

pub(crate) fn output_entry_path(tree: &ArchiveTree, id: NodeId) -> String {
    let mut parts = vec![archive_output_name(&tree.nodes[id].full_path)];
    let mut parent = tree.nodes[id].parent;
    while let Some(id) = parent {
        parts.push(archive_output_name(&tree.nodes[id].full_path));
        parent = tree.nodes[id].parent;
    }
    parts.reverse();
    parts.join("/")
}

pub(crate) fn apply_patterns(tree: &mut ArchiveTree, patterns: &[String]) {
    let patterns: Vec<_> = patterns
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();
    let options = glob::MatchOptions {
        require_literal_separator: false,
        ..Default::default()
    };
    for id in 0..tree.nodes.len() {
        if matches!(tree.nodes[id].kind, NodeKind::File) {
            tree.nodes[id].selected = patterns.iter().any(|p| {
                p.matches_with(&tree.nodes[id].name, options)
                    || p.matches_with(&tree.nodes[id].full_path, options)
                    || p.matches_with(&entry_path(tree, id), options)
            });
        }
    }
}

pub(crate) fn rotation_name(path: &str) -> Option<(&str, u64)> {
    let path = path
        .strip_suffix(".gz")
        .or_else(|| path.strip_suffix(".bz2"))
        .or_else(|| path.strip_suffix(".xz"))
        .unwrap_or(path);
    let end = path.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let prefix = &path[..end];
    let base = if end == path.len() {
        prefix
    } else {
        prefix.strip_suffix('.').unwrap_or(prefix)
    };
    if base.is_empty() || base.ends_with('/') {
        return None;
    }
    let rank = if end == path.len() {
        0
    } else {
        path[end..].parse().ok()?
    };
    Some((base, rank))
}

pub(crate) fn rotation_groups(tree: &ArchiveTree) -> Vec<RotationGroup> {
    let mut groups = BTreeMap::<(Option<NodeId>, String), Vec<(u64, NodeId)>>::new();
    for node in &tree.nodes {
        if !node.selected || !matches!(node.kind, NodeKind::File) {
            continue;
        }
        if let Some((base, rank)) = rotation_name(&node.full_path) {
            groups
                .entry((node.parent, base.to_string()))
                .or_default()
                .push((rank, node.id));
        }
    }
    groups
        .into_values()
        .filter_map(|mut items| {
            if items.len() < 2 || !items.iter().any(|(rank, _)| *rank > 0) {
                return None;
            }
            items.sort_by_key(|&(rank, id)| (std::cmp::Reverse(rank), id));
            let representative = items.last().unwrap().1;
            let relative = output_entry_path(tree, representative);
            let (base, _) = rotation_name(&relative)?;
            let path = std::path::Path::new(base);
            let mut name = path.file_stem()?.to_os_string();
            name.push("-merged");
            if let Some(extension) = path.extension() {
                name.push(".");
                name.push(extension);
            }
            Some(RotationGroup {
                representative,
                relative: path.with_file_name(name).to_string_lossy().into_owned(),
                members: items.into_iter().map(|(_, id)| id).collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::archive_tree::ArchiveNode;
    fn tree() -> ArchiveTree {
        ArchiveTree {
            roots: (0..7).collect(),
            nodes: [
                "a/runtime.log",
                "a/runtime.log1",
                "a/runtime.log10",
                "a/startup.log",
                "a/startup.log.2",
                "b/runtime.log1",
                "notes.txt",
            ]
            .into_iter()
            .enumerate()
            .map(|(id, name)| ArchiveNode {
                id,
                parent: None,
                name: name.rsplit('/').next().unwrap().into(),
                full_path: name.into(),
                depth: 0,
                kind: NodeKind::File,
                selected: false,
                merge_marked: false,
                cached_bytes: None,
                collapsed: false,
                disk_path: false,
            })
            .collect(),
        }
    }
    #[test]
    fn archive_output_names_strip_simple_compound_and_alias_extensions() {
        for name in [
            "bundle.zip",
            "bundle.tar.gz",
            "bundle.tar.bz2",
            "bundle.tar.xz",
            "bundle.tgz",
            "bundle.tbz2",
            "bundle.txz",
            "bundle.TAR.GZ",
        ] {
            assert_eq!(archive_output_name(name), "bundle");
        }
        assert_eq!(archive_output_name("logs/nested.zip"), "logs/nested");
        assert_eq!(archive_output_name("runtime.log"), "runtime.log");
        let mut tree = tree();
        tree.nodes[6].full_path = "nested.tar.gz".into();
        tree.nodes[0].parent = Some(6);
        assert_eq!(output_entry_path(&tree, 0), "nested/a/runtime.log");
    }
    #[test]
    fn numbered_rotations_support_other_extensions_and_compression() {
        assert_eq!(
            rotation_name("logs/example.out12.gz"),
            Some(("logs/example.out", 12))
        );
        assert_eq!(rotation_name("logs/messages.2"), Some(("logs/messages", 2)));
        assert_eq!(rotation_name("logs/messages"), Some(("logs/messages", 0)));
    }
    #[test]
    fn compressed_copies_without_numbering_are_not_rotations() {
        let mut tree = tree();
        for node in &mut tree.nodes {
            node.selected = false;
        }
        for (id, name) in [(0, "a/runtime.log"), (1, "a/runtime.log.gz")] {
            tree.nodes[id].selected = true;
            tree.nodes[id].name = name.into();
            tree.nodes[id].full_path = name.into();
        }
        assert!(rotation_groups(&tree).is_empty());
    }
    #[test]
    fn saved_globs_match_names_and_paths_and_can_be_removed() {
        let mut tree = tree();
        apply_patterns(&mut tree, &["runtime.log*".into(), "a/startup.log*".into()]);
        assert_eq!(tree.nodes.iter().filter(|n| n.selected).count(), 6);
        apply_patterns(&mut tree, &["a/startup.log*".into()]);
        assert_eq!(tree.nodes.iter().filter(|n| n.selected).count(), 2);
        apply_patterns(&mut tree, &[]);
        assert!(!tree.any_file_selected());
        tree.nodes[0].parent = Some(6);
        apply_patterns(&mut tree, &["a/runtime.log".into()]);
        assert!(
            tree.nodes[0].selected,
            "relative paths match inside nested archives too"
        );
        assert_eq!(tree.nodes.iter().filter(|n| n.selected).count(), 1);
    }
    #[test]
    fn rotations_use_numeric_oldest_first_order_and_keep_families_and_folders_separate() {
        let mut tree = tree();
        tree.set_all_files_selected(true);
        let groups = rotation_groups(&tree);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].relative, "a/runtime-merged.log");
        assert_eq!(groups[0].members, vec![2, 1, 0]);
        assert_eq!(groups[1].members, vec![4, 3]);
        tree.nodes[1].parent = Some(6);
        tree.nodes[2].parent = Some(6);
        assert_eq!(rotation_groups(&tree)[0].members, vec![4, 3]);
    }
}
