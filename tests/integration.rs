use logsmith_rs::analyzer::{FilterType, LogAnalyzer, LogLevel};
use logsmith_rs::db::Database;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;

fn setup() -> (Arc<tokio::runtime::Runtime>, Arc<Database>, LogAnalyzer) {
    let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
    let db = rt.block_on(Database::in_memory()).unwrap();
    let db = Arc::new(db);
    let analyzer = LogAnalyzer::new(db.clone(), rt.clone());
    (rt, db, analyzer)
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
fn test_full_ingestion_and_query_flow() {
    let (_rt, _db, analyzer) = setup();
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();

    analyzer.ingest_file(path).unwrap();

    let logs = analyzer.get_logs();
    assert_eq!(logs.len(), 7);

    // Verify parsed fields
    assert_eq!(logs[0].timestamp, Some("Jun 28 10:00:01".to_string()));
    assert_eq!(logs[0].hostname, Some("myhost".to_string()));
    assert_eq!(logs[0].process_name, Some("myapp".to_string()));
    assert_eq!(logs[0].pid, Some(1234));
    assert_eq!(logs[0].level, LogLevel::Info);
    assert_eq!(logs[0].message, "Application started");

    // Check error level
    assert_eq!(logs[1].level, LogLevel::Error);

    // Check kernel log (no PID)
    assert_eq!(logs[2].process_name, Some("kernel".to_string()));
    assert_eq!(logs[2].pid, None);

    // Check plain text line
    assert_eq!(logs[6].message, "plain text log line with no format");
    assert_eq!(logs[6].level, LogLevel::Unknown);
}

#[test]
fn test_filter_include_exclude_flow() {
    let (_rt, _db, analyzer) = setup();
    let file = create_sample_log_file();
    analyzer.ingest_file(file.path().to_str().unwrap()).unwrap();

    // Include only logs with "Connection" in the message
    analyzer.add_filter("Connection".to_string(), FilterType::Include);
    let logs = analyzer.get_logs();
    let filtered = analyzer.apply_filters(&logs).unwrap();
    assert_eq!(filtered.len(), 2);
    assert!(
        filtered
            .iter()
            .any(|l| l.message.contains("Connection failed"))
    );
    assert!(
        filtered
            .iter()
            .any(|l| l.message.contains("Connection established"))
    );

    // Clear and test exclude
    analyzer.clear_filters();
    analyzer.add_filter("Linux".to_string(), FilterType::Exclude);
    let logs = analyzer.get_logs();
    let filtered = analyzer.apply_filters(&logs).unwrap();
    assert_eq!(filtered.len(), 6);
    assert!(!filtered.iter().any(|l| l.message.contains("Linux")));
}

#[test]
fn test_mark_and_export_flow() {
    let (_rt, _db, analyzer) = setup();
    let file = create_sample_log_file();
    analyzer.ingest_file(file.path().to_str().unwrap()).unwrap();

    // Mark some entries
    analyzer.toggle_mark(0);
    analyzer.toggle_mark(2);

    let marked = analyzer.get_marked_logs();
    assert_eq!(marked.len(), 2);
    assert_eq!(marked[0].id, 0);
    assert_eq!(marked[1].id, 2);

    // Unmark one
    analyzer.toggle_mark(0);
    let marked = analyzer.get_marked_logs();
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].id, 2);
}

