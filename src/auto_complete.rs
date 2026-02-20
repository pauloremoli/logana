pub struct CommandInfo {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "filter",
        usage: "filter [-m] [--fg <color>] [--bg <color>] <pattern>",
        description: "Add an include filter. -m colors match only. e.g. filter -m --fg Red error",
    },
    CommandInfo {
        name: "exclude",
        usage: "exclude <pattern>",
        description: "Add an exclude filter. e.g. exclude debug",
    },
    CommandInfo {
        name: "set-color",
        usage: "set-color [-m] --fg <color> --bg <color>",
        description: "Set color for the selected filter. -m colors match only. e.g. set-color --fg Green",
    },
    CommandInfo {
        name: "export-marked",
        usage: "export-marked <path>",
        description: "Export marked logs to a file. e.g. export-marked /tmp/marked.log",
    },
    CommandInfo {
        name: "save-filters",
        usage: "save-filters <path>",
        description: "Save current filters to JSON. e.g. save-filters filters.json",
    },
    CommandInfo {
        name: "load-filters",
        usage: "load-filters <path>",
        description: "Load filters from JSON. e.g. load-filters filters.json",
    },
    CommandInfo {
        name: "wrap",
        usage: "wrap",
        description: "Toggle line wrapping on/off",
    },
    CommandInfo {
        name: "set-theme",
        usage: "set-theme <name>",
        description: "Change the color theme. e.g. set-theme dracula",
    },
    CommandInfo {
        name: "level-colors",
        usage: "level-colors",
        description: "Toggle ERROR/WARN log level color highlighting on/off",
    },
    CommandInfo {
        name: "open",
        usage: "open <path>",
        description: "Open a file in a new tab. e.g. open /var/log/syslog",
    },
    CommandInfo {
        name: "close-tab",
        usage: "close-tab",
        description: "Close the current tab (quits if last tab)",
    },
    CommandInfo {
        name: "line-numbers",
        usage: "line-numbers",
        description: "Toggle line numbers on/off",
    },
];

pub fn command_names() -> Vec<&'static str> {
    COMMANDS.iter().map(|c| c.name).collect()
}

pub fn find_matching_command(input: &str) -> Option<&'static CommandInfo> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cmd_word = trimmed.split_whitespace().next().unwrap_or("");
    COMMANDS.iter().find(|c| c.name == cmd_word)
}

pub fn shell_split(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in input.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub fn find_command_completions(prefix: &str) -> Vec<&'static str> {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return command_names();
    }
    // Only complete the command name (first word)
    if trimmed.contains(' ') {
        return vec![];
    }
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(trimmed))
        .map(|c| c.name)
        .collect()
}

pub const COLOR_NAMES: &[&str] = &[
    "Black",
    "Red",
    "Green",
    "Yellow",
    "Blue",
    "Magenta",
    "Cyan",
    "Gray",
    "DarkGray",
    "LightRed",
    "LightGreen",
    "LightYellow",
    "LightBlue",
    "LightMagenta",
    "LightCyan",
    "White",
    "Reset",
];

/// If the input ends with `--fg <partial>` or `--bg <partial>`, returns the partial color prefix.
pub fn extract_color_partial(input: &str) -> Option<&str> {
    let color_commands = ["filter", "set-color"];
    let trimmed = input.trim();
    let cmd = trimmed.split_whitespace().next().unwrap_or("");
    if !color_commands.iter().any(|c| *c == cmd) {
        return None;
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    let last = tokens[tokens.len() - 1];
    let second_last = tokens[tokens.len() - 2];

    if second_last == "--fg" || second_last == "--bg" {
        return Some(last);
    }

    if (last == "--fg" || last == "--bg") && input.ends_with(' ') {
        return Some("");
    }

    None
}

pub fn complete_color(partial: &str) -> Vec<&'static str> {
    let lower = partial.to_lowercase();
    COLOR_NAMES
        .iter()
        .filter(|c| c.to_lowercase().starts_with(&lower))
        .copied()
        .collect()
}

