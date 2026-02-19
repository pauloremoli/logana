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
    fn test_shell_split_mixed_args() {
        assert_eq!(
            shell_split(r#"filter "my pattern" --fg Red --bg Blue"#),
            vec!["filter", "my pattern", "--fg", "Red", "--bg", "Blue"]
        );
    }

    #[test]
    fn test_complete_color() {
        let results = complete_color("Re");
        assert!(results.contains(&"Red"));
        assert!(results.contains(&"Reset"));
    }

    #[test]
    fn test_complete_color_empty() {
        let results = complete_color("");
        assert_eq!(results.len(), COLOR_NAMES.len());
    }

    #[test]
    fn test_find_command_completions_empty() {
        let results = find_command_completions("");
        assert!(!results.is_empty());
    }

    #[test]
    fn test_find_command_completions_prefix() {
        let results = find_command_completions("fi");
        assert!(results.contains(&"filter"));
    }

    #[test]
    fn test_extract_color_partial_fg() {
        assert_eq!(extract_color_partial("filter --fg Re"), Some("Re"));
        assert_eq!(extract_color_partial("set-color --bg "), Some(""));
    }

    #[test]
    fn test_extract_color_partial_none() {
        assert_eq!(extract_color_partial("filter foo"), None);
        assert_eq!(extract_color_partial("exclude --fg Red"), None);
    }
}
