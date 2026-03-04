pub struct CommandInfo {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "filter",
        usage: "filter [-m] [--fg <color>] [--bg <color>] <pattern>",
        description: "Add an include filter. -m colors match only. e.g. filter --fg Red error, filter --fg [255,0,0] error",
    },
    CommandInfo {
        name: "exclude",
        usage: "exclude <pattern>",
        description: "Add an exclude filter. e.g. exclude debug",
    },
    CommandInfo {
        name: "set-color",
        usage: "set-color [-m] --fg <color> --bg <color>",
        description: "Set color for the selected filter. -m colors match only. e.g. set-color --fg Green, set-color --fg [0,255,0]",
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
    CommandInfo {
        name: "clear-filters",
        usage: "clear-filters",
        description: "Remove all filter definitions",
    },
    CommandInfo {
        name: "disable-filters",
        usage: "disable-filters",
        description: "Disable all filters without removing them",
    },
    CommandInfo {
        name: "enable-filters",
        usage: "enable-filters",
        description: "Enable all disabled filters",
    },
    CommandInfo {
        name: "filtering",
        usage: "filtering",
        description: "Toggle global filtering on/off (bypass all filters)",
    },
    CommandInfo {
        name: "hide-field",
        usage: "hide-field <name|index>",
        description: "Hide a JSON field by name or 0-based index. e.g. hide-field MESSAGE or hide-field 0",
    },
    CommandInfo {
        name: "show-field",
        usage: "show-field <name|index>",
        description: "Show a previously hidden JSON field. e.g. show-field MESSAGE",
    },
    CommandInfo {
        name: "show-all-fields",
        usage: "show-all-fields",
        description: "Clear all hidden fields and show the complete JSON line",
    },
    CommandInfo {
        name: "select-fields",
        usage: "select-fields",
        description: "Open a modal to select which JSON fields to display and their order",
    },
    CommandInfo {
        name: "docker",
        usage: "docker",
        description: "List running Docker containers and stream logs from the selected one",
    },
    CommandInfo {
        name: "value-colors",
        usage: "value-colors",
        description: "Toggle value-based color coding (HTTP methods, status codes, IPs, UUIDs)",
    },
    CommandInfo {
        name: "export",
        usage: "export [-t <template>] <path>",
        description: "Export analysis (comments + marked lines) to a file. -t sets the template (default: markdown). e.g. export /tmp/report.md",
    },
    CommandInfo {
        name: "date-filter",
        usage: "date-filter <expression>",
        description: "Filter lines by timestamp. e.g. date-filter 01:00 .. 02:00, date-filter > 2024-02-22, date-filter >= Feb 21",
    },
    CommandInfo {
        name: "tail",
        usage: "tail",
        description: "Toggle tail mode — when on, always scrolls to the last line as new content arrives",
    },
];

