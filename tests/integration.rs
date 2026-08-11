use clap::Parser;
use logana::commands::auto_complete::shell_split;
use logana::commands::{CommandLine, Commands};
use logana::db::Database;
use logana::db::LogManager;
use logana::db::MarkManager;
use logana::filters::FilterManager;
use logana::filters::{FilterOptions, FilterType};
use logana::headless::run_headless_to_writer;
use logana::ingestion::FileReader;
use logana::ui::SidebarSide;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;

async fn setup() -> (Arc<Database>, LogManager) {
    let db = Database::in_memory().await.unwrap();
    let db = Arc::new(db);
    let manager = LogManager::new(db.clone(), None).await;
    (db, manager)
}

fn create_sample_log_file() -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        "Jun 28 10:00:01 myhost myapp[1234]: INFO: Application started"
    )
    .unwrap();
    writeln!(
        file,
        "Jun 28 10:00:02 myhost myapp[1234]: ERROR: Connection failed"
    )
    .unwrap();
    writeln!(
        file,
        "Jun 28 10:00:03 myhost kernel: Linux version 6.8.0-31-generic"
    )
    .unwrap();
    writeln!(
        file,
        "Jun 28 10:00:04 myhost myapp[1234]: WARNING: Retrying connection"
    )
    .unwrap();
    writeln!(
        file,
        "Jun 28 10:00:05 myhost myapp[1234]: INFO: Connection established"
    )
    .unwrap();
    writeln!(
        file,
        "Jun 28 10:00:06 myhost sshd[5678]: DEBUG: Key exchange completed"
    )
    .unwrap();
    writeln!(file, "plain text log line with no format").unwrap();
    file
}

#[test]
fn test_file_reader_line_count() {
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();
    assert_eq!(reader.line_count(), 7);
}

#[test]
fn test_file_reader_get_line() {
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    let line0 = std::str::from_utf8(reader.get_line(0)).unwrap();
    assert!(line0.contains("INFO"));
    assert!(line0.contains("Application started"));

    let line1 = std::str::from_utf8(reader.get_line(1)).unwrap();
    assert!(line1.contains("ERROR"));
    assert!(line1.contains("Connection failed"));

    let line6 = std::str::from_utf8(reader.get_line(6)).unwrap();
    assert_eq!(line6, "plain text log line with no format");
}

#[tokio::test]
async fn test_filter_include_reduces_visible() {
    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    // No filters → all lines visible
    let (fm, _, _, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);
    assert_eq!(visible.len(), 7);

    // Include only lines containing "Connection"
    manager
        .add_filter_with_color(
            "Connection".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);
    assert_eq!(visible.len(), 2);
    // Lines 1 and 4 contain "Connection"
    assert!(visible.contains(&1));
    assert!(visible.contains(&4));
}

#[tokio::test]
async fn test_filter_exclude_removes_lines() {
    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    // Exclude lines containing "INFO"
    manager
        .add_filter_with_color("INFO".into(), FilterType::Exclude, FilterOptions::default())
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);

    // Lines 0 and 4 contain "INFO"; 7 total - 2 = 5
    assert_eq!(visible.len(), 5);
    assert!(!visible.contains(&0));
    assert!(!visible.contains(&4));
}

#[tokio::test]
async fn test_filter_include_and_exclude() {
    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    // Add "Connection" (Include) first, then "failed" (Exclude) second.
    // With oldest-first ordering, "Connection" Include is at index 0 (highest precedence).
    // First-match-wins: "Connection failed" matches the Include first → visible.
    manager
        .add_filter_with_color(
            "Connection".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    manager
        .add_filter_with_color(
            "failed".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);

    // Line 1: "Connection failed" — "Connection" Include matches first → visible
    // Line 4: "Connection established" — "Connection" Include matches → visible
    assert_eq!(visible.len(), 2);
    assert!(visible.contains(&1));
    assert!(visible.contains(&4));
}

#[tokio::test]
async fn test_disabled_filter_is_ignored() {
    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    manager
        .add_filter_with_color("INFO".into(), FilterType::Include, FilterOptions::default())
        .await;
    let id = manager.get_filters()[0].id;
    manager.toggle_filter(id).await; // disable it

    // Disabled → no active include filters → all lines visible
    let (fm, _, _, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);
    assert_eq!(visible.len(), 7);
}

#[test]
fn test_filter_manager_no_filters_shows_all() {
    let fm = FilterManager::empty();
    let data = b"line1\nline2\nline3\n";
    let reader = FileReader::from_bytes(data.to_vec());
    let visible = fm.compute_visible(&reader);
    assert_eq!(visible, vec![0, 1, 2]);
}

#[test]
fn test_marks_persistence() {
    let mut marks = MarkManager::default();

    marks.toggle(0);
    marks.toggle(2);
    marks.toggle(5);

    assert!(marks.is_marked(0));
    assert!(marks.is_marked(2));
    assert!(marks.is_marked(5));
    assert!(!marks.is_marked(1));
    assert!(!marks.is_marked(3));

    let indices = marks.get_indices();
    assert_eq!(indices, vec![0, 2, 5]);

    // Toggle off
    marks.toggle(2);
    assert!(!marks.is_marked(2));
    assert_eq!(marks.get_indices(), vec![0, 5]);
}

#[test]
fn test_get_marked_lines() {
    let data = b"alpha\nbeta\ngamma\n";
    let reader = FileReader::from_bytes(data.to_vec());

    let mut marks = MarkManager::default();
    marks.toggle(0);
    marks.toggle(2);

    let lines = marks.get_lines(&reader);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], b"alpha");
    assert_eq!(lines[1], b"gamma");
}

