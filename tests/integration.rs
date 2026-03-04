use logana::db::Database;
use logana::file_reader::FileReader;
use logana::filters::FilterManager;
use logana::log_manager::LogManager;
use logana::types::FilterType;
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
    let (fm, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);
    assert_eq!(visible.len(), 7);

    // Include only lines containing "Connection"
    manager
        .add_filter_with_color("Connection".into(), FilterType::Include, None, None, true)
        .await;
    let (fm, _) = manager.build_filter_manager();
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
        .add_filter_with_color("INFO".into(), FilterType::Exclude, None, None, true)
        .await;
    let (fm, _) = manager.build_filter_manager();
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
    // With newest-first ordering, "failed" Exclude ends up at index 0 (highest precedence).
    // First-match-wins: "Connection failed" matches the top Exclude and is hidden.
    manager
        .add_filter_with_color("Connection".into(), FilterType::Include, None, None, true)
        .await;
    manager
        .add_filter_with_color("failed".into(), FilterType::Exclude, None, None, true)
        .await;
    let (fm, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);

    // Line 1: "Connection failed" — top Exclude wins → hidden
    // Line 4: "Connection established" — no Exclude match → Include matches → visible
    assert_eq!(visible.len(), 1);
    assert!(visible.contains(&4));
}

#[tokio::test]
async fn test_disabled_filter_is_ignored() {
    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    manager
        .add_filter_with_color("INFO".into(), FilterType::Include, None, None, true)
        .await;
    let id = manager.get_filters()[0].id;
    manager.toggle_filter(id).await; // disable it

    // Disabled → no active include filters → all lines visible
    let (fm, _) = manager.build_filter_manager();
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

#[tokio::test]
async fn test_marks_persistence() {
    let (_db, mut manager) = setup().await;

    manager.toggle_mark(0);
    manager.toggle_mark(2);
    manager.toggle_mark(5);

    assert!(manager.is_marked(0));
    assert!(manager.is_marked(2));
    assert!(manager.is_marked(5));
    assert!(!manager.is_marked(1));
    assert!(!manager.is_marked(3));

    let indices = manager.get_marked_indices();
    assert_eq!(indices, vec![0, 2, 5]);

    // Toggle off
    manager.toggle_mark(2);
    assert!(!manager.is_marked(2));
    assert_eq!(manager.get_marked_indices(), vec![0, 5]);
}

#[tokio::test]
async fn test_get_marked_lines() {
    let (_db, mut manager) = setup().await;

    let data = b"alpha\nbeta\ngamma\n";
    let reader = FileReader::from_bytes(data.to_vec());

    manager.toggle_mark(0);
    manager.toggle_mark(2);

    let lines = manager.get_marked_lines(&reader);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], b"alpha");
    assert_eq!(lines[1], b"gamma");
}

#[tokio::test]
async fn test_add_and_remove_filters() {
    let (_db, mut manager) = setup().await;

    manager
        .add_filter_with_color("error".into(), FilterType::Include, None, None, true)
        .await;
    manager
        .add_filter_with_color("debug".into(), FilterType::Exclude, None, None, true)
        .await;
    assert_eq!(manager.get_filters().len(), 2);

    // Newest first: "debug" is at index 0; removing it leaves "error"
    let id = manager.get_filters()[0].id;
    manager.remove_filter(id).await;
    assert_eq!(manager.get_filters().len(), 1);
    assert_eq!(manager.get_filters()[0].pattern, "error");
}

#[tokio::test]
async fn test_move_filter_up_down() {
    let (_db, mut manager) = setup().await;

    manager
        .add_filter_with_color("first".into(), FilterType::Include, None, None, true)
        .await;
    manager
        .add_filter_with_color("second".into(), FilterType::Include, None, None, true)
        .await;
    manager
        .add_filter_with_color("third".into(), FilterType::Include, None, None, true)
        .await;

    // After three inserts (newest first): ["third", "second", "first"]
    // "second" is at index 1; move_filter_up swaps [0] and [1]
    let id_second = manager.get_filters()[1].id;
    manager.move_filter_up(id_second).await;

    // Result: ["second", "third", "first"]
    let filters = manager.get_filters();
    assert_eq!(filters[0].pattern, "second");
    assert_eq!(filters[1].pattern, "third");
    assert_eq!(filters[2].pattern, "first");
}

#[tokio::test]
async fn test_filter_regex_pattern() {
    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    // Regex pattern matching either INFO or ERROR
    manager
        .add_filter_with_color("INFO|ERROR".into(), FilterType::Include, None, None, true)
        .await;
    let (fm, _) = manager.build_filter_manager();
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
        .add_filter_with_color("error".into(), FilterType::Include, None, None, true)
        .await;
    manager
        .add_filter_with_color("debug".into(), FilterType::Exclude, None, None, true)
        .await;
    assert_eq!(manager.get_filters().len(), 2);

    manager.clear_filters().await;
    assert!(manager.get_filters().is_empty());
}

#[tokio::test]
async fn test_search_on_visible_lines() {
    use logana::search::Search;

    let (_db, mut manager) = setup().await;
    let file = create_sample_log_file();
    let path = file.path().to_str().unwrap();
    let reader = FileReader::new(path).unwrap();

    // Include only INFO lines
    manager
        .add_filter_with_color("INFO".into(), FilterType::Include, None, None, true)
        .await;
    let (fm, _) = manager.build_filter_manager();
    let visible = fm.compute_visible(&reader);
    assert_eq!(visible.len(), 2);

    // Search for "Application" within visible lines only
    let mut search = Search::new();
    search
        .search("Application", &visible, |li| {
            Some(String::from_utf8_lossy(reader.get_line(li)).into_owned())
        })
        .unwrap();
    let results = search.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].line_idx, 0);
}