/// Complete a partial file path by listing matching entries in the parent directory.
/// Returns a sorted list of absolute or relative paths that match the prefix.
/// Directories get a trailing `/` appended.
pub fn complete_file_path(partial: &str) -> Vec<String> {
    use std::path::Path;

    let path = Path::new(partial);

    let (dir, name_prefix) =
        if partial.ends_with('/') || partial.ends_with(std::path::MAIN_SEPARATOR) {
            (path.to_path_buf(), String::new())
        } else if let Some(parent) = path.parent() {
            let prefix = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let dir = if parent.as_os_str().is_empty() {
                Path::new(".").to_path_buf()
            } else {
                parent.to_path_buf()
            };
            (dir, prefix)
        } else {
            (Path::new(".").to_path_buf(), partial.to_string())
        };

    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return vec![],
    };

    let mut completions: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if name.starts_with('.') && !name_prefix.starts_with('.') {
                return None;
            }
            if !name.starts_with(&name_prefix) {
                return None;
            }
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

            let base = if partial.ends_with('/') || partial.ends_with(std::path::MAIN_SEPARATOR) {
                partial.to_string()
            } else if let Some(parent) = Path::new(partial).parent() {
                let p = parent.to_str().unwrap_or("");
                if p.is_empty() {
                    String::new()
                } else {
                    format!("{}/", p)
                }
            } else {
                String::new()
            };

            let suffix = if is_dir { "/" } else { "" };
            Some(format!("{}{}{}", base, name, suffix))
        })
        .collect();

    completions.sort();
    completions
}