#[tokio::test]
async fn test_add_and_remove_filters() {
    let (_db, mut manager) = setup().await;

    manager
        .add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    manager
        .add_filter_with_color(
            "debug".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;
    assert_eq!(manager.get_filters().len(), 2);

    // Oldest first: "error" is at index 0; removing it leaves "debug"
    let id = manager.get_filters()[0].id;
    manager.remove_filter(id).await;
    assert_eq!(manager.get_filters().len(), 1);
    assert_eq!(manager.get_filters()[0].pattern, "debug");
}

#[tokio::test]
async fn test_move_filter_up_down() {
    let (_db, mut manager) = setup().await;

    manager
        .add_filter_with_color(
            "first".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    manager
        .add_filter_with_color(
            "second".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    manager
        .add_filter_with_color(
            "third".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

    // After three inserts (oldest first): ["first", "second", "third"]
    // "second" is at index 1; move_filter_up swaps [0] and [1]
    let id_second = manager.get_filters()[1].id;
    manager.move_filter_up(id_second).await;

    // Result: ["second", "first", "third"]
    let filters = manager.get_filters();
    assert_eq!(filters[0].pattern, "second");
    assert_eq!(filters[1].pattern, "first");
    assert_eq!(filters[2].pattern, "third");
}

#[tokio::test]
async fn test_filter_regex_pattern() {
    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    // Regex pattern matching either INFO or ERROR
    manager
        .add_filter_with_color(
            "INFO|ERROR".into(),
            FilterType::Include,
            FilterOptions::default().regex(),
        )
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);

    // Lines 0 (INFO), 1 (ERROR), 4 (INFO) → 3 lines
    assert_eq!(visible.len(), 3);
    assert!(visible.contains(&0));
    assert!(visible.contains(&1));
    assert!(visible.contains(&4));
}

#[test]
fn test_file_reader_from_bytes() {
    let data = b"line one\nline two\nline three\n";
    let reader = FileReader::from_bytes(data.to_vec());
    assert_eq!(reader.line_count(), 3);
    assert_eq!(reader.get_line(0), b"line one");
    assert_eq!(reader.get_line(1), b"line two");
    assert_eq!(reader.get_line(2), b"line three");
}

#[tokio::test]
async fn test_clear_filters() {
    let (_db, mut manager) = setup().await;
    manager
        .add_filter_with_color(
            "error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    manager
        .add_filter_with_color(
            "debug".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;
    assert_eq!(manager.get_filters().len(), 2);

    manager.clear_filters().await;
    assert!(manager.get_filters().is_empty());
}

#[tokio::test]
async fn test_single_pass_predicate_matches_compute_visible() {
    // Verify that the single-pass visible-line computation (VisibilityPredicate
    // evaluated during indexing) produces the same result as compute_visible run
    // after the load completes — the two paths must be equivalent.
    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap().to_string();

    manager
        .add_filter_with_color("INFO".into(), FilterType::Include, FilterOptions::default())
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();

    // Post-load path: compute_visible on the already-indexed reader.
    let reader = FileReader::new(&path).unwrap();
    let expected = fm.compute_visible(&reader);

    // Single-pass path: predicate evaluated during indexing.
    let pred = logana::ingestion::VisibilityPredicate::new(fm);
    let handle = FileReader::load(
        path,
        Some(pred),
        false,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        false,
    )
    .await
    .unwrap();
    let result = handle.result_rx.await.unwrap().unwrap();
    let precomputed = result.precomputed_visible.unwrap();

    assert_eq!(precomputed, expected);
}

#[tokio::test]
async fn test_search_on_visible_lines() {
    use logana::utils::search::Search;

    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    // Include only INFO lines
    manager
        .add_filter_with_color("INFO".into(), FilterType::Include, FilterOptions::default())
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);
    assert_eq!(visible.len(), 2);

    // Search for "Application" within visible lines only
    let mut search = Search::new();
    search
        .search("Application", visible.iter().copied(), |li| {
            Some(String::from_utf8_lossy(reader.get_line(li)).into_owned())
        })
        .unwrap();
    let results = search.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].line_idx, 0);
}

#[tokio::test]
async fn test_field_filter_level_include() {
    let (_db, mut manager) = setup().await;
    manager
        .add_filter_with_color(
            "@field:level:error".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

    let reader = FileReader::from_bytes(
        b"{\"level\":\"info\",\"msg\":\"starting up\"}\n\
          {\"level\":\"error\",\"msg\":\"something failed\"}\n\
          {\"level\":\"debug\",\"msg\":\"verbose output\"}\n"
            .to_vec(),
    );
    let mut out = Vec::new();
    run_headless_to_writer(reader, &manager, &mut out).unwrap();
    let result = String::from_utf8(out).unwrap();
    assert!(result.contains("something failed"));
    assert!(!result.contains("starting up"));
    assert!(!result.contains("verbose output"));
}

#[tokio::test]
async fn test_field_filter_level_exclude() {
    let (_db, mut manager) = setup().await;
    manager
        .add_filter_with_color(
            "@field:level:debug".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;

    let reader = FileReader::from_bytes(
        b"{\"level\":\"info\",\"msg\":\"starting up\"}\n\
          {\"level\":\"error\",\"msg\":\"something failed\"}\n\
          {\"level\":\"debug\",\"msg\":\"verbose output\"}\n"
            .to_vec(),
    );
    let mut out = Vec::new();
    run_headless_to_writer(reader, &manager, &mut out).unwrap();
    let result = String::from_utf8(out).unwrap();
    assert!(result.contains("starting up"));
    assert!(result.contains("something failed"));
    assert!(!result.contains("verbose output"));
}

#[tokio::test]
async fn test_headless_multiple_includes_or_semantics() {
    let (_db, mut manager) = setup().await;
    manager
        .add_filter_with_color(
            "ERROR".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    manager
        .add_filter_with_color(
            "WARNING".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

    let file = create_sample_log_file();
    let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
    let mut out = Vec::new();
    run_headless_to_writer(reader, &manager, &mut out).unwrap();
    let result = String::from_utf8(out).unwrap();
    assert!(result.contains("ERROR"));
    assert!(result.contains("WARNING"));
    assert!(!result.contains("INFO"));
    assert!(!result.contains("DEBUG"));
}

#[tokio::test]
async fn test_headless_regex_filter() {
    let (_db, mut manager) = setup().await;
    manager
        .add_filter_with_color(
            "INFO|ERROR".into(),
            FilterType::Include,
            FilterOptions::default().regex(),
        )
        .await;

    let file = create_sample_log_file();
    let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
    let mut out = Vec::new();
    run_headless_to_writer(reader, &manager, &mut out).unwrap();
    let lines: Vec<&str> = out
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| std::str::from_utf8(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert!(
        lines
            .iter()
            .all(|l| l.contains("INFO") || l.contains("ERROR"))
    );
}

#[tokio::test]
async fn test_headless_no_matching_lines() {
    let (_db, mut manager) = setup().await;
    manager
        .add_filter_with_color(
            "CRITICAL".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

    let reader = FileReader::from_bytes(b"INFO foo\nDEBUG bar\nERROR baz\n".to_vec());
    let mut out = Vec::new();
    run_headless_to_writer(reader, &manager, &mut out).unwrap();
    assert!(out.is_empty());
}

#[tokio::test]
async fn test_headless_exclude_before_include() {
    // Exclude added first → wins over include for overlapping lines (first-match-wins).
    let (_db, mut manager) = setup().await;
    manager
        .add_filter_with_color(
            "established".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;
    manager
        .add_filter_with_color(
            "Connection".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;

    let file = create_sample_log_file();
    let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
    let mut out = Vec::new();
    run_headless_to_writer(reader, &manager, &mut out).unwrap();
    let result = String::from_utf8(out).unwrap();
    assert!(result.contains("Connection failed"));
    assert!(!result.contains("Connection established"));
}

#[tokio::test]
async fn test_headless_filter_file_roundtrip() {
    let filter_file = NamedTempFile::new().unwrap();
    let filter_path = filter_file.path().to_str().unwrap().to_string();

    {
        let (_db, mut manager) = setup().await;
        manager
            .add_filter_with_color(
                "ERROR".into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        manager
            .add_filter_with_color(
                "DEBUG".into(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        manager.save_filters(&filter_path).unwrap();
    }

    let (_db, mut manager) = setup().await;
    manager.load_filters(&filter_path).await.unwrap();

    let reader = FileReader::from_bytes(b"INFO line\nERROR line\nDEBUG line\n".to_vec());
    let mut out = Vec::new();
    run_headless_to_writer(reader, &manager, &mut out).unwrap();
    let result = String::from_utf8(out).unwrap();
    assert_eq!(result, "ERROR line\n");
}

#[test]
fn test_filter_regex_spaced_pattern_parses() {
    let args = CommandLine::parse_from(shell_split("filter -r \\d{3} \\d+"));
    match args.command.unwrap() {
        Commands::Filter { pattern, regex, .. } => {
            assert!(regex);
            assert_eq!(pattern.join(" "), r"\d{3} \d+");
        }
        other => panic!("expected Filter, got {:?}", other),
    }
}

#[test]
fn test_exclude_regex_spaced_pattern_parses() {
    let args = CommandLine::parse_from(shell_split("exclude -r \\d{3} \\d+"));
    match args.command.unwrap() {
        Commands::Exclude { pattern, regex, .. } => {
            assert!(regex);
            assert_eq!(pattern.join(" "), r"\d{3} \d+");
        }
        other => panic!("expected Exclude, got {:?}", other),
    }
}

#[test]
fn test_filter_ignore_case_flag_parses() {
    let args = CommandLine::parse_from(shell_split("filter --ignore-case ERROR"));
    match args.command.unwrap() {
        Commands::Filter { ignore_case, .. } => {
            assert!(ignore_case);
        }
        other => panic!("expected Filter, got {:?}", other),
    }
}

#[test]
fn test_filter_short_ignore_case_flag_parses() {
    let args = CommandLine::parse_from(shell_split("filter -i ERROR"));
    match args.command.unwrap() {
        Commands::Filter { ignore_case, .. } => {
            assert!(ignore_case);
        }
        other => panic!("expected Filter, got {:?}", other),
    }
}

#[tokio::test]
async fn test_regex_filter_with_space_in_pattern() {
    let (_db, mut manager) = setup().await;
    manager
        .add_filter_with_color(
            r"\d{3} \d+".into(),
            FilterType::Include,
            FilterOptions::default().regex(),
        )
        .await;

    let reader = FileReader::from_bytes(
        b"error 404 123 not found\nconnection established\nstatus 200 1 ok\n".to_vec(),
    );
    let mut out = Vec::new();
    run_headless_to_writer(reader, &manager, &mut out).unwrap();
    let result = String::from_utf8(out).unwrap();
    assert!(result.contains("404 123"));
    assert!(result.contains("200 1"));
    assert!(!result.contains("connection established"));
}

#[test]
fn test_sidebar_position_parses_left() {
    let args = CommandLine::parse_from(shell_split("sidebar-position left"));
    match args.command.unwrap() {
        Commands::SidebarPosition { side } => assert_eq!(side, SidebarSide::Left),
        other => panic!("expected SidebarPosition, got {:?}", other),
    }
}

#[test]
fn test_sidebar_position_parses_right() {
    let args = CommandLine::parse_from(shell_split("sidebar-position right"));
    match args.command.unwrap() {
        Commands::SidebarPosition { side } => assert_eq!(side, SidebarSide::Right),
        other => panic!("expected SidebarPosition, got {:?}", other),
    }
}

#[test]
fn test_sidebar_position_rejects_invalid() {
    assert!(CommandLine::try_parse_from(shell_split("sidebar-position top")).is_err());
}

fn build_dlt_storage_header(secs: u32, usecs: u32, ecu: &[u8; 4]) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(b"DLT\x01");
    h.extend_from_slice(&secs.to_le_bytes());
    h.extend_from_slice(&usecs.to_le_bytes());
    h.extend_from_slice(ecu);
    h
}

fn build_dlt_std_header(htyp: u8, mcnt: u8, length: u16) -> Vec<u8> {
    let mut h = Vec::new();
    h.push(htyp);
    h.push(mcnt);
    h.extend_from_slice(&length.to_be_bytes());
    h
}

fn build_dlt_ext_header(msin: u8, noar: u8, apid: &[u8; 4], ctid: &[u8; 4]) -> Vec<u8> {
    let mut h = Vec::new();
    h.push(msin);
    h.push(noar);
    h.extend_from_slice(apid);
    h.extend_from_slice(ctid);
    h
}

fn make_dlt_binary_data(count: usize) -> Vec<u8> {
    let mut data = Vec::new();
    for i in 0..count {
        data.extend_from_slice(&build_dlt_storage_header(1705312245 + i as u32, 0, b"ECU1"));
        let htyp = 0x01; // UEH
        let msin = 0x01 | (4 << 4); // verbose, log, info
        let ext = build_dlt_ext_header(msin, 0, b"APP1", b"CTX1");
        let msg_len = (4 + ext.len()) as u16;
        let mut msg = build_dlt_std_header(htyp, i as u8, msg_len);
        msg.extend_from_slice(&ext);
        data.extend_from_slice(&msg);
    }
    data
}

#[test]
fn test_dlt_binary_roundtrip() {
    use logana::parser::dlt::DltParser;
    use logana::parser::types::LogFormatParser;

    let dlt_data = make_dlt_binary_data(3);
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&dlt_data).unwrap();
    f.flush().unwrap();

    let reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
    assert!(reader.is_binary);
    assert_eq!(reader.line_count(), 3);

    let parser = DltParser;
    for i in 0..reader.line_count() {
        let line = reader.get_line(i);
        let parts = parser.parse_line(line);
        assert!(parts.is_some(), "Line {} should be parseable", i);
        let parts = parts.unwrap();
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("APP1"));
    }
}

#[test]
fn test_detect_format_selects_dlt_for_dlt_text() {
    use logana::parser::detect_format;

    let lines: Vec<&[u8]> = vec![
        b"2024/01/15 09:50:45.000000 0 000 ECU1 APP1 CTX1 log info verbose 0 msg1",
        b"2024/01/15 09:50:46.000000 0 000 ECU1 APP1 CTX1 log warn verbose 0 msg2",
    ];
    let parser = detect_format(&lines).unwrap();
    assert_eq!(parser.name(), "dlt");
}

#[test]
fn test_detect_format_does_not_select_dlt_for_non_dlt() {
    use logana::parser::detect_format;

    let lines: Vec<&[u8]> = vec![
        br#"{"level":"INFO","msg":"hello"}"#,
        br#"{"level":"WARN","msg":"world"}"#,
    ];
    let parser = detect_format(&lines).unwrap();
    assert_ne!(parser.name(), "dlt");
}

// ---------------------------------------------------------------------------
// Multiline / continuation-line tests
// ---------------------------------------------------------------------------

/// Build a FileReader from a multiline log string with Java-style stack traces.
fn make_multiline_log() -> FileReader {
    // Line 0: ERROR entry with timestamp (parent)
    // Line 1: stack frame (continuation)
    // Line 2: stack frame (continuation)
    // Line 3: INFO entry with timestamp (standalone parent)
    let data = b"\
2024-01-15 10:00:00 ERROR com.example.App - NullPointerException\n\
    at com.example.Foo.bar(Foo.java:42)\n\
    at com.example.Main.main(Main.java:10)\n\
2024-01-16 11:00:00 INFO  com.example.App - Application started\n";
    FileReader::from_bytes(data.to_vec())
}

#[test]
fn test_build_continuation_map_basic() {
    use logana::parser::detect_format;
    use logana::ui::build_continuation_map;

    let reader = make_multiline_log();
    let sample: Vec<&[u8]> = (0..reader.line_count())
        .map(|i| reader.get_line(i))
        .collect();
    let parser = detect_format(&sample).expect("format should be detected");

    let cmap = build_continuation_map(&reader, parser.as_ref());

    assert_eq!(cmap.len(), 4);
    assert_eq!(cmap[0], 0, "line 0 is its own parent");
    assert_eq!(cmap[1], 0, "line 1 is a continuation of line 0");
    assert_eq!(cmap[2], 0, "line 2 is a continuation of line 0");
    assert_eq!(cmap[3], 3, "line 3 is its own parent");
}

/// Regression test: a schema's `continuation.end_pattern` line must be
/// grouped as part of the block it terminates even when its text also
/// happens to satisfy the schema's own main header pattern — an easy
/// authoring overlap, since an end line's wording (e.g. "... ended at: ...")
/// often closely resembles its header's (e.g. "... started at: ..."). Before
/// this was fixed, `build_continuation_map` only asked `parse_line`, so an
/// end line that independently parsed became its own standalone parent
/// instead of collapsing into the preceding record.
#[test]
fn test_build_continuation_map_end_pattern_never_starts_a_new_block() {
    use logana::config::{CustomSchemaConfig, TemplateLine, TemplateValue};
    use logana::parser::CustomParser;
    use logana::ui::build_continuation_map;

    let cfg = CustomSchemaConfig {
        schema_url: None,
        name: "txn-overlap".to_string(),
        description: None,
        // Deliberately loose: matches both "started at" and "ended at" lines.
        template: Some(TemplateValue::Lines(vec![
            TemplateLine::Str("### Transaction {rest}".to_string()),
            TemplateLine::Str("### Transaction ended at: {ended_at}".to_string()),
        ])),
        pattern: None,
        fields: [("rest".to_string(), "extra".to_string())]
            .into_iter()
            .collect(),
        levels: Default::default(),
        multiline: false,
    };
    let parser = CustomParser::from_config(&cfg).unwrap();

    let data = b"### Transaction 42 started at: 10:00\n\
                 Status: PENDING\n\
                 ### Transaction ended at: 10:05\n\
                 ### Transaction 43 started at: 10:06\n\
                 Status: PENDING\n\
                 ### Transaction ended at: 10:11\n"
        .to_vec();
    let reader = FileReader::from_bytes(data);

    let cmap = build_continuation_map(&reader, &parser);

    assert_eq!(cmap, vec![0, 0, 0, 3, 3, 3]);
}

#[test]
fn test_apply_continuation_correction_hides_orphaned_continuations() {
    use logana::ui::{VisibleLines, apply_continuation_correction};

    // Simulate: filter kept only line 3 (INFO); lines 0, 1, 2 were filtered.
    // Without correction, continuation lines 1 and 2 might still be visible.
    // With correction they must be hidden because their parent (line 0) is hidden.
    let cmap = vec![0usize, 0, 0, 3]; // lines 1,2 → parent 0; line 3 → parent 3
    let mut visible = VisibleLines::Filtered(vec![1, 2, 3]); // lines 1 & 2 erroneously visible
    apply_continuation_correction(&mut visible, &cmap, false);
    // line 0 was NOT in visible, so its continuations (1, 2) must be removed.
    // line 3 is its own parent and was visible → stays.
    assert_eq!(visible, VisibleLines::Filtered(vec![3]));
}

#[test]
fn test_apply_continuation_correction_keeps_continuations_with_parent() {
    use logana::ui::{VisibleLines, apply_continuation_correction};

    // Simulate: filter kept line 0 (ERROR parent); continuations 1 and 2 also passed.
    let cmap = vec![0usize, 0, 0, 3];
    let mut visible = VisibleLines::Filtered(vec![0, 1, 2, 3]);
    apply_continuation_correction(&mut visible, &cmap, false);
    // Parent 0 is visible → continuations 1 and 2 stay visible.
    assert_eq!(visible, VisibleLines::Filtered(vec![0, 1, 2, 3]));
}

#[test]
fn test_apply_continuation_correction_noop_for_all_variant() {
    use logana::ui::{VisibleLines, apply_continuation_correction};

    let cmap = vec![0usize, 0, 1];
    let mut visible = VisibleLines::All(3);
    apply_continuation_correction(&mut visible, &cmap, false);
    // All-variant must remain unchanged.
    assert_eq!(visible, VisibleLines::All(3));
}

#[tokio::test]
async fn test_exclude_filter_hides_continuation_lines() {
    use logana::filters::FilterType;
    use logana::parser::detect_format;
    use logana::ui::{VisibleLines, apply_continuation_correction, build_continuation_map};

    let (_db, mut manager) = setup().await;
    let reader = make_multiline_log();
    let sample: Vec<&[u8]> = (0..reader.line_count())
        .map(|i| reader.get_line(i))
        .collect();
    let parser = detect_format(&sample).expect("format detected");
    let cmap = build_continuation_map(&reader, parser.as_ref());

    // Exclude lines containing "ERROR" — should hide line 0 AND its continuations
    manager
        .add_filter_with_color(
            "ERROR".into(),
            FilterType::Exclude,
            FilterOptions::default(),
        )
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();
    let mut visible = VisibleLines::Filtered(fm.compute_visible(&reader));
    apply_continuation_correction(&mut visible, &cmap, fm.has_include());

    // Line 0 (ERROR) excluded; lines 1 & 2 are its continuations → also excluded.
    // Line 3 (INFO) should be visible.
    assert!(!visible.contains(0), "ERROR entry should be hidden");
    assert!(
        !visible.contains(1),
        "continuation 1 should be hidden with parent"
    );
    assert!(
        !visible.contains(2),
        "continuation 2 should be hidden with parent"
    );
    assert!(visible.contains(3), "INFO entry should be visible");
}

#[tokio::test]
async fn test_include_filter_shows_continuations_with_parent() {
    use logana::filters::FilterType;
    use logana::parser::detect_format;
    use logana::ui::{VisibleLines, apply_continuation_correction, build_continuation_map};

    let (_db, mut manager) = setup().await;
    let reader = make_multiline_log();
    let sample: Vec<&[u8]> = (0..reader.line_count())
        .map(|i| reader.get_line(i))
        .collect();
    let parser = detect_format(&sample).expect("format detected");
    let cmap = build_continuation_map(&reader, parser.as_ref());

    // Include only lines containing "ERROR" — parent matches; continuations should follow.
    manager
        .add_filter_with_color(
            "ERROR".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();
    let mut visible = VisibleLines::Filtered(fm.compute_visible(&reader));
    apply_continuation_correction(&mut visible, &cmap, fm.has_include());

    // Line 0 matches; its continuations (1, 2) should be shown.
    // Line 3 (INFO) does not match include → hidden.
    assert!(visible.contains(0), "ERROR entry should be visible");
    assert!(
        visible.contains(1),
        "continuation 1 should follow its visible parent"
    );
    assert!(
        visible.contains(2),
        "continuation 2 should follow its visible parent"
    );
    assert!(
        !visible.contains(3),
        "INFO entry should be hidden (no match)"
    );
}

/// Regression test: an include filter that only matches text inside a
/// *continuation* line (never the header) must still pull the whole
/// multiline record into view, not just leave it hidden because the
/// header itself didn't match.
#[tokio::test]
async fn test_include_filter_matching_only_continuation_shows_whole_record() {
    use logana::filters::FilterType;
    use logana::parser::detect_format;
    use logana::ui::{VisibleLines, apply_continuation_correction, build_continuation_map};

    let (_db, mut manager) = setup().await;
    let reader = make_multiline_log();
    let sample: Vec<&[u8]> = (0..reader.line_count())
        .map(|i| reader.get_line(i))
        .collect();
    let parser = detect_format(&sample).expect("format detected");
    let cmap = build_continuation_map(&reader, parser.as_ref());

    // "Foo.bar" appears only in continuation line 1, not in header line 0
    // ("...NullPointerException") nor continuation line 2 ("...Main.main...").
    manager
        .add_filter_with_color(
            "Foo.bar".into(),
            FilterType::Include,
            FilterOptions::default(),
        )
        .await;
    let (fm, _, _, _) = manager.build_filter_manager();
    let mut visible = VisibleLines::Filtered(fm.compute_visible(&reader));
    apply_continuation_correction(&mut visible, &cmap, fm.has_include());

    assert!(
        visible.contains(0),
        "header should be shown because a line in its record matched"
    );
    assert!(visible.contains(1), "the matching continuation line itself");
    assert!(
        visible.contains(2),
        "sibling continuation should follow the promoted record"
    );
    assert!(
        !visible.contains(3),
        "unrelated INFO entry should stay hidden (no match)"
    );
}

#[test]
fn test_multiline_schema_transaction_end_to_end() {
    use logana::config::{ContinuationFieldSpec, CustomSchemaConfig, TemplateLine, TemplateValue};
    use logana::parser::{CustomParser, LogFormatParser};
    use logana::ui::build_continuation_map;

    // The scenario from the multiline custom-schema feature request: a
    // header capturing a transaction id, continuation lines carrying their
    // own fields (including one embedded JSON object), terminated by an
    // explicit end marker.
    let cfg = CustomSchemaConfig {
        name: "transaction".to_string(),
        description: None,
        template: Some(TemplateValue::Lines(vec![
            TemplateLine::Str("### Start transaction {id}".to_string()),
            TemplateLine::Plain(ContinuationFieldSpec {
                template: Some("field1: {field1}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            }),
            TemplateLine::Plain(ContinuationFieldSpec {
                template: Some("field2: {field2}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            }),
            TemplateLine::Plain(ContinuationFieldSpec {
                template: Some("Object {payload}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: true,
            }),
            TemplateLine::Str("### End transaction".to_string()),
        ])),
        pattern: None,
        fields: [("id".to_string(), "extra".to_string())]
            .into_iter()
            .collect(),
        levels: Default::default(),
        multiline: false,
        ..Default::default()
    };
    let parser = CustomParser::from_config(&cfg).expect("schema should compile");

    let data = b"### Start transaction 42\nfield1: 10\nfield2: 3\nObject {\"user\":\"alice\",\"amount\":99}\n### End transaction\n### Start transaction 43\nfield1: 20\n### End transaction\n".to_vec();
    let reader = FileReader::from_bytes(data);
    let cmap = build_continuation_map(&reader, &parser);

    assert_eq!(cmap, vec![0, 0, 0, 0, 0, 5, 5, 5]);

    // Walk record 1's continuation block the same way the render pipeline
    // does (see `apply_continuation_fields` in `ui::tab_state`), stopping at
    // the end_pattern line.
    let extract_record = |header_idx: usize| -> Vec<(String, String)> {
        let mut fields = Vec::new();
        let mut j = header_idx + 1;
        while j < cmap.len() && cmap[j] == header_idx {
            let line = reader.get_line(j);
            if parser.is_continuation_end(line) {
                break;
            }
            for (_, k, v) in parser.extract_continuation_fields(line) {
                fields.push((k.to_string(), v.to_string()));
            }
            j += 1;
        }
        fields
    };

    let record1 = extract_record(0);
    assert!(record1.contains(&("field1".to_string(), "10".to_string())));
    assert!(record1.contains(&("field2".to_string(), "3".to_string())));
    assert!(record1.contains(&("user".to_string(), "alice".to_string())));
    assert!(record1.contains(&("amount".to_string(), "99".to_string())));

    let header1 = parser.parse_line(reader.get_line(0)).unwrap();
    let id1 = header1
        .extra_fields
        .iter()
        .find(|(_, k, _)| *k == "id")
        .map(|(_, _, v)| *v);
    assert_eq!(id1, Some("42"));

    // The second transaction parses independently: its own id and field1,
    // with nothing leaked from the first record.
    let header2 = parser.parse_line(reader.get_line(5)).unwrap();
    let id2 = header2
        .extra_fields
        .iter()
        .find(|(_, k, _)| *k == "id")
        .map(|(_, _, v)| *v);
    assert_eq!(id2, Some("43"));

    let record2 = extract_record(5);
    assert_eq!(record2, vec![("field1".to_string(), "20".to_string())]);
}

/// The exact worked example from the "repeated field groups" / unified
/// `template` array feature request: a transaction with one multi-line
/// `operations` item, two single-line `votes` items and one single-line
/// `abort_stage` item, using the schema shipped as
/// `examples/custom-schema-transaction-groups.example.json`.
const TRANSACTION_GROUPS_LOG: &str = r#"### Transaction 148 started at: 1980-01-06 00:00:22.234
Status: FAILED
User: syscom
Operations:
operation_type: CREATE
object {
  object_name: "txCarrier1"
  class_name: "ECpriTxArrayCarrierConf"
  [hlapi.ho.ECpriTxArrayCarrierConf.object] {
    array: "txArray4a"
    frequency_Hz: 634500000
    bandwidth_Hz: 5000000
    active: INACTIVE
    type: NR_5G_FDD
    sfn_offsets {
      alpha: 0
      beta: 0
    }
    default_subcarrier_spacing: KHZ_15
    cyclic_prefix: NORMAL_CP
    reference_level: 0
    data_mode: TESTRAM
    gain: 47
  }
}
Voting Round 0:
ReqId: 148001, Actor: l1low::@10120007, VotingStatus: YES
ReqId: 148002, Actor: draco::instance1, VotingStatus: FAILED
Abort stage:
ReqId: 148003, Actor: l1low::@10120007, AbortStatus: OK
### Transaction ended at: 1980-01-06 00:00:22.297
"#;

fn transaction_groups_parser() -> logana::parser::CustomParser {
    let cfg: logana::config::CustomSchemaConfig = serde_json::from_str(include_str!(
        "../examples/custom-schema-transaction-groups.example.json"
    ))
    .unwrap();
    logana::parser::CustomParser::from_config(&cfg).expect("schema should compile")
}

/// Parses the header at line 0 and merges its continuation block, the same
/// way `ui::tab_state::apply_continuation_fields` does — hand-rolled here
/// (rather than calling that private function) using only the public
/// `LogFormatParser` trait methods it's built from.
fn parse_transaction_groups_header<'a>(
    parser: &logana::parser::CustomParser,
    reader: &'a FileReader,
    cmap: &[usize],
) -> logana::parser::DisplayParts<'a> {
    use logana::parser::LogFormatParser;

    let mut parts = parser
        .parse_line(reader.get_line(0))
        .expect("header line should parse");
    let mut lines: Vec<&'a [u8]> = Vec::new();
    let mut j = 1;
    while j < cmap.len() && cmap[j] == 0 {
        let line = reader.get_line(j);
        if parser.is_continuation_end(line) {
            break;
        }
        lines.push(line);
        j += 1;
    }
    let result = parser.walk_continuation(&lines);
    parts.extra_fields.extend(result.flat_fields);
    parts.field_groups = result.groups;
    parts
}

#[test]
fn test_transaction_groups_schema_end_to_end() {
    use logana::ui::build_continuation_map;

    let parser = transaction_groups_parser();
    let reader = FileReader::from_bytes(TRANSACTION_GROUPS_LOG.as_bytes().to_vec());
    let cmap = build_continuation_map(&reader, &parser);

    let parts = parse_transaction_groups_header(&parser, &reader, &cmap);

    let group = |name: &str| {
        parts
            .field_groups
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("group '{name}' not found in {:?}", parts.field_groups))
    };
    fn field<'a>(
        fields: &[(logana::parser::FieldSemantic, &'a str, &'a str)],
        key: &str,
    ) -> Option<&'a str> {
        fields
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(_, _, v)| *v)
    }

    // "operations" has exactly one multi-line item. Every `key: value` line
    // inside it is captured by `auto_fields` — regardless of brace nesting
    // depth (object_name/class_name at depth 1, frequency_Hz/active/type at
    // depth 2, alpha/beta at depth 3 under sfn_offsets) or of never having
    // been declared in the schema — since the schema has no `fields[]` list
    // at all for this group, only the default `auto_fields: true`.
    let (_, operations) = group("operations");
    assert_eq!(operations.len(), 1, "exactly one operation in this record");
    let op = &operations[0].fields;
    assert_eq!(field(op, "operation_type"), Some("CREATE"));
    assert_eq!(field(op, "object_name"), Some("txCarrier1"));
    assert_eq!(field(op, "class_name"), Some("ECpriTxArrayCarrierConf"));
    assert_eq!(field(op, "array"), Some("txArray4a"));
    assert_eq!(field(op, "frequency_Hz"), Some("634500000"));
    assert_eq!(field(op, "bandwidth_Hz"), Some("5000000"));
    assert_eq!(field(op, "active"), Some("INACTIVE"));
    assert_eq!(field(op, "type"), Some("NR_5G_FDD"));
    assert_eq!(field(op, "alpha"), Some("0"));
    assert_eq!(field(op, "beta"), Some("0"));
    assert_eq!(field(op, "default_subcarrier_spacing"), Some("KHZ_15"));
    assert_eq!(field(op, "cyclic_prefix"), Some("NORMAL_CP"));
    assert_eq!(field(op, "reference_level"), Some("0"));
    assert_eq!(field(op, "data_mode"), Some("TESTRAM"));
    assert_eq!(field(op, "gain"), Some("47"));
    // Brace-only and bracketed-extension-name lines ("object {",
    // "[hlapi.ho.ECpriTxArrayCarrierConf.object] {", "sfn_offsets {", "}")
    // have no `key: value` shape and contribute nothing.
    assert_eq!(
        op.len(),
        15,
        "exactly the 15 key:value lines, nothing from brace/bracket lines: {op:?}"
    );

    // "votes" has two single-line items — each `ReqId:...VotingStatus:...`
    // line is both its own start and its own complete item.
    let (_, votes) = group("votes");
    assert_eq!(votes.len(), 2);
    assert_eq!(field(&votes[0].fields, "req_id"), Some("148001"));
    assert_eq!(field(&votes[0].fields, "voting_status"), Some("YES"));
    assert_eq!(field(&votes[1].fields, "req_id"), Some("148002"));
    assert_eq!(field(&votes[1].fields, "voting_status"), Some("FAILED"));

    // "abort_stage" has one single-line item.
    let (_, abort_stage) = group("abort_stage");
    assert_eq!(abort_stage.len(), 1);
    assert_eq!(field(&abort_stage[0].fields, "req_id"), Some("148003"));
    assert_eq!(field(&abort_stage[0].fields, "abort_status"), Some("OK"));

    // Flat top-level fields (Status/User) and section-label lines
    // ("Operations:", "Voting Round 0:", "Abort stage:") are unaffected —
    // the labels contribute nothing anywhere.
    assert_eq!(field(&parts.extra_fields, "status"), Some("FAILED"));
    assert_eq!(field(&parts.extra_fields, "user"), Some("syscom"));
    assert!(
        !parts
            .extra_fields
            .iter()
            .any(|(_, k, _)| k.contains("Round"))
    );

    // Rendered columns show the flattened operations.0.*/votes.0.*/votes.1.*/
    // abort_stage.0.* shape via the same pipeline the log panel uses.
    let layout = logana::ui::FieldLayout::default();
    let hidden = std::collections::HashSet::new();
    let cols = logana::ui::field_layout::apply_field_layout(&parts, &layout, &hidden, true, None);
    assert!(
        cols.contains(&"operations.0.object_name=txCarrier1".to_string()),
        "{cols:?}"
    );
    assert!(
        cols.contains(&"votes.0.req_id=148001".to_string()),
        "{cols:?}"
    );
    assert!(
        cols.contains(&"votes.1.req_id=148002".to_string()),
        "{cols:?}"
    );
    assert!(
        cols.contains(&"abort_stage.0.req_id=148003".to_string()),
        "{cols:?}"
    );
}

#[test]
fn test_transaction_groups_field_filter_any_item_semantics_end_to_end() {
    use logana::filters::{FilterDef, FilterType, extract_field_filters, field_filter_matches};
    use logana::ui::build_continuation_map;

    let parser = transaction_groups_parser();
    let reader = FileReader::from_bytes(TRANSACTION_GROUPS_LOG.as_bytes().to_vec());
    let cmap = build_continuation_map(&reader, &parser);
    let parts = parse_transaction_groups_header(&parser, &reader, &cmap);

    let filter_for = |field: &str, pattern: &str| -> logana::filters::FieldFilter {
        let def = FilterDef {
            id: 1,
            pattern: logana::filters::encode_field_filter(
                &[(field.to_string(), pattern.to_string())],
                None,
            ),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: None,
        };
        let (includes, _) = extract_field_filters(std::slice::from_ref(&def));
        includes.into_iter().next().unwrap()
    };

    let matches_operation = filter_for("operations.object_name", "txCarrier1");
    assert!(field_filter_matches(
        &matches_operation,
        &parts,
        reader.get_line(0)
    ));

    // Matches only the *second* votes item — proves "any item", not "first".
    let matches_second_vote = filter_for("votes.voting_status", "FAILED");
    assert!(field_filter_matches(
        &matches_second_vote,
        &parts,
        reader.get_line(0)
    ));

    let no_match = filter_for("operations.object_name", "no-such-carrier");
    assert!(!field_filter_matches(&no_match, &parts, reader.get_line(0)));
}

/// `main.rs`'s `--headless` CLI path is glue code the library-level
/// `run_headless_to_writer` tests above never exercise (they build a
/// `LogManager`/`FileReader` directly, bypassing `main.rs` entirely). This
/// spawns the actual compiled binary to catch bugs that only live in that
/// glue — e.g. `--headless` used to never call `init_schemas()`, so custom
/// schemas (and any `--field` filter on a field they define) silently had no
/// effect in headless mode even though the same schema worked in the TUI.
#[test]
fn test_headless_binary_loads_custom_schema_and_honors_field_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let schema_dir = tmp.path().join("logana").join("schema");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::write(
        schema_dir.join("acme.json"),
        r#"{"name": "acme", "template": "{level} {message}"}"#,
    )
    .unwrap();

    let log_path = tmp.path().join("sample.log");
    std::fs::write(&log_path, "ERROR boom\nINFO fine\n").unwrap();

    // `-i`'s value is one shell-split filter spec (see `logana --help`), not
    // separate argv entries — the space inside is intentional.
    #[allow(clippy::suspicious_command_arg_space)]
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_logana"))
        .arg("--headless")
        .arg(&log_path)
        .arg("-i")
        .arg("--field level=ERROR")
        .env("XDG_CONFIG_HOME", tmp.path())
        .output()
        .expect("failed to run the logana binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ERROR boom"),
        "matching line missing from stdout: {stdout:?} (stderr: {})",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !stdout.contains("INFO fine"),
        "non-matching line should have been filtered out: {stdout:?}"
    );
}
