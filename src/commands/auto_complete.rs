use std::collections::HashMap;

use super::{COMMANDS, command_names};

pub const COMMAND_FLAGS: &[(&str, &[&str])] = &[
    (
        "filter",
        &["--field", "-f", "--fg", "--bg", "-l", "--group", "-g"],
    ),
    ("exclude", &["--field", "-f", "--group", "-g"]),
    (
        "highlight",
        &["--field", "-f", "--fg", "--bg", "-l", "--group", "-g"],
    ),
    ("set-color", &["--fg", "--bg", "-l"]),
    ("date-filter", &["--fg", "--bg", "-l"]),
    ("export", &["-t", "--template"]),
    ("group", &["--fg", "--bg", "-l", "--auto", "--clear"]),
];

/// If the current token being typed starts with `-` and there is at least one
/// preceding token (the command), returns `(prefix, partial)` where `prefix`
/// is everything up to and including the last space, and `partial` is the
/// flag token in progress.
pub fn extract_flag_partial(input: &str) -> Option<(String, String)> {
    if input.ends_with(' ') {
        return None;
    }
    let last_space = input.rfind(' ').map(|i| i + 1).unwrap_or(0);
    let partial = &input[last_space..];
    let prefix = &input[..last_space];
    if prefix.trim().is_empty() {
        return None;
    }
    if partial.starts_with('-') {
        Some((prefix.to_string(), partial.to_string()))
    } else {
        None
    }
}

/// Return flag completions for `cmd` filtered by `partial` using fuzzy matching.
pub fn complete_flags(cmd: &str, partial: &str) -> Vec<&'static str> {
    let flags = COMMAND_FLAGS
        .iter()
        .find(|(name, _)| *name == cmd)
        .map(|(_, f)| *f)
        .unwrap_or(&[]);
    flags
        .iter()
        .filter(|f| fuzzy_match(partial, f))
        .copied()
        .collect()
}

/// Index of unique field names and values collected from visible log lines.
#[derive(Debug, Default, Clone)]
pub struct FieldIndex {
    /// Sorted, deduplicated list of all known field names.
    pub names: Vec<String>,
    /// For each field name: sorted, deduplicated list of observed values.
    pub values: HashMap<String, Vec<String>>,
}

/// Describes what a partial `--field` expression is currently completing.
#[derive(Debug, PartialEq)]
pub enum FieldCompletion {
    /// Completing the field name part (before `=`). Holds the partial name typed so far.
    Name(String),
    /// Completing the value part (after `=`). Holds the field name and partial value.
    Value { field: String, partial: String },
}

/// Detect whether the current command input is in the middle of a `--field key=value`
/// argument and return what needs completing.
///
/// Recognised commands: `filter`, `exclude`.
///
/// Returns:
/// - `Some(FieldCompletion::Name(partial))` when cursor is on the field name (before `=`)
/// - `Some(FieldCompletion::Value { field, partial })` when cursor is on the value (after `=`)
/// - `None` when the input is not a `--field` context or the pattern is complete
pub fn extract_field_partial(input: &str) -> Option<FieldCompletion> {
    let field_commands = ["filter", "exclude", "highlight"];
    let trimmed = input.trim();
    let cmd = trimmed.split_whitespace().next().unwrap_or("");
    if !field_commands.contains(&cmd) {
        return None;
    }

    // Look for `--field` / `-f` token in the input
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let field_pos = tokens.iter().position(|&t| t == "--field" || t == "-f")?;

    // The token immediately after `--field` is the pattern being typed
    let after_field = tokens.get(field_pos + 1);

    match after_field {
        None => {
            // `--field` is the last token and input ends with a space
            if input.ends_with(' ') {
                Some(FieldCompletion::Name(String::new()))
            } else {
                None
            }
        }
        Some(&pattern) => {
            // If there are more tokens after the pattern, the pattern is complete
            if tokens.len() > field_pos + 2 {
                return None;
            }
            if let Some(eq_pos) = pattern.find('=') {
                let field = &pattern[..eq_pos];
                let partial = &pattern[eq_pos + 1..];
                // If input ends with ' ' after the complete pattern, it is done
                if input.ends_with(' ') {
                    return None;
                }
                Some(FieldCompletion::Value {
                    field: field.to_string(),
                    partial: partial.to_string(),
                })
            } else {
                // Still typing the field name (no `=` yet)
                if input.ends_with(' ') {
                    return None; // pattern with no '=' and trailing space → done
                }
                Some(FieldCompletion::Name(pattern.to_string()))
            }
        }
    }
}