/// Commands whose last argument is a file path and should receive path auto-completion.
pub const FILE_PATH_COMMANDS: &[&str] = &[
    "open",
    "load-filters",
    "save-filters",
    "export-marked",
    "export",
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
    let mut in_brackets = false;
    for ch in input.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '[' if !in_quotes => {
                in_brackets = true;
                current.push(ch);
            }
            ']' if in_brackets => {
                in_brackets = false;
                current.push(ch);
            }
            c if c.is_whitespace() && !in_quotes && !in_brackets => {
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
        .filter(|c| fuzzy_match(trimmed, c.name))
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
];

/// If the input ends with `--fg <partial>` or `--bg <partial>`, returns the partial color prefix.
pub fn extract_color_partial(input: &str) -> Option<&str> {
    let color_commands = ["filter", "set-color"];
    let trimmed = input.trim();
    let cmd = trimmed.split_whitespace().next().unwrap_or("");
    if !color_commands.contains(&cmd) {
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
    if partial.is_empty() {
        return COLOR_NAMES.to_vec();
    }
    COLOR_NAMES
        .iter()
        .filter(|c| fuzzy_match(partial, c))
        .copied()
        .collect()
}

/// Expand a leading `~` to the user's home directory.
/// `"~"` → `"/home/user"`, `"~/foo"` → `"/home/user/foo"`.
/// Paths that don't start with `~` are returned unchanged.
pub fn expand_tilde(path: &str) -> String {
    if (path == "~" || path.starts_with("~/"))
        && let Some(home) = dirs::home_dir()
    {
        if path == "~" {
            return home.to_string_lossy().into_owned();
        } else {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_owned()
}

/// Complete a partial file path by listing matching entries in the parent directory.
/// Returns a sorted list of absolute or relative paths that match the prefix.
/// Directories get a trailing `/` appended.  A leading `~` is expanded to the
/// user's home directory and preserved in the returned completions.
pub fn complete_file_path(partial: &str) -> Vec<String> {
    use std::path::Path;

    // Expand a leading `~` / `~/` to the real home directory for directory reads,
    // then restore the `~` prefix in the returned completions.
    let home = dirs::home_dir();
    let expanded: Option<String> = if partial == "~" || partial.starts_with("~/") {
        home.as_ref().map(|h| {
            if partial == "~" {
                format!("{}/", h.display())
            } else {
                format!("{}{}", h.display(), &partial[1..])
            }
        })
    } else {
        None
    };
    let tilde_expanded = expanded.is_some();
    let partial: &str = expanded.as_deref().unwrap_or(partial);

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
            if !name_prefix.is_empty() && !fuzzy_match(&name_prefix, &name) {
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

    // Restore the `~` prefix in paths that were expanded from the home directory.
    if tilde_expanded && let Some(h) = home {
        let home_str = h.to_string_lossy();
        return completions
            .into_iter()
            .map(|c| {
                if c.starts_with(home_str.as_ref()) {
                    format!("~{}", &c[home_str.len()..])
                } else {
                    c
                }
            })
            .collect();
    }
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

    #[test]
    fn test_shell_split_brackets_kept_as_single_token() {
        assert_eq!(
            shell_split("filter --fg [255, 128, 0] error"),
            vec!["filter", "--fg", "[255, 128, 0]", "error"]
        );
    }

    #[test]
    fn test_shell_split_brackets_no_spaces() {
        assert_eq!(
            shell_split("filter --fg [255,0,0] error"),
            vec!["filter", "--fg", "[255,0,0]", "error"]
        );
    }

    #[test]
    fn test_shell_split_unclosed_bracket_keeps_rest() {
        assert_eq!(
            shell_split("filter --fg [255, 0"),
            vec!["filter", "--fg", "[255, 0"]
        );
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
    fn test_find_command_completions_prefix_matches() {
        // Prefix is always a valid fuzzy match
        let results = find_command_completions("fi");
        assert!(results.contains(&"filter"));
        assert!(results.contains(&"filtering"));
        assert!(!results.contains(&"exclude"));
    }

    #[test]
    fn test_find_command_completions_subsequence_match() {
        // "flr" is not a prefix of "filter" but it is a subsequence: f-i-l-t-e-r
        let results = find_command_completions("flr");
        assert!(results.contains(&"filter"));
        assert!(results.contains(&"filtering"));
        assert!(results.contains(&"clear-filters"));
        assert!(results.contains(&"disable-filters"));
        assert!(results.contains(&"enable-filters"));
        assert!(results.contains(&"load-filters"));
        assert!(results.contains(&"save-filters"));
    }

    #[test]
    fn test_find_command_completions_abbreviation_match() {
        // "cf" matches "clear-filters" via c…f subsequence
        let results = find_command_completions("cf");
        assert!(results.contains(&"clear-filters"));
    }

    #[test]
    fn test_find_command_completions_case_insensitive() {
        let lower = find_command_completions("wrap");
        let upper = find_command_completions("WRAP");
        assert_eq!(lower, upper);
        assert!(lower.contains(&"wrap"));
    }

    #[test]
    fn test_find_command_completions_exact_match() {
        let results = find_command_completions("wrap");
        assert!(results.contains(&"wrap"));
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
    fn test_find_command_completions_trailing_space_fuzzy_matches() {
        // Trailing space is trimmed so "filter " fuzzy-matches "filter" and "filtering"
        let results = find_command_completions("filter ");
        assert!(results.contains(&"filter"));
        assert!(results.contains(&"filtering"));
    }

    #[test]
    fn test_find_command_completions_set_subsequence() {
        // "st" matches set-color and set-theme (s…t), but not "filter"
        let results = find_command_completions("stc");
        assert!(results.contains(&"set-color"));
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
    fn test_complete_color_fuzzy_re_matches_red_and_green() {
        // "Re" as a fuzzy subsequence: r then e appears in Red, Green, LightRed, LightGreen
        let results = complete_color("Re");
        assert!(results.contains(&"Red"), "Red should match");
        assert!(
            results.contains(&"Green"),
            "Green should fuzzy-match (g-r-e-e-n)"
        );
        assert!(results.contains(&"LightRed"), "LightRed should match");
        assert!(results.contains(&"LightGreen"), "LightGreen should match");
        assert!(
            !results.contains(&"Blue"),
            "Blue has no 'r' so should not match"
        );
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
        std::fs::write(path.join("zzz.log"), b"").unwrap();

        // "al" fuzzy-matches "alpha.log" but not "zzz.log" (no 'a' then 'l' in sequence)
        let prefix = format!("{}/al", path.to_str().unwrap());
        let results = complete_file_path(&prefix);
        assert!(results.iter().any(|r| r.ends_with("alpha.log")));
        assert!(!results.iter().any(|r| r.ends_with("zzz.log")));
    }

    #[test]
    fn test_complete_file_path_fuzzy_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("application.log"), b"").unwrap();
        std::fs::write(path.join("access.log"), b"").unwrap();
        std::fs::write(path.join("error.txt"), b"").unwrap();

        // "ag" matches "application.log" (a…g) and "access.log" (a…g via a-c-c-e-s-s-.-l-o-g)
        // but not "error.txt" (no 'a')
        let prefix = format!("{}/ag", path.to_str().unwrap());
        let results = complete_file_path(&prefix);
        assert!(
            results.iter().any(|r| r.ends_with("application.log")),
            "application.log should match 'ag'"
        );
        assert!(
            results.iter().any(|r| r.ends_with("access.log")),
            "access.log should match 'ag'"
        );
        assert!(
            !results.iter().any(|r| r.ends_with("error.txt")),
            "error.txt should not match 'ag'"
        );
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
    fn test_expand_tilde_bare() {
        if let Some(home) = dirs::home_dir() {
            let result = expand_tilde("~");
            assert_eq!(result, home.to_string_lossy().as_ref());
        }
    }

    #[test]
    fn test_expand_tilde_with_path() {
        if let Some(home) = dirs::home_dir() {
            let result = expand_tilde("~/foo/bar.log");
            assert_eq!(result, format!("{}/foo/bar.log", home.display()));
        }
    }

    #[test]
    fn test_expand_tilde_no_tilde() {
        assert_eq!(expand_tilde("/abs/path"), "/abs/path");
        assert_eq!(expand_tilde("relative/path"), "relative/path");
    }

    #[test]
    fn test_expand_tilde_not_at_start() {
        // A `~` that doesn't start the string should not be expanded.
        assert_eq!(expand_tilde("/foo/~bar"), "/foo/~bar");
    }

    #[test]
    fn test_complete_file_path_tilde_slash_lists_home() {
        let results = complete_file_path("~/");
        assert!(
            !results.is_empty(),
            "~/ should list home directory contents"
        );
        for r in &results {
            assert!(r.starts_with("~/"), "expected ~/ prefix, got: {r}");
        }
    }

    #[test]
    fn test_complete_file_path_tilde_alone_expands_home() {
        // Bare `~` should also expand and list home directory contents
        let results = complete_file_path("~");
        assert!(
            !results.is_empty(),
            "~ should expand to home and list contents"
        );
        for r in &results {
            assert!(r.starts_with("~/"), "expected ~/ prefix, got: {r}");
        }
    }

    #[test]
    fn test_complete_file_path_tilde_with_prefix_filters() {
        let dir = tempfile::tempdir().unwrap();
        // We can't mock dirs::home_dir(), so test the substitution logic
        // by verifying that a real ~/path returns ~ prefixed results.
        let results = complete_file_path("~/");
        // All completions must have the tilde restored, never the raw home path
        if let Some(home) = dirs::home_dir() {
            let home_str = home.to_string_lossy();
            for r in &results {
                assert!(
                    !r.starts_with(home_str.as_ref()),
                    "raw home path leaked into completion: {r}"
                );
            }
        }
        drop(dir);
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
