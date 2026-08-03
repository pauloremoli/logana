pub struct CommandInfo {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
    /// Example invocations, one per entry — rendered as a separate line
    /// each in the command-help popup instead of crammed into `description`.
    pub examples: &'static [&'static str],
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "filter",
        usage: "filter [-r] [-i] [-l] [--fg <color>] [--bg <color>] [-a] [--group <name>] [--field <key>=<value>] <pattern>",
        description: "Add an include filter — only matching lines stay visible.",
        examples: &[
            "filter ERROR",
            "filter connection refused",
            "filter -r \"ERR(OR)?\"",
            "filter --ignore-case error",
            "filter --field level=error",
            "filter --group errors ERROR",
            "filter --auto ERROR",
        ],
    },
    CommandInfo {
        name: "exclude",
        usage: "exclude [-r] [-i] [--group <name>] [--field <key>=<value>] <pattern>",
        description: "Add an exclude filter — matching lines are hidden.",
        examples: &[
            "exclude debug",
            "exclude connection refused",
            "exclude -r \"health.?check\"",
            "exclude --ignore-case debug",
            "exclude --field level=debug",
            "exclude --group noise debug",
        ],
    },
    CommandInfo {
        name: "highlight",
        usage: "highlight [-r] [-i] [-l] [--fg <color>] [--bg <color>] [-a] [--group <name>] [--field <key>=<value>] <pattern>",
        description: "Add a highlight filter — colors matches without hiding other lines.",
        examples: &[
            "highlight ERROR",
            "highlight --field level=error",
            "highlight --auto ERROR",
            "h ERROR",
        ],
    },
    CommandInfo {
        name: "set-color",
        usage: "set-color [-l] [--fg <color>] [--bg <color>]",
        description: "Set color for the selected filter. -l colors the whole line.",
        examples: &["set-color --fg Green", "set-color --fg [0,255,0]"],
    },
    CommandInfo {
        name: "export-marked",
        usage: "export-marked <path>",
        description: "Export marked logs to a file.",
        examples: &["export-marked /tmp/marked.log"],
    },
    CommandInfo {
        name: "save",
        usage: "save <path>",
        description: "Save visible (filtered) lines to a file in raw format.",
        examples: &["save /tmp/visible.log"],
    },
    CommandInfo {
        name: "save-filters",
        usage: "save-filters <path>",
        description: "Save current filters to a file.",
        examples: &["save-filters filters.json"],
    },
    CommandInfo {
        name: "load-filters",
        usage: "load-filters <path>",
        description: "Load filters from a file.",
        examples: &["load-filters filters.json"],
    },
    CommandInfo {
        name: "wrap",
        usage: "wrap",
        description: "Toggle line wrapping on/off.",
        examples: &[],
    },
    CommandInfo {
        name: "set-theme",
        usage: "set-theme <name>",
        description: "Change the color theme.",
        examples: &["set-theme dracula"],
    },
    CommandInfo {
        name: "level-colors",
        usage: "level-colors",
        description: "Toggle ERROR/WARN log level color highlighting on/off.",
        examples: &[],
    },
    CommandInfo {
        name: "open",
        usage: "open <path>",
        description: "Open a file in a new tab.",
        examples: &["open /var/log/syslog"],
    },
    CommandInfo {
        name: "close-tab",
        usage: "close-tab",
        description: "Close the current tab (quits if last tab).",
        examples: &[],
    },
    CommandInfo {
        name: "line-numbers",
        usage: "line-numbers",
        description: "Toggle line numbers on/off.",
        examples: &[],
    },
    CommandInfo {
        name: "clear-filters",
        usage: "clear-filters",
        description: "Remove all filter definitions.",
        examples: &[],
    },
    CommandInfo {
        name: "disable-filters",
        usage: "disable-filters",
        description: "Disable all filters without removing them.",
        examples: &[],
    },
    CommandInfo {
        name: "enable-filters",
        usage: "enable-filters",
        description: "Enable all disabled filters.",
        examples: &[],
    },
    CommandInfo {
        name: "toggle-group",
        usage: "toggle-group <name>",
        description: "Toggle all filters in a named group on/off together.",
        examples: &["toggle-group errors"],
    },
    CommandInfo {
        name: "filtering",
        usage: "filtering",
        description: "Toggle global filtering on/off (bypass all filters).",
        examples: &[],
    },
    CommandInfo {
        name: "hide-field",
        usage: "hide-field <name|index>",
        description: "Hide a field by name or 0-based index.",
        examples: &["hide-field level", "hide-field 0"],
    },
    CommandInfo {
        name: "show-field",
        usage: "show-field <name>",
        description: "Show a previously hidden field.",
        examples: &["show-field level"],
    },
    CommandInfo {
        name: "show-all-fields",
        usage: "show-all-fields",
        description: "Clear all hidden fields and show all fields.",
        examples: &[],
    },
    CommandInfo {
        name: "select-fields",
        usage: "select-fields",
        description: "Open a modal to select which fields to display and their order.",
        examples: &[],
    },
    CommandInfo {
        name: "merge",
        usage: "merge",
        description: "Open a popup to select tabs and create a new interleaved view sorted by timestamp.",
        examples: &[],
    },
    CommandInfo {
        name: "docker",
        usage: "docker",
        description: "List running Docker containers and stream logs from the selected one.",
        examples: &[],
    },
    CommandInfo {
        name: "value-colors",
        usage: "value-colors",
        description: "Toggle value-based color coding (HTTP methods, status codes, IPs, UUIDs).",
        examples: &[],
    },
    CommandInfo {
        name: "export",
        usage: "export [-t <template>] <path>",
        description: "Export analysis (comments + marked lines) to a file. -t sets the template (default: markdown).",
        examples: &["export /tmp/report.md"],
    },
    CommandInfo {
        name: "date-filter",
        usage: "date-filter <expression>",
        description: "Filter lines by timestamp.",
        examples: &[
            "date-filter 01:00 .. 02:00",
            "date-filter > 2024-02-22",
            "date-filter >= Feb 21",
        ],
    },
    CommandInfo {
        name: "tail",
        usage: "tail",
        description: "Toggle tail mode — always scrolls to the last line as new content arrives.",
        examples: &[],
    },
    CommandInfo {
        name: "show-keys",
        usage: "show-keys",
        description: "Show field keys alongside values in structured log display (e.g. method=GET instead of GET).",
        examples: &[],
    },
    CommandInfo {
        name: "hide-keys",
        usage: "hide-keys",
        description: "Show only values in structured log display, hiding field keys (default).",
        examples: &[],
    },
    CommandInfo {
        name: "raw",
        usage: "raw",
        description: "Toggle raw mode — disables the format parser and shows unformatted log lines.",
        examples: &[],
    },
    CommandInfo {
        name: "stop",
        usage: "stop",
        description: "Stop all incoming data for the current tab (file watcher and/or stream).",
        examples: &[],
    },
    CommandInfo {
        name: "pause",
        usage: "pause",
        description: "Pause applying incoming data to the view (watcher/stream keeps running in the background).",
        examples: &[],
    },
    CommandInfo {
        name: "resume",
        usage: "resume",
        description: "Resume applying incoming data after a pause.",
        examples: &[],
    },
    CommandInfo {
        name: "reset",
        usage: "reset",
        description: "Restore all settings to defaults and clear all persisted state.",
        examples: &[],
    },
    CommandInfo {
        name: "dlt",
        usage: "dlt",
        description: "Show configured DLT devices and connect to one.",
        examples: &[],
    },
    CommandInfo {
        name: "otel",
        usage: "otel [--http] [port]",
        description: "Start an OTel collector receiver. Default: gRPC on port 4317; use --http for HTTP/JSON on port 4318.",
        examples: &["otel", "otel 4317", "otel --http", "otel --http 4318"],
    },
    CommandInfo {
        name: "enable-mcp",
        usage: "enable-mcp [--port <port>]",
        description: "Start the embedded MCP server (default port 9876).",
        examples: &["enable-mcp --port 8080"],
    },
    CommandInfo {
        name: "disable-mcp",
        usage: "disable-mcp",
        description: "Stop the embedded MCP server.",
        examples: &[],
    },
    CommandInfo {
        name: "run",
        usage: "run <program> [args...]",
        description: "Execute a command and stream its output to a new tab. Stderr lines are shown as errors.",
        examples: &[
            "run docker logs -f mycontainer",
            "run tail -f /var/log/syslog",
        ],
    },
    CommandInfo {
        name: "sidebar-position",
        usage: "sidebar-position <left|right>",
        description: "Move the filter sidebar to the left or right of the log panel.",
        examples: &["sidebar-position left"],
    },
    CommandInfo {
        name: "schema",
        usage: "schema [name]",
        description: "Show the active schema, or switch this tab to a named custom or built-in schema. Use 'none' to clear the schema and treat the file as plain text. Autocomplete lists every available one.",
        examples: &["schema", "schema acme", "schema none"],
    },
    CommandInfo {
        name: "default-filters",
        usage: "default-filters [format] [path]",
        description: "Configure a filter file to auto-load whenever a format is assigned to a tab with no filters yet. No args opens a popup listing every format; <format> alone clears its mapping. Never retroactively affects the tab you're currently on.",
        examples: &[
            "default-filters",
            "default-filters acme ~/logs/filters/acme.json",
            "default-filters acme",
        ],
    },
];

/// Commands whose last argument is a file path and should receive path auto-completion.
pub const FILE_PATH_COMMANDS: &[&str] = &[
    "open",
    "load-filters",
    "save-filters",
    "export-marked",
    "export",
    "save",
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
            "reset",
            "schema",
            "default-filters",
        ] {
            assert!(names.contains(expected), "missing command: {expected}");
        }
    }

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
    fn test_find_matching_command_highlight() {
        let cmd = find_matching_command("highlight").unwrap();
        assert_eq!(cmd.name, "highlight");
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
    fn test_find_matching_command_default_filters() {
        let cmd = find_matching_command("default-filters").unwrap();
        assert_eq!(cmd.name, "default-filters");
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
        assert!(!cmd.examples.is_empty());
    }

    #[test]
    fn test_every_command_has_non_empty_usage_and_description() {
        for cmd in COMMANDS {
            assert!(!cmd.usage.is_empty(), "{} has empty usage", cmd.name);
            assert!(
                !cmd.description.is_empty(),
                "{} has empty description",
                cmd.name
            );
        }
    }
}