/// Return completions for a partial field name.
pub fn complete_field_name(partial: &str, index: &FieldIndex) -> Vec<String> {
    index
        .names
        .iter()
        .filter(|n| fuzzy_match(partial, n))
        .cloned()
        .collect()
}

/// Return completions for a partial field value given the field name.
pub fn complete_field_value(field: &str, partial: &str, index: &FieldIndex) -> Vec<String> {
    let Some(values) = index.values.get(field) else {
        return vec![];
    };
    values
        .iter()
        .filter(|v| fuzzy_match(partial, v))
        .cloned()
        .collect()
}

pub fn shell_split(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut in_brackets = false;
    let mut escape_next = false;
    for ch in input.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escape_next = true,
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
    "Blue",
    "Brown",
    "Coral",
    "Cyan",
    "DarkGray",
    "Gold",
    "Gray",
    "Green",
    "Indigo",
    "LightBlue",
    "LightCyan",
    "LightGreen",
    "LightMagenta",
    "LightRed",
    "LightYellow",
    "Lime",
    "Magenta",
    "Maroon",
    "Navy",
    "Olive",
    "Orange",
    "Pink",
    "Purple",
    "Red",
    "Salmon",
    "Teal",
    "Turquoise",
    "Violet",
    "White",
    "Yellow",
];

