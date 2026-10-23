use crate::config::init_schemas;
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug, Clone, Default)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Files to process. If none are given, reads from stdin. Each
    /// additional path beyond the first opens in its own tab (in --headless
    /// mode, its matching lines are appended to the output after the
    /// previous file's).
    pub files: Vec<String>,

    /// Path to a JSON filter file to preload (e.g. saved with :save-filters).
    #[arg(short = 'f', long)]
    pub filters: Option<String>,

    /// Start at the end of the file and enable tail mode.
    #[arg(long)]
    pub tail: bool,

    /// Add an include filter. Accepts the same arguments as the :filter command.
    /// May be repeated. Examples: -i "error"  or  -i "--bg Red --field level=ERROR"
    #[arg(
        short = 'i',
        long = "include",
        value_name = "ARGS",
        allow_hyphen_values = true
    )]
    pub include_filters: Vec<String>,

    /// Add an exclude filter. Accepts the same arguments as the :exclude command.
    /// May be repeated. Examples: -o "debug"  or  -o "--field level=debug"
    #[arg(
        short = 'o',
        long = "exclude",
        value_name = "ARGS",
        allow_hyphen_values = true
    )]
    pub exclude_filters: Vec<String>,

    /// Add a date/time range filter. Accepts the same arguments as :date-filter.
    /// May be repeated. Examples: -t "> 2024-02-21"  or  -t "01:00 .. 02:00"
    #[arg(
        short = 't',
        long = "timestamp",
        value_name = "ARGS",
        allow_hyphen_values = true
    )]
    pub timestamp_filters: Vec<String>,

    /// Enable the embedded MCP server on startup. Port defaults to 9876.
    /// Use --mcp for the default port or --mcp <PORT> for a custom port.
    #[arg(long, num_args = 0..=1, default_missing_value = "9876", value_name = "PORT")]
    pub mcp: Option<u16>,

    /// Run without an interface, write matching lines to stdout or --output.
    #[arg(long)]
    pub headless: bool,

    /// Write output to PATH instead of stdout (requires --headless or
    /// --merge). With --merge and no --headless, the resulting tab is saved
    /// to PATH immediately instead of staying temp-backed.
    #[arg(long, value_name = "PATH")]
    pub output: Option<std::path::PathBuf>,

    /// Execute a command and stream its output to a tab.
    /// The value is a quoted command string passed to the program directly
    /// (no shell); arguments are separated by whitespace.
    /// Example: logana --run "docker logs -f mycontainer"
    /// Stderr lines are prefixed with "ERROR " for visibility.
    #[arg(long, value_name = "COMMAND", conflicts_with = "files")]
    pub run: Option<String>,

    /// Directory to extract archive contents into, when a given path is an
    /// archive or a directory. Defaults to the current working directory.
    /// Extracted files are always written to real, permanent files — never
    /// temp files.
    #[arg(long, value_name = "DIR")]
    pub out: Option<std::path::PathBuf>,

    /// Interleave all input files by timestamp instead of concatenating
    /// them (or, without --headless, open the result as a tab). Requires
    /// at least 2 input files; archive inputs contribute one merge source
    /// per file inside the archive.
    #[arg(long)]
    pub merge: bool,
}

/// Resolves the `--out` flag to a concrete directory: the given path, or
/// the current working directory when omitted (falling back to `.` on the
/// rare system where the cwd can't be read).
pub fn resolve_out_dir(out: Option<&std::path::PathBuf>) -> std::path::PathBuf {
    out.cloned().unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    })
}

/// Expands any glob-pattern entry in `files` (e.g. `system.log*`) into its
/// sorted list of matching paths, leaving literal paths untouched. A pattern
/// that matches nothing is an error naming the pattern, since the user
/// clearly meant to reference files that don't exist rather than pass a
/// literal `*` through. A file that literally exists on disk is always
/// passed through as-is, even if its name contains `*`/`?`/`[`.
pub fn expand_cli_files(files: Vec<String>) -> std::result::Result<Vec<String>, String> {
    let mut expanded = Vec::with_capacity(files.len());
    for f in files {
        if crate::utils::filesystem::should_glob_expand(&f) {
            let matches =
                crate::utils::filesystem::expand_glob(&f).map_err(|e| format!("Error: {e}"))?;
            if matches.is_empty() {
                return Err(format!("Error: No files match '{}'.", f));
            }
            expanded.extend(matches);
        } else {
            expanded.push(f);
        }
    }
    Ok(expanded)
}

pub fn validate_file_arg(path: &str) -> std::result::Result<(), String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err(format!("'{}' not found.", path));
    }
    Ok(())
}