#[test]
fn test_search_flow() {
    let (_rt, _db, analyzer) = setup();
    let file = create_sample_log_file();
    analyzer.ingest_file(file.path().to_str().unwrap()).unwrap();

    let results = analyzer.search("Connection").unwrap();
    assert_eq!(results.len(), 2); // "Connection failed" and "Connection established"

    // Case-insensitive search for "started"
    let results = analyzer.search("(?i)started").unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_filter_json_roundtrip_with_db() {
    let (_rt, _db, analyzer) = setup();

    // Add filters with different types
    analyzer.add_filter("error".to_string(), FilterType::Include);
    analyzer.add_filter("debug".to_string(), FilterType::Exclude);

    // Save to JSON
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    analyzer.save_filters(path).unwrap();

    // Create new analyzer and load from JSON
    let (_, _, analyzer2) = setup();
    analyzer2.load_filters(path).unwrap();

    let filters = analyzer2.get_filters();
    assert_eq!(filters.len(), 2);
    assert_eq!(filters[0].pattern, "error");
    assert_eq!(filters[0].filter_type, FilterType::Include);
    assert_eq!(filters[1].pattern, "debug");
    assert_eq!(filters[1].filter_type, FilterType::Exclude);
}

#[test]
fn test_persistence_check() {
    let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_path_str = db_path.to_str().unwrap();

    // First session: ingest data
    {
        let db = rt.block_on(Database::new(db_path_str)).unwrap();
        let db = Arc::new(db);
        let analyzer = LogAnalyzer::new(db.clone(), rt.clone());

        let file = create_sample_log_file();
        analyzer.ingest_file(file.path().to_str().unwrap()).unwrap();

        analyzer.add_filter("error".to_string(), FilterType::Include);

        let logs = analyzer.get_logs();
        assert_eq!(logs.len(), 7);
    }

    // Second session: data persists
    {
        let db = rt.block_on(Database::new(db_path_str)).unwrap();
        let db = Arc::new(db);
        let analyzer = LogAnalyzer::new(db.clone(), rt.clone());

        let logs = analyzer.get_logs();
        assert_eq!(logs.len(), 7);

        let filters = analyzer.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].pattern, "error");
    }
}

#[test]
fn test_has_logs_for_source_prevents_duplicate_ingestion() {
    let (_rt, _db, analyzer) = setup();
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();

    assert!(!analyzer.has_logs_for_source(path));
    analyzer.ingest_file(path).unwrap();
    assert!(analyzer.has_logs_for_source(path));

    let logs = analyzer.get_logs();
    assert_eq!(logs.len(), 7);
}

#[test]
fn test_reader_ingestion() {
    let (_rt, _db, analyzer) = setup();

    let input = "line 1\nline 2\nline 3\n";
    let cursor = std::io::Cursor::new(input.as_bytes());
    analyzer.ingest_reader(cursor).unwrap();

    let logs = analyzer.get_logs();
    assert_eq!(logs.len(), 3);
    assert_eq!(logs[0].message, "line 1");
    assert_eq!(logs[2].message, "line 3");
}

#[test]
fn test_filter_reorder() {
    let (_rt, _db, analyzer) = setup();

    analyzer.add_filter("first".to_string(), FilterType::Include);
    analyzer.add_filter("second".to_string(), FilterType::Include);
    analyzer.add_filter("third".to_string(), FilterType::Include);

    let filters = analyzer.get_filters();
    assert_eq!(filters[0].pattern, "first");
    assert_eq!(filters[1].pattern, "second");
    assert_eq!(filters[2].pattern, "third");

    // Move second up
    let id = filters[1].id;
    analyzer.move_filter_up(id);

    let filters = analyzer.get_filters();
    assert_eq!(filters[0].pattern, "second");
    assert_eq!(filters[1].pattern, "first");
    assert_eq!(filters[2].pattern, "third");
}

#[test]
fn test_combined_filters_and_search() {
    let (_rt, _db, analyzer) = setup();

    let input = "ERROR: database connection timeout\nINFO: user login successful\nERROR: authentication failed\nDEBUG: cache miss for key xyz\n";
    let cursor = std::io::Cursor::new(input.as_bytes());
    analyzer.ingest_reader(cursor).unwrap();

    // Add include filter for ERROR
    analyzer.add_filter("ERROR".to_string(), FilterType::Include);

    let logs = analyzer.get_logs();
    let filtered = analyzer.apply_filters(&logs).unwrap();
    assert_eq!(filtered.len(), 2);

    // Search within all logs
    let results = analyzer.search("connection").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].log_id, 0);
}
