use clap::{Parser, Subcommand};

use crate::ui::SidebarSide;

#[derive(Parser, Debug)]
#[command(author, version, about, no_binary_name = true)]
pub struct CommandLine {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Add an include filter
    Filter {
        #[arg(trailing_var_arg = true)]
        pattern: Vec<String>,
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color to the whole line instead of only the matched text
        #[arg(short = 'l')]
        line_mode: bool,
        /// Match a 'key=value' parsed field — repeat to require several
        /// fields at once (AND); any trailing pattern text is also required
        /// (AND), matched against the full line.
        #[arg(long = "field", short = 'f')]
        field: Vec<String>,
        /// Treat pattern as a regular expression instead of a literal string
        #[arg(long = "regex", short = 'r')]
        regex: bool,
        /// Match regardless of case
        #[arg(long = "ignore-case", short = 'i')]
        ignore_case: bool,
        /// Assign the filter to a named group, so it can be toggled together with others
        #[arg(long, short = 'g')]
        group: Option<String>,
        /// Generate a random readable fg/bg color pair instead of specifying --fg/--bg
        #[arg(long = "auto", short = 'a')]
        auto: bool,
    },
    /// Add an exclude filter
    Exclude {
        #[arg(trailing_var_arg = true)]
        pattern: Vec<String>,
        /// Match a 'key=value' parsed field — repeat to require several
        /// fields at once (AND); any trailing pattern text is also required
        /// (AND), matched against the full line.
        #[arg(long = "field", short = 'f')]
        field: Vec<String>,
        /// Treat pattern as a regular expression instead of a literal string
        #[arg(long = "regex", short = 'r')]
        regex: bool,
        /// Match regardless of case
        #[arg(long = "ignore-case", short = 'i')]
        ignore_case: bool,
        /// Assign the filter to a named group, so it can be toggled together with others
        #[arg(long, short = 'g')]
        group: Option<String>,
    },
    /// Add a highlight filter (styling only, never affects visibility)
    #[command(alias = "h")]
    Highlight {
        #[arg(trailing_var_arg = true)]
        pattern: Vec<String>,
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color to the whole line instead of only the matched text
        #[arg(short = 'l')]
        line_mode: bool,
        /// Match a 'key=value' parsed field — repeat to require several
        /// fields at once (AND); any trailing pattern text is also required
        /// (AND), matched against the full line.
        #[arg(long = "field", short = 'f')]
        field: Vec<String>,
        /// Treat pattern as a regular expression instead of a literal string
        #[arg(long = "regex", short = 'r')]
        regex: bool,
        /// Match regardless of case
        #[arg(long = "ignore-case", short = 'i')]
        ignore_case: bool,
        /// Assign the filter to a named group, so it can be toggled together with others
        #[arg(long, short = 'g')]
        group: Option<String>,
        /// Generate a random readable fg/bg color pair instead of specifying --fg/--bg
        #[arg(long = "auto", short = 'a')]
        auto: bool,
    },
    /// Set color for the selected filter
    SetColor {
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color to the whole line instead of only the matched text
        #[arg(short = 'l')]
        line_mode: bool,
    },
    /// Export marked logs
    ExportMarked { path: String },
    /// Save visible (filtered) lines to file in raw format
    Save { path: String },
    /// Save filters to file
    SaveFilters { path: String },
    /// Load filters from file
    LoadFilters { path: String },
    /// Toggle line wrapping
    Wrap,
    /// Toggle line numbers
    LineNumbers,
    /// Toggle relative line numbers
    RelativeLineNumbers,
    /// Collapse continuation lines to their entry's first line
    Collapse,
    /// Expand continuation lines previously hidden by `collapse`
    Expand,
    /// Set the theme
    SetTheme { theme_name: String },
    /// Open a picker to preview and switch the color theme
    Theme,
    /// Toggle log level color highlighting
    LevelColors,
    /// Open a file in a new tab
    Open { path: String },
    /// Close the current tab
    CloseTab,
    /// Remove all filter definitions
    ClearFilters,
    /// Disable all filters without removing them
    DisableFilters,
    /// Enable all filters
    EnableFilters,
    /// Toggle all filters in a named group on/off together
    ToggleGroup { name: String },
    /// Set, update, or clear the predefined visual style for a filter group
    Group {
        name: String,
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color to the whole line instead of only the matched text
        #[arg(short = 'l')]
        line_mode: bool,
        /// Generate a random readable fg/bg color pair instead of specifying --fg/--bg
        #[arg(long = "auto", short = 'a')]
        auto: bool,
        /// Remove the group's predefined style
        #[arg(long)]
        clear: bool,
    },
    /// Toggle global filtering on/off
    Filtering,
    /// Hide a JSON field by name or 0-based index
    HideField { field: String },
    /// Show a hidden JSON field by name or 0-based index
    ShowField { field: String },
    /// Clear all hidden fields
    ShowAllFields,
    /// Open a modal to select which JSON fields to display
    SelectFields,
    /// Open a popup to merge multiple tabs into a new interleaved view
    Merge,
    /// List running Docker containers and attach to one
    Docker,
    /// Toggle value-based color coding (HTTP methods, status codes, IPs, UUIDs)
    ValueColors,
    /// Export analysis (comments + marked lines) using a template
    Export {
        path: String,
        #[arg(short, long, default_value = "markdown")]
        template: String,
    },
    /// Filter log lines by timestamp range or comparison
    DateFilter {
        /// Date filter expression (e.g. "01:00 .. 02:00", "> 2024-02-22")
        expr: Vec<String>,
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color to the whole line instead of only the timestamp
        #[arg(short = 'l')]
        line_mode: bool,
    },
    /// Toggle tail mode (always scroll to last line on new content)
    Tail,
    /// Show field keys alongside values (e.g. key=value) in structured log display
    ShowKeys,
    /// Hide field keys and show only values in structured log display
    HideKeys,
    /// Toggle raw mode — disable the format parser and show unformatted log lines
    Raw,
    /// Stop all incoming data for the current tab (file watcher and/or stream)
    Stop,
    /// Pause applying incoming data to the view (watcher/stream keeps running)
    Pause,
    /// Resume applying incoming data after a pause
    Resume,
    /// Restore all settings to defaults and clear all persisted state
    Reset,
    /// Show configured DLT devices and connect to one
    Dlt,
    /// Start an OTel collector receiver (gRPC by default on port 4317; use --http for HTTP on port 4318)
    Otel {
        /// Use HTTP/JSON transport instead of gRPC
        #[arg(long)]
        http: bool,
        /// Port to listen on (defaults to 4317 for gRPC, 4318 for --http)
        port: Option<u16>,
    },
    /// Start the embedded MCP server on the given port
    EnableMcp {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Stop the embedded MCP server
    DisableMcp,
    /// Show the active parser schema for this tab, or switch to a named schema.
    #[command(name = "schema")]
    Schema {
        /// Schema name to activate. Omit to display the current schema name.
        name: Option<String>,
    },
    /// Configure a filter file to auto-load for a format. Omit both to open
    /// a popup listing every format; give only `format` to clear its mapping.
    #[command(name = "default-filters")]
    DefaultFilters {
        /// Format name (custom schema or built-in). Omit to open the popup.
        format: Option<String>,
        /// Filter file path. Omit (with `format` given) to clear that format's mapping.
        path: Option<String>,
    },
    /// Set the filter sidebar position relative to the log panel (left or right)
    SidebarPosition { side: SidebarSide },
    /// Execute a command and stream its output to a new tab.
    /// Provide the program followed by its arguments.
    /// Stderr lines are prefixed with "ERROR " for visibility.
    /// Example: :run docker logs -f mycontainer
    Run {
        /// Program and its arguments
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}