pub fn validate_inline_filter(prefix: &str, args_str: &str) -> std::result::Result<(), String> {
    use crate::commands::CommandLine;
    use crate::commands::auto_complete::shell_split;
    use clap::Parser as _;

    let cmd = format!("{} {}", prefix, args_str);
    CommandLine::try_parse_from(shell_split(&cmd))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn validate_startup_args(args: &Args) -> std::result::Result<(), String> {
    for path in &args.files {
        if let Err(msg) = validate_file_arg(path) {
            return Err(format!("Error: {}", msg));
        }
    }

    if let Some(ref fpath) = args.filters
        && let Err(msg) = validate_file_arg(fpath)
    {
        return Err(format!("Error (--filters): {}", msg));
    }
    for args_str in &args.include_filters {
        if let Err(msg) = validate_inline_filter("filter", args_str) {
            return Err(format!("Error (-i/--include): {}", msg));
        }
    }
    for args_str in &args.exclude_filters {
        if let Err(msg) = validate_inline_filter("exclude", args_str) {
            return Err(format!("Error (-o/--exclude): {}", msg));
        }
    }
    for args_str in &args.timestamp_filters {
        if let Err(msg) = validate_inline_filter("date-filter", args_str) {
            return Err(format!("Error (-t/--timestamp): {}", msg));
        }
    }

    if args.output.is_some() && !args.headless && !args.merge {
        return Err("Error: --output requires --headless or --merge.".to_string());
    }
    if args.merge {
        if args.files.len() < 2 {
            return Err(format!(
                "Error: --merge requires at least 2 input files, got {}.",
                args.files.len()
            ));
        }
        for path in &args.files {
            if std::path::Path::new(path).is_dir() {
                return Err(format!(
                    "Error: '{}' is a directory. --merge requires file paths.",
                    path
                ));
            }
        }
    }

    Ok(())
}

pub async fn run_headless_mode(args: Args) -> Result<()> {
    for path in &args.files {
        if std::path::Path::new(path).is_dir() {
            eprintln!(
                "Error: '{}' is a directory. --headless requires file paths or stdin.",
                path
            );
            std::process::exit(1);
        }
    }

    let mut schema_warnings = init_schemas();
    schema_warnings.extend(crate::parser::validate_custom_schemas(
        crate::config::custom_schemas(),
    ));
    for warning in &schema_warnings {
        eprintln!("Warning: {warning}");
    }

    let out_dir = resolve_out_dir(args.out.as_ref());
    crate::headless::run_headless(&crate::headless::HeadlessArgs {
        files: args.files,
        filters: args.filters,
        include_filters: args.include_filters,
        exclude_filters: args.exclude_filters,
        timestamp_filters: args.timestamp_filters,
        output: args.output,
        out_dir,
        merge: args.merge,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_frontends_accept_the_full_argument_set() {
        for program in ["logana", "logana-gui"] {
            let args = Args::try_parse_from([
                program,
                "--tail",
                "-f",
                "filters.json",
                "-i",
                "--field level=ERROR",
                "-i",
                "failure",
                "-o",
                "debug",
                "-t",
                "> 2024-02-21",
                "--mcp",
                "9877",
                "--headless",
                "--merge",
                "--out",
                "extracted",
                "--output",
                "merged.log",
                "logs/東京 log.log",
                "bundle.tar.gz",
            ])
            .unwrap();
            assert!(args.tail && args.headless && args.merge);
            assert_eq!(args.files, ["logs/東京 log.log", "bundle.tar.gz"]);
            assert_eq!(args.include_filters, ["--field level=ERROR", "failure"]);
            assert_eq!(args.exclude_filters, ["debug"]);
            assert_eq!(args.timestamp_filters, ["> 2024-02-21"]);
            assert_eq!(args.mcp, Some(9877));
            assert_eq!(args.filters.as_deref(), Some("filters.json"));
            assert_eq!(args.out.unwrap(), std::path::PathBuf::from("extracted"));
            assert_eq!(args.output.unwrap(), std::path::PathBuf::from("merged.log"));
        }
    }

    #[test]
    fn defaults_run_conflicts_and_dash_paths_are_shared() {
        assert_eq!(
            Args::try_parse_from(["logana-gui", "--mcp"]).unwrap().mcp,
            Some(9876)
        );
        assert_eq!(
            Args::try_parse_from(["logana-gui", "--", "-app.log"])
                .unwrap()
                .files,
            ["-app.log"]
        );
        assert!(Args::try_parse_from(["logana-gui", "--run", "echo hello", "app.log"]).is_err());
        assert_eq!(
            Args::try_parse_from(["logana-gui", "--run", "echo hello"])
                .unwrap()
                .run
                .as_deref(),
            Some("echo hello")
        );
    }
}
