use crate::ingestion::archive_tree::{ArchiveTree, NodeId, NodeKind};
use std::collections::HashSet;

#[derive(Debug)]
pub(crate) enum SearchMatcher {
    MatchAll,
    Regex(regex::Regex),
    Literal(String),
}

impl SearchMatcher {
    pub(crate) fn compile(query: &str) -> Self {
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

    pub(crate) fn is_match(&self, haystack: &str) -> bool {
        match self {
            Self::MatchAll => true,
            Self::Regex(re) => re.is_match(haystack),
            Self::Literal(needle) => haystack.to_lowercase().contains(needle.as_str()),
        }
    }
}

pub(crate) fn filter_rows(
    tree: &ArchiveTree,
    rows: &[NodeId],
    matcher: &SearchMatcher,
    retain_unsearched: bool,
) -> Vec<NodeId> {
    if matches!(matcher, SearchMatcher::MatchAll) {
        return rows.to_vec();
    }
    let mut keep = HashSet::new();
    for &id in rows {
        let node = &tree.nodes[id];
        let unsearched = matches!(
            node.kind,
            NodeKind::LazyContainer { .. } | NodeKind::UnreadableContainer { .. }
        ) || (node.collapsed && matches!(node.kind, NodeKind::Container { .. }));
        if matcher.is_match(&node.name) || (retain_unsearched && unsearched) {
            let mut current = Some(id);
            while let Some(id) = current {
                keep.insert(id);
                current = tree.nodes[id].parent;
            }
        }
    }
    rows.iter()
        .copied()
        .filter(|id| keep.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::ArchiveType;
    use crate::ingestion::archive_tree::ArchiveNode;

    fn node(
        id: usize,
        parent: Option<usize>,
        name: &str,
        kind: NodeKind,
        collapsed: bool,
    ) -> ArchiveNode {
        ArchiveNode {
            id,
            parent,
            name: name.into(),
            full_path: name.into(),
            depth: usize::from(parent.is_some()),
            kind,
            selected: false,
            merge_marked: false,
            cached_bytes: None,
            collapsed,
            disk_path: false,
        }
    }

    #[test]
    fn search_keeps_ancestors_and_optionally_unsearched_containers() {
        let tree = ArchiveTree {
            roots: vec![0, 2, 3, 4],
            nodes: vec![
                node(
                    0,
                    None,
                    "root",
                    NodeKind::Container {
                        children: vec![1],
                        archive_type: None,
                    },
                    false,
                ),
                node(1, Some(0), "value1.log", NodeKind::File, false),
                node(
                    2,
                    None,
                    "bundle.zip",
                    NodeKind::LazyContainer {
                        archive_type: ArchiveType::Zip,
                    },
                    false,
                ),
                node(
                    3,
                    None,
                    "bad.zip",
                    NodeKind::UnreadableContainer {
                        error: "unreadable".into(),
                    },
                    false,
                ),
                node(
                    4,
                    None,
                    "folded",
                    NodeKind::Container {
                        children: vec![],
                        archive_type: None,
                    },
                    true,
                ),
            ],
        };
        let rows = tree.visible_rows();
        let matcher = SearchMatcher::compile("VALUE1");
        assert_eq!(filter_rows(&tree, &rows, &matcher, false), vec![0, 1]);
        assert_eq!(filter_rows(&tree, &rows, &matcher, true), rows);
        assert!(filter_rows(&tree, &rows, &SearchMatcher::compile("no-match"), false).is_empty());
        assert_eq!(
            filter_rows(&tree, &rows, &SearchMatcher::compile(""), false),
            rows
        );
        assert!(SearchMatcher::compile("[").is_match("value[1"));
        assert!(!SearchMatcher::compile("[").is_match("value1"));
    }
}