/// Returns true if all characters of `needle` appear in `haystack` in order (subsequence check),
/// case-insensitive.
pub fn fuzzy_match(needle: &str, haystack: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle_lc = needle.to_lowercase();
    let haystack_lc = haystack.to_lowercase();
    let mut needle_chars = needle_lc.chars();
    let mut current = needle_chars.next();
    for c in haystack_lc.chars() {
        if let Some(nc) = current {
            if c == nc {
                current = needle_chars.next();
            }
        } else {
            break;
        }
    }
    current.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── command_names ────────────────────────────────────────────────────────

    #[test]
    fn test_command_names_returns_all_commands() {
        let names = command_names();
        assert_eq!(names.len(), COMMANDS.len());
    }

    #[test]
    fn test_command_names_contains_known_commands() {
        let names = command_names();
        for expected in &[
            "filter",
            "exclude",
            "set-color",
            "wrap",
            "set-theme",
            "level-colors",
            "open",
            "close-tab",
            "line-numbers",
            "export-marked",
            "save-filters",
            "load-filters",
        ] {
            assert!(names.contains(expected), "missing command: {expected}");
        }
    }

    // ── find_matching_command ────────────────────────────────────────────────

    #[test]
    fn test_find_matching_command_exact() {
        let cmd = find_matching_command("filter").unwrap();
        assert_eq!(cmd.name, "filter");
    }

    #[test]
    fn test_find_matching_command_with_args() {
        let cmd = find_matching_command("filter --fg Red error").unwrap();
        assert_eq!(cmd.name, "filter");
    }

    #[test]
    fn test_find_matching_command_with_leading_spaces() {
        let cmd = find_matching_command("  wrap  ").unwrap();
        assert_eq!(cmd.name, "wrap");
    }

    #[test]
    fn test_find_matching_command_empty_returns_none() {
        assert!(find_matching_command("").is_none());
        assert!(find_matching_command("   ").is_none());
    }

    #[test]
    fn test_find_matching_command_unknown_returns_none() {
        assert!(find_matching_command("unknown-cmd").is_none());
    }

    #[test]
    fn test_find_matching_command_partial_prefix_returns_none() {
        assert!(find_matching_command("fil").is_none());
    }

    #[test]
    fn test_find_matching_command_usage_and_description_populated() {
        let cmd = find_matching_command("filter").unwrap();
        assert!(!cmd.usage.is_empty());
        assert!(!cmd.description.is_empty());
    }

    // ── shell_split ──────────────────────────────────────────────────────────

    #[test]
    fn test_shell_split_basic() {
        assert_eq!(shell_split("filter foo"), vec!["filter", "foo"]);
        assert_eq!(shell_split("  filter  foo  "), vec!["filter", "foo"]);
        assert_eq!(shell_split(""), Vec::<String>::new());
    }

    #[test]
    fn test_shell_split_single_token() {
        assert_eq!(shell_split("wrap"), vec!["wrap"]);
    }

    #[test]
    fn test_shell_split_only_whitespace() {
        assert_eq!(shell_split("   "), Vec::<String>::new());
    }

    #[test]
    fn test_shell_split_quoted() {
        assert_eq!(
            shell_split(r#"filter "hello world""#),
            vec!["filter", "hello world"]
        );
        assert_eq!(
            shell_split(r#"exclude "foo bar baz""#),
            vec!["exclude", "foo bar baz"]
        );
    }

    #[test]
    fn test_shell_split_quoted_preserves_inner_spaces() {
        assert_eq!(
            shell_split(r#"filter "  spaced  ""#),
            vec!["filter", "  spaced  "]
        );
    }

    #[test]
    fn test_shell_split_mixed_args() {
        assert_eq!(
            shell_split(r#"filter "my pattern" --fg Red --bg Blue"#),
            vec!["filter", "my pattern", "--fg", "Red", "--bg", "Blue"]
        );
    }

    #[test]
    fn test_shell_split_unclosed_quote_treated_as_one_token() {
        assert_eq!(
            shell_split(r#"filter "unclosed"#),
            vec!["filter", "unclosed"]
        );
    }

    #[test]
    fn test_shell_split_empty_quoted_string() {
        assert_eq!(shell_split(r#"filter """#), vec!["filter"]);
    }

    // ── find_command_completions ─────────────────────────────────────────────

    #[test]
    fn test_find_command_completions_empty_returns_all() {
        let results = find_command_completions("");
        assert_eq!(results.len(), COMMANDS.len());
    }

    #[test]
    fn test_find_command_completions_whitespace_returns_all() {
        let results = find_command_completions("  ");
        assert_eq!(results.len(), COMMANDS.len());
    }

    #[test]
    fn test_find_command_completions_prefix_multiple_matches() {
        let results = find_command_completions("fi");
        assert!(results.contains(&"filter"));
        assert!(!results.contains(&"exclude"));
    }

    #[test]
    fn test_find_command_completions_prefix_single_match() {
        let results = find_command_completions("wra");
        assert_eq!(results, vec!["wrap"]);
    }

    #[test]
    fn test_find_command_completions_exact_match() {
        let results = find_command_completions("wrap");
        assert_eq!(results, vec!["wrap"]);
    }

    #[test]
    fn test_find_command_completions_no_match() {
        let results = find_command_completions("zzz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_find_command_completions_with_args_returns_empty() {
        // Once there are two words (command + argument), no completions
        assert!(find_command_completions("filter foo").is_empty());
        assert!(find_command_completions("wrap extra").is_empty());
    }

    #[test]
    fn test_find_command_completions_trailing_space_trims_to_exact_match() {
        // Trailing space is trimmed so "filter " behaves like "filter"
        let results = find_command_completions("filter ");
        assert_eq!(results, vec!["filter"]);
    }

    #[test]
    fn test_find_command_completions_set_prefix() {
        let results = find_command_completions("set-");
        assert!(results.contains(&"set-color"));
        assert!(results.contains(&"set-theme"));
        assert!(!results.contains(&"filter"));
    }

    // ── extract_color_partial ────────────────────────────────────────────────

    #[test]
    fn test_extract_color_partial_fg_with_partial_value() {
        assert_eq!(extract_color_partial("filter --fg Re"), Some("Re"));
    }

    #[test]
    fn test_extract_color_partial_bg_with_partial_value() {
        assert_eq!(extract_color_partial("filter --bg Gr"), Some("Gr"));
    }

    #[test]
    fn test_extract_color_partial_set_color_fg() {
        assert_eq!(extract_color_partial("set-color --fg Li"), Some("Li"));
    }

    #[test]
    fn test_extract_color_partial_trailing_space_returns_empty() {
        assert_eq!(extract_color_partial("filter --fg "), Some(""));
        assert_eq!(extract_color_partial("set-color --bg "), Some(""));
    }

    #[test]
    fn test_extract_color_partial_fg_without_trailing_space_returns_none() {
        // "--fg" at end with no space after = not yet triggering completion
        assert_eq!(extract_color_partial("filter --fg"), None);
    }

    #[test]
    fn test_extract_color_partial_non_color_command_returns_none() {
        assert_eq!(extract_color_partial("exclude --fg Red"), None);
        assert_eq!(extract_color_partial("open --fg Red"), None);
    }

    #[test]
    fn test_extract_color_partial_no_flag_returns_none() {
        assert_eq!(extract_color_partial("filter foo"), None);
        assert_eq!(extract_color_partial("filter"), None);
    }

    #[test]
    fn test_extract_color_partial_empty_input_returns_none() {
        assert_eq!(extract_color_partial(""), None);
    }

    #[test]
    fn test_extract_color_partial_filter_with_multiple_args() {
        // filter pattern --fg Red --bg Gr  →  second_last=--bg, last=Gr
        assert_eq!(
            extract_color_partial("filter pattern --fg Red --bg Gr"),
            Some("Gr")
        );
    }

    // ── complete_color ───────────────────────────────────────────────────────

    #[test]
    fn test_complete_color_empty_returns_all() {
        let results = complete_color("");
        assert_eq!(results.len(), COLOR_NAMES.len());
    }

    #[test]
    fn test_complete_color_prefix_uppercase() {
        let results = complete_color("Re");
        assert!(results.contains(&"Red"));
        assert!(results.contains(&"Reset"));
        assert!(!results.contains(&"Blue"));
    }

    #[test]
    fn test_complete_color_case_insensitive() {
        let upper = complete_color("RED");
        let lower = complete_color("red");
        let mixed = complete_color("Red");
        assert_eq!(upper, lower);
        assert_eq!(upper, mixed);
        assert!(upper.contains(&"Red"));
    }

    #[test]
    fn test_complete_color_light_prefix() {
        let results = complete_color("Light");
        assert!(results.contains(&"LightRed"));
        assert!(results.contains(&"LightGreen"));
        assert!(results.contains(&"LightBlue"));
        assert!(results.contains(&"LightMagenta"));
        assert!(results.contains(&"LightCyan"));
        assert!(results.contains(&"LightYellow"));
        assert!(!results.contains(&"DarkGray"));
    }

    #[test]
    fn test_complete_color_no_match_returns_empty() {
        assert!(complete_color("Zzz").is_empty());
    }

    #[test]
    fn test_complete_color_exact_match() {
        let results = complete_color("Black");
        assert_eq!(results, vec!["Black"]);
    }

    #[test]
    fn test_complete_file_path_nonexistent_dir_returns_empty() {
        let results = complete_file_path("/nonexistent_dir_xyz/");
        assert!(results.is_empty());
    }

    #[test]
    fn test_complete_file_path_lists_files_in_dir_with_trailing_slash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("alpha.log"), b"").unwrap();
        std::fs::write(path.join("beta.log"), b"").unwrap();

        let prefix = format!("{}/", path.to_str().unwrap());
        let results = complete_file_path(&prefix);
        assert!(results.iter().any(|r| r.ends_with("alpha.log")));
        assert!(results.iter().any(|r| r.ends_with("beta.log")));
    }

    #[test]
    fn test_complete_file_path_filters_by_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("alpha.log"), b"").unwrap();
        std::fs::write(path.join("beta.log"), b"").unwrap();

        let prefix = format!("{}/al", path.to_str().unwrap());
        let results = complete_file_path(&prefix);
        assert_eq!(results.len(), 1);
        assert!(results[0].ends_with("alpha.log"));
    }

    #[test]
    fn test_complete_file_path_directory_gets_trailing_slash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::create_dir(path.join("subdir")).unwrap();

        let prefix = format!("{}/sub", path.to_str().unwrap());
        let results = complete_file_path(&prefix);
        assert_eq!(results.len(), 1);
        assert!(results[0].ends_with("subdir/"));
    }

    #[test]
    fn test_complete_file_path_hidden_files_excluded_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join(".hidden"), b"").unwrap();
        std::fs::write(path.join("visible"), b"").unwrap();

        let prefix = format!("{}/", path.to_str().unwrap());
        let results = complete_file_path(&prefix);
        assert!(!results.iter().any(|r| r.ends_with(".hidden")));
        assert!(results.iter().any(|r| r.ends_with("visible")));
    }

    #[test]
    fn test_complete_file_path_hidden_files_included_with_dot_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join(".hidden"), b"").unwrap();

        let prefix = format!("{}/.hid", path.to_str().unwrap());
        let results = complete_file_path(&prefix);
        assert!(results.iter().any(|r| r.ends_with(".hidden")));
    }

    #[test]
    fn test_complete_file_path_results_are_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("z_last.log"), b"").unwrap();
        std::fs::write(path.join("a_first.log"), b"").unwrap();
        std::fs::write(path.join("m_middle.log"), b"").unwrap();

        let prefix = format!("{}/", path.to_str().unwrap());
        let results = complete_file_path(&prefix);
        let sorted = {
            let mut s = results.clone();
            s.sort();
            s
        };
        assert_eq!(results, sorted);
    }

    #[test]
    fn test_complete_file_path_relative_prefix() {
        // Completing a bare filename prefix in the current dir must not panic
        let results = complete_file_path("Cargo");
        // Should find Cargo.toml / Cargo.lock at minimum
        assert!(results.iter().any(|r| r.starts_with("Cargo")));
    }

    #[test]
    fn test_fuzzy_match() {
        assert!(fuzzy_match("dra", "dracula"));
        assert!(fuzzy_match("dul", "dracula"));
        assert!(fuzzy_match("DRA", "dracula"));
        assert!(fuzzy_match("", "anything"));
        assert!(!fuzzy_match("xyz", "dracula"));
    }
}
