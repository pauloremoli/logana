use clap::{Parser, Subcommand};

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
        pattern: String,
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color to the whole line instead of only the matched text
        #[arg(short = 'l')]
        line_mode: bool,
        /// Treat pattern as key=value and match against the named parsed field
        #[arg(long = "field", short = 'f')]
        field: bool,
    },
    /// Add an exclude filter
    Exclude {
        pattern: String,
        /// Treat pattern as key=value and match against the named parsed field
        #[arg(long = "field", short = 'f')]
        field: bool,
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
    /// Set the theme
    SetTheme { theme_name: String },
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
        #[arg(long, default_value = "9876")]
        port: u16,
    },
    /// Stop the embedded MCP server
    DisableMcp,
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