/// If the input ends with `--fg <partial>` or `--bg <partial>`, returns the partial color prefix.
pub fn extract_color_partial(input: &str) -> Option<&str> {
    let color_commands = ["filter", "set-color", "date-filter", "highlight"];
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

/// Every schema name `:schema <name>` can select, or that's useful to
/// surface as available: custom names first (alphabetical), then the
/// reserved `none` keyword (clears the tab's format, treating it as raw
/// text), then built-in format names (alphabetical) — `true` marks a
/// custom schema, so the caller can render the two kinds differently.
/// `custom_names` is passed in rather than read from
/// `config::custom_schemas()` directly so this stays unit-testable
/// without touching that process-global `OnceLock`.
pub fn schema_completion_names(custom_names: &[String]) -> Vec<(String, bool)> {
    let mut custom = custom_names.to_vec();
    custom.sort();
    let mut builtin = crate::parser::builtin_format_names();
    builtin.sort();
    custom
        .into_iter()
        .map(|n| (n, true))
        .chain(std::iter::once(("none".to_string(), false)))
        .chain(builtin.into_iter().map(|n| (n, false)))
        .collect()
}

/// Fuzzy-filters `schema_completion_names`'s output against `partial`,
/// preserving the custom-first/alphabetical-within-group ordering.
pub fn complete_schema_with_builtins(
    partial: &str,
    custom_names: &[String],
) -> Vec<(String, bool)> {
    schema_completion_names(custom_names)
        .into_iter()
        .filter(|(n, _)| fuzzy_match(partial, n))
        .collect()
}

/// If the input ends with `--group <partial>` or `-g <partial>`, returns the partial group-name prefix.
pub fn extract_group_partial(input: &str) -> Option<&str> {
    let group_commands = ["filter", "exclude", "highlight"];
    let trimmed = input.trim();
    let cmd = trimmed.split_whitespace().next().unwrap_or("");
    if !group_commands.contains(&cmd) {
        return None;
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    let last = tokens[tokens.len() - 1];
    let second_last = tokens[tokens.len() - 2];

    if second_last == "--group" || second_last == "-g" {
        return Some(last);
    }

    if (last == "--group" || last == "-g") && input.ends_with(' ') {
        return Some("");
    }

    None
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

/// Case-insensitive regex search: `needle` is compiled as a regex and
/// matched against `haystack`. An empty `needle` always matches.
///
/// `needle` is typed one character at a time by a live, per-keystroke
/// search, so it's frequently an incomplete/invalid regex mid-composition
/// (e.g. `"(err"` before `"(error|err)"`) — rather than going blank or
/// panicking, an invalid pattern falls back to a plain case-insensitive
/// substring match of `needle` itself.
pub fn regex_search_match(needle: &str, haystack: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    match regex::RegexBuilder::new(needle)
        .case_insensitive(true)
        .build()
    {
        Ok(re) => re.is_match(haystack),
        Err(_) => haystack.to_lowercase().contains(&needle.to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_shell_split_escaped_quote_inside_quotes() {
        assert_eq!(
            shell_split(r#"filter "hello \"world\"""#),
            vec!["filter", r#"hello "world""#]
        );
    }

    #[test]
    fn test_shell_split_escaped_quote_preserves_spaces() {
        assert_eq!(
            shell_split(r#"filter "say \"hi\" now""#),
            vec!["filter", r#"say "hi" now"#]
        );
    }

    #[test]
    fn test_shell_split_backslash_outside_quotes_passes_through() {
        // Outside quotes, backslash is not special — it is pushed as-is.
        assert_eq!(
            shell_split(r"filter hello\.world"),
            vec!["filter", r"hello\.world"]
        );
    }

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
    fn test_extract_color_partial_recognizes_highlight() {
        assert_eq!(extract_color_partial("highlight --fg Re"), Some("Re"));
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

    #[test]
    fn test_extract_group_partial_filter_with_partial_value() {
        assert_eq!(extract_group_partial("filter --group er"), Some("er"));
    }

    #[test]
    fn test_extract_group_partial_exclude_with_partial_value() {
        assert_eq!(extract_group_partial("exclude --group no"), Some("no"));
    }

    #[test]
    fn test_extract_group_partial_recognizes_highlight() {
        assert_eq!(extract_group_partial("highlight --group err"), Some("err"));
    }

    #[test]
    fn test_extract_group_partial_trailing_space_returns_empty() {
        assert_eq!(extract_group_partial("filter --group "), Some(""));
    }

    #[test]
    fn test_extract_group_partial_without_trailing_space_returns_none() {
        assert_eq!(extract_group_partial("filter --group"), None);
    }

    #[test]
    fn test_extract_group_partial_non_group_command_returns_none() {
        assert_eq!(extract_group_partial("set-color --group er"), None);
    }

    #[test]
    fn test_extract_group_partial_no_flag_returns_none() {
        assert_eq!(extract_group_partial("filter foo"), None);
    }

    #[test]
    fn test_extract_group_partial_with_preceding_args() {
        assert_eq!(
            extract_group_partial("filter --fg red --group er"),
            Some("er")
        );
    }

    #[test]
    fn test_extract_group_partial_short_flag() {
        assert_eq!(extract_group_partial("filter -g er"), Some("er"));
        assert_eq!(extract_group_partial("exclude -g "), Some(""));
    }

    #[test]
    fn test_color_names_sorted_alphabetically() {
        let mut sorted = COLOR_NAMES.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            COLOR_NAMES, sorted,
            "COLOR_NAMES should be listed alphabetically so filter autocomplete suggestions are sorted"
        );
    }

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
    fn test_complete_color_extended_names_present() {
        let all = complete_color("");
        for name in &[
            "Orange",
            "Pink",
            "Purple",
            "Violet",
            "Indigo",
            "Teal",
            "Turquoise",
            "Coral",
            "Salmon",
            "Gold",
            "Lime",
            "Maroon",
            "Navy",
            "Olive",
            "Brown",
        ] {
            assert!(all.contains(name), "{name} should be in COLOR_NAMES");
        }
    }

    #[test]
    fn test_complete_color_extended_fuzzy_match() {
        let results = complete_color("Ora");
        assert!(
            results.contains(&"Orange"),
            "Orange should fuzzy-match 'Ora'"
        );

        let results = complete_color("Teal");
        assert_eq!(results, vec!["Teal"]);

        let results = complete_color("nav");
        assert!(results.contains(&"Navy"), "Navy should fuzzy-match 'nav'");
    }

    #[test]
    fn test_schema_completion_names_custom_first_alphabetical() {
        let names = vec!["zeta".to_string(), "acme".to_string()];
        let results = schema_completion_names(&names);
        let custom: Vec<&(String, bool)> = results.iter().filter(|(_, c)| *c).collect();
        assert_eq!(
            custom.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            vec!["acme", "zeta"]
        );
    }

    #[test]
    fn test_schema_completion_names_includes_builtin_formats() {
        let results = schema_completion_names(&[]);
        assert!(
            results
                .iter()
                .any(|(n, is_custom)| n == "syslog" && !is_custom)
        );
    }

    #[test]
    fn test_schema_completion_names_builtins_are_alphabetical() {
        let results = schema_completion_names(&[]);
        let builtin: Vec<&str> = results
            .iter()
            .filter(|(n, is_custom)| !is_custom && n != "none")
            .map(|(n, _)| n.as_str())
            .collect();
        let mut sorted = builtin.clone();
        sorted.sort();
        assert_eq!(builtin, sorted);
    }

    #[test]
    fn test_schema_completion_names_includes_none() {
        let results = schema_completion_names(&["acme".to_string()]);
        assert!(
            results
                .iter()
                .any(|(n, is_custom)| n == "none" && !is_custom)
        );
    }

    #[test]
    fn test_schema_completion_names_none_present_without_custom_schemas() {
        let results = schema_completion_names(&[]);
        assert!(results.iter().any(|(n, _)| n == "none"));
    }

    #[test]
    fn test_complete_schema_with_builtins_empty_returns_all() {
        let names = vec!["acme".to_string()];
        let results = complete_schema_with_builtins("", &names);
        assert_eq!(results.len(), schema_completion_names(&names).len());
    }

    #[test]
    fn test_complete_schema_with_builtins_matches_custom_name() {
        let names = vec!["acme".to_string(), "syslog2".to_string()];
        let results = complete_schema_with_builtins("ae", &names);
        assert_eq!(results, vec![("acme".to_string(), true)]);
    }

    #[test]
    fn test_complete_schema_with_builtins_matches_builtin_name() {
        let results = complete_schema_with_builtins("syslog", &[]);
        assert!(
            results
                .iter()
                .any(|(n, is_custom)| n == "syslog" && !is_custom)
        );
    }

    #[test]
    fn test_complete_schema_with_builtins_no_match_returns_empty() {
        assert!(complete_schema_with_builtins("zzz_no_match", &["acme".to_string()]).is_empty());
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
    fn test_extract_flag_partial_none_when_trailing_space() {
        assert_eq!(extract_flag_partial("filter "), None);
    }

    #[test]
    fn test_extract_flag_partial_none_when_no_command() {
        assert_eq!(extract_flag_partial("-"), None);
    }

    #[test]
    fn test_extract_flag_partial_none_when_not_flag() {
        assert_eq!(extract_flag_partial("filter error"), None);
    }

    #[test]
    fn test_extract_flag_partial_single_dash() {
        assert_eq!(
            extract_flag_partial("filter -"),
            Some(("filter ".to_string(), "-".to_string()))
        );
    }

    #[test]
    fn test_extract_flag_partial_double_dash() {
        assert_eq!(
            extract_flag_partial("filter --"),
            Some(("filter ".to_string(), "--".to_string()))
        );
    }

    #[test]
    fn test_extract_flag_partial_partial_flag() {
        assert_eq!(
            extract_flag_partial("filter --f"),
            Some(("filter ".to_string(), "--f".to_string()))
        );
    }

    #[test]
    fn test_extract_flag_partial_mid_input() {
        assert_eq!(
            extract_flag_partial("filter --fg Blue -"),
            Some(("filter --fg Blue ".to_string(), "-".to_string()))
        );
    }

    #[test]
    fn test_complete_flags_filter_all() {
        let flags = complete_flags("filter", "-");
        assert_eq!(flags.len(), 7);
        assert!(flags.contains(&"--field"));
        assert!(flags.contains(&"-f"));
        assert!(flags.contains(&"--fg"));
        assert!(flags.contains(&"--bg"));
        assert!(flags.contains(&"-l"));
        assert!(flags.contains(&"--group"));
        assert!(flags.contains(&"-g"));
    }

    #[test]
    fn test_complete_flags_filter_partial() {
        let flags = complete_flags("filter", "--f");
        assert!(flags.contains(&"--field"));
        assert!(flags.contains(&"--fg"));
        assert!(!flags.contains(&"--bg"));
        assert!(!flags.contains(&"-l"));
    }

    #[test]
    fn test_complete_flags_highlight_partial() {
        let flags = complete_flags("highlight", "--f");
        assert!(flags.contains(&"--field"));
        assert!(flags.contains(&"--fg"));
    }

    #[test]
    fn test_complete_flags_set_color() {
        let flags = complete_flags("set-color", "-");
        assert!(flags.contains(&"--fg"));
        assert!(flags.contains(&"--bg"));
        assert!(flags.contains(&"-l"));
    }

    #[test]
    fn test_complete_flags_group() {
        let flags = complete_flags("group", "--f");
        assert!(flags.contains(&"--fg"));
    }

    #[test]
    fn test_complete_flags_no_flags_command() {
        let flags = complete_flags("wrap", "-");
        assert!(flags.is_empty());
    }

    #[test]
    fn test_complete_flags_empty_for_unknown_cmd() {
        let flags = complete_flags("unknown", "-");
        assert!(flags.is_empty());
    }

    #[test]
    fn test_fuzzy_match() {
        assert!(fuzzy_match("dra", "dracula"));
        assert!(fuzzy_match("dul", "dracula"));
        assert!(fuzzy_match("DRA", "dracula"));
        assert!(fuzzy_match("", "anything"));
        assert!(!fuzzy_match("xyz", "dracula"));
    }

    #[test]
    fn test_regex_search_match_empty_needle_matches_anything() {
        assert!(regex_search_match("", "anything"));
    }

    #[test]
    fn test_regex_search_match_literal_substring() {
        assert!(regex_search_match("dra", "dracula"));
        assert!(!regex_search_match("xyz", "dracula"));
    }

    #[test]
    fn test_regex_search_match_is_case_insensitive() {
        assert!(regex_search_match("DRA", "dracula"));
    }

    #[test]
    fn test_regex_search_match_not_fuzzy_subsequence() {
        // "dul" is a fuzzy subsequence of "dracula" (d..u..l) but not a
        // contiguous/regex substring — regex search must NOT match it,
        // unlike fuzzy_match which does (see test_fuzzy_match above).
        assert!(!regex_search_match("dul", "dracula"));
    }

    #[test]
    fn test_regex_search_match_actual_regex_alternation() {
        assert!(regex_search_match("err(or)?", "an error occurred"));
        assert!(regex_search_match("err(or)?", "err"));
        assert!(!regex_search_match("err(or)?", "warning"));
    }

    #[test]
    fn test_regex_search_match_invalid_regex_falls_back_to_literal_substring() {
        // "(" alone is invalid regex (unclosed group) — likely typed
        // mid-composition of a real pattern. Must not panic or go blank;
        // falls back to a literal, case-insensitive substring match.
        assert!(regex_search_match("(err", "a line with (err in it"));
        assert!(!regex_search_match("(err", "no match here"));
    }

    #[test]
    fn test_field_partial_name_empty_after_space() {
        assert_eq!(
            extract_field_partial("filter --field "),
            Some(FieldCompletion::Name(String::new()))
        );
    }

    #[test]
    fn test_field_partial_name_partial_typed() {
        assert_eq!(
            extract_field_partial("filter --field lev"),
            Some(FieldCompletion::Name("lev".to_string()))
        );
    }

    #[test]
    fn test_field_partial_recognizes_highlight() {
        assert_eq!(
            extract_field_partial("highlight --field lev"),
            Some(FieldCompletion::Name("lev".to_string()))
        );
    }

    #[test]
    fn test_field_partial_value_empty_after_eq() {
        assert_eq!(
            extract_field_partial("filter --field level="),
            Some(FieldCompletion::Value {
                field: "level".to_string(),
                partial: String::new()
            })
        );
    }

    #[test]
    fn test_field_partial_value_partial_typed() {
        assert_eq!(
            extract_field_partial("filter --field level=err"),
            Some(FieldCompletion::Value {
                field: "level".to_string(),
                partial: "err".to_string()
            })
        );
    }

    #[test]
    fn test_field_partial_complete_pattern_with_space_returns_none() {
        assert_eq!(extract_field_partial("filter --field level=error "), None);
    }

    #[test]
    fn test_field_partial_no_field_flag_returns_none() {
        assert_eq!(extract_field_partial("filter level=error"), None);
    }

    #[test]
    fn test_field_partial_non_field_command_returns_none() {
        assert_eq!(extract_field_partial("open --field level=err"), None);
    }

    #[test]
    fn test_field_partial_exclude_command_works() {
        assert_eq!(
            extract_field_partial("exclude --field target=au"),
            Some(FieldCompletion::Value {
                field: "target".to_string(),
                partial: "au".to_string()
            })
        );
    }

    fn make_index(names: &[&str], values: &[(&str, &[&str])]) -> FieldIndex {
        let mut idx = FieldIndex {
            names: names.iter().map(|s| s.to_string()).collect(),
            values: HashMap::new(),
        };
        for (field, vals) in values {
            idx.values.insert(
                field.to_string(),
                vals.iter().map(|v| v.to_string()).collect(),
            );
        }
        idx
    }

    #[test]
    fn test_complete_field_name_empty_partial_returns_all() {
        let idx = make_index(&["level", "target"], &[]);
        let result = complete_field_name("", &idx);
        assert!(result.contains(&"level".to_string()));
        assert!(result.contains(&"target".to_string()));
    }

    #[test]
    fn test_complete_field_name_partial_fuzzy_matches() {
        let idx = make_index(&["level", "target", "component"], &[]);
        let result = complete_field_name("lev", &idx);
        assert!(result.contains(&"level".to_string()));
        assert!(!result.contains(&"target".to_string()));
    }

    #[test]
    fn test_complete_field_name_no_match() {
        let idx = make_index(&["level", "target"], &[]);
        let result = complete_field_name("zzz", &idx);
        assert!(result.is_empty());
    }

    #[test]
    fn test_complete_field_value_empty_partial_returns_all() {
        let idx = make_index(
            &["level"],
            &[("level", &["info", "error", "warn", "debug"])],
        );
        let result = complete_field_value("level", "", &idx);
        assert_eq!(result.len(), 4);
        assert!(result.contains(&"info".to_string()));
        assert!(result.contains(&"error".to_string()));
    }

    #[test]
    fn test_complete_field_value_partial_fuzzy() {
        let idx = make_index(&["level"], &[("level", &["info", "error", "warn"])]);
        let result = complete_field_value("level", "err", &idx);
        assert_eq!(result, vec!["error"]);
    }

    #[test]
    fn test_complete_field_value_unknown_field_returns_empty() {
        let idx = make_index(&[], &[]);
        let result = complete_field_value("nonexistent", "", &idx);
        assert!(result.is_empty());
    }
}
