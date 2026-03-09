//! Command registry: metadata and lookup for all TUI commands.
//!
//! [`COMMANDS`] is the authoritative list of all available commands.
//! [`find_matching_command`] resolves a command-line input string to its
//! [`CommandInfo`] entry. [`FILE_PATH_COMMANDS`] lists commands whose last
//! argument expects a file path (used by tab completion).

pub struct CommandInfo {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "filter",
        usage: "filter [-l] [--fg <color>] [--bg <color>] <pattern>",
        description: "Add an include filter. -l colors the whole line. e.g. filter --fg Red error, filter --fg [255,0,0] error",
    },
    CommandInfo {
        name: "exclude",
        usage: "exclude <pattern>",
        description: "Add an exclude filter. e.g. exclude debug",
    },
    CommandInfo {
        name: "set-color",
        usage: "set-color [-l] [--fg <color>] [--bg <color>]",
        description: "Set color for the selected filter. -l colors the whole line. e.g. set-color --fg Green, set-color --fg [0,255,0]",
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
    CommandInfo {
        name: "show-keys",
        usage: "show-keys",
        description: "Show field keys alongside values in structured log display (e.g. method=GET instead of GET)",
    },
    CommandInfo {
        name: "hide-keys",
        usage: "hide-keys",
        description: "Show only values in structured log display, hiding field keys (default)",
    },
    CommandInfo {
        name: "raw",
        usage: "raw",
        description: "Toggle raw mode — disables the format parser and shows unformatted log lines",
    },
    CommandInfo {
        name: "stop",
        usage: "stop",
        description: "Stop all incoming data for the current tab (file watcher and/or stream)",
    },
    CommandInfo {
        name: "pause",
        usage: "pause",
        description: "Pause applying incoming data to the view (watcher/stream keeps running in the background)",
    },
    CommandInfo {
        name: "resume",
        usage: "resume",
        description: "Resume applying incoming data after a pause",
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
}
