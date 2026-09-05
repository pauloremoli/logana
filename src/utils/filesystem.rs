/// List the flat (non-recursive), non-hidden regular files in `path`.
/// Returns absolute path strings sorted by name.
/// Returns an empty Vec for non-existent or unreadable paths.
pub fn list_dir_files(path: &str) -> Vec<String> {
    let dir = match std::fs::read_dir(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut files: Vec<String> = dir
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let fname = entry.file_name();
            let name = fname.to_string_lossy();
            // Skip hidden files (dot-prefixed).
            if name.starts_with('.') {
                return false;
            }
            // Keep regular files only (no dirs, symlinks to dirs, etc.).
            entry.file_type().map(|t| t.is_file()).unwrap_or(false)
        })
        .filter_map(|entry| entry.path().to_str().map(|s| s.to_string()))
        .collect();
    files.sort();
    files
}

/// Lists `path`'s immediate (non-recursive), non-hidden entries, split into
/// regular files and subdirectories — both sorted by name, absolute path
/// strings. Returns two empty `Vec`s for a non-existent or unreadable path.
/// Used by the archive/directory picker's recursive directory listing (see
/// `ingestion::archive_tree::list_directory_entries`), which needs to walk
/// into subdirectories itself rather than have them filtered out the way
/// [`list_dir_files`] does.
pub fn list_dir_entries(path: &str) -> (Vec<String>, Vec<String>) {
    let dir = match std::fs::read_dir(path) {
        Ok(d) => d,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in dir.filter_map(|entry| entry.ok()) {
        let fname = entry.file_name();
        let name = fname.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let Some(entry_path) = entry.path().to_str().map(|s| s.to_string()) else {
            continue;
        };
        if file_type.is_dir() {
            dirs.push(entry_path);
        } else if file_type.is_file() {
            files.push(entry_path);
        }
    }
    files.sort();
    dirs.sort();
    (files, dirs)
}

/// True if `path` contains glob wildcard metacharacters (`*`, `?`, `[`).
pub fn has_glob_metachars(path: &str) -> bool {
    path.contains(['*', '?', '['])
}

/// Expands a glob pattern (e.g. `/var/log/syslog*`) into the sorted list of
/// matching paths. A pattern with no matches returns `Ok(vec![])` — it's up
/// to the caller to decide whether that's an error. Only successfully
/// resolved entries are included (permission-denied entries are skipped).
pub fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    let paths = glob::glob(pattern).map_err(|e| format!("Invalid pattern '{pattern}': {e}"))?;
    let mut matches: Vec<String> = paths
        .filter_map(|entry| entry.ok())
        .filter_map(|p| p.to_str().map(|s| s.to_string()))
        .collect();
    matches.sort();
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_dir_files_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("b.log"), b"b").unwrap();
        std::fs::write(dir.join("a.log"), b"a").unwrap();
        let files = list_dir_files(dir.to_str().unwrap());
        assert_eq!(files.len(), 2);
        // sorted by name
        assert!(files[0].ends_with("a.log"));
        assert!(files[1].ends_with("b.log"));
    }

    #[test]
    fn test_list_dir_files_excludes_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("visible.log"), b"v").unwrap();
        std::fs::write(dir.join(".hidden"), b"h").unwrap();
        let files = list_dir_files(dir.to_str().unwrap());
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("visible.log"));
    }

    #[test]
    fn test_list_dir_files_excludes_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("file.log"), b"f").unwrap();
        std::fs::create_dir(dir.join("subdir")).unwrap();
        let files = list_dir_files(dir.to_str().unwrap());
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("file.log"));
    }

    #[test]
    fn test_list_dir_files_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let files = list_dir_files(tmp.path().to_str().unwrap());
        assert!(files.is_empty());
    }

    #[test]
    fn test_list_dir_files_nonexistent() {
        let files = list_dir_files("/nonexistent/path/xyz123");
        assert!(files.is_empty());
    }

    #[test]
    fn test_list_dir_entries_splits_files_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("b.log"), b"b").unwrap();
        std::fs::write(dir.join("a.log"), b"a").unwrap();
        std::fs::create_dir(dir.join("subdir")).unwrap();
        let (files, dirs) = list_dir_entries(dir.to_str().unwrap());
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with("a.log"));
        assert!(files[1].ends_with("b.log"));
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].ends_with("subdir"));
    }

    #[test]
    fn test_list_dir_entries_excludes_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("visible.log"), b"v").unwrap();
        std::fs::write(dir.join(".hidden"), b"h").unwrap();
        std::fs::create_dir(dir.join(".hidden_dir")).unwrap();
        let (files, dirs) = list_dir_entries(dir.to_str().unwrap());
        assert_eq!(files.len(), 1);
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_list_dir_entries_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let (files, dirs) = list_dir_entries(tmp.path().to_str().unwrap());
        assert!(files.is_empty());
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_list_dir_entries_nonexistent() {
        let (files, dirs) = list_dir_entries("/nonexistent/path/xyz123");
        assert!(files.is_empty());
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_has_glob_metachars() {
        assert!(has_glob_metachars("/var/log/system.log*"));
        assert!(has_glob_metachars("file?.log"));
        assert!(has_glob_metachars("file[0-9].log"));
        assert!(!has_glob_metachars("/var/log/system.log"));
    }

    #[test]
    fn test_expand_glob_matches_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("system.log"), b"a").unwrap();
        std::fs::write(dir.join("system.log.1"), b"b").unwrap();
        std::fs::write(dir.join("other.log"), b"c").unwrap();
        let pattern = dir.join("system.log*").to_str().unwrap().to_string();
        let matches = expand_glob(&pattern).unwrap();
        assert_eq!(matches.len(), 2);
        assert!(matches[0].ends_with("system.log"));
        assert!(matches[1].ends_with("system.log.1"));
    }

    #[test]
    fn test_expand_glob_no_matches_is_empty_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let pattern = tmp.path().join("nomatch*").to_str().unwrap().to_string();
        let matches = expand_glob(&pattern).unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn test_expand_glob_invalid_pattern_errors() {
        assert!(expand_glob("file[").is_err());
    }
}
