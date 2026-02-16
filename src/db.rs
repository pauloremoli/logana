use anyhow::Result;
use async_trait::async_trait;
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

use crate::analyzer::{ColorConfig, Filter, FilterType, LogEntry, LogLevel};

#[async_trait]
pub trait LogStore: Send + Sync {
    async fn insert_logs_batch(&self, entries: &[LogEntry]) -> Result<()>;
    async fn get_all_logs(&self) -> Result<Vec<LogEntry>>;
    async fn get_log_count(&self) -> Result<i64>;
    async fn toggle_mark(&self, id: i64) -> Result<()>;
    async fn get_marked_logs(&self) -> Result<Vec<LogEntry>>;
    async fn has_logs_for_source(&self, source: &str) -> Result<bool>;
    async fn clear_logs(&self) -> Result<()>;
}

#[async_trait]
pub trait FilterStore: Send + Sync {
    async fn insert_filter(
        &self,
        pattern: &str,
        filter_type: &FilterType,
        enabled: bool,
        color_config: Option<&ColorConfig>,
        source_file: Option<&str>,
    ) -> Result<i64>;
    async fn get_filters(&self) -> Result<Vec<Filter>>;
    async fn get_filters_for_source(&self, source_file: &str) -> Result<Vec<Filter>>;
    async fn update_filter_pattern(&self, id: i64, new_pattern: &str) -> Result<()>;
    async fn update_filter_color(&self, id: i64, color_config: Option<&ColorConfig>) -> Result<()>;
    async fn delete_filter(&self, id: i64) -> Result<()>;
    async fn toggle_filter(&self, id: i64) -> Result<()>;
    async fn swap_filter_order(&self, id1: i64, id2: i64) -> Result<()>;
    async fn clear_filters(&self) -> Result<()>;
    async fn replace_all_filters(
        &self,
        filters: &[Filter],
        source_file: Option<&str>,
    ) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileContext {
    pub source_file: String,
    pub scroll_offset: usize,
    pub search_query: String,
    pub wrap: bool,
    pub level_colors: bool,
    pub show_sidebar: bool,
    pub horizontal_scroll: usize,
    pub marked_lines: Vec<usize>,
    pub file_hash: Option<String>,
}

#[async_trait]
pub trait FileContextStore: Send + Sync {
    async fn save_file_context(&self, ctx: &FileContext) -> Result<()>;
    async fn load_file_context(&self, source_file: &str) -> Result<Option<FileContext>>;
}

pub struct Database {
    pool: SqlitePool,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish()
    }
}

impl Database {
    pub async fn new(path: &str) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let url = format!("sqlite:{}?mode=rwc", path);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await?;

        let db = Self { pool };
        db.configure_pragmas().await?;
        db.run_migrations().await?;
        Ok(db)
    }

    pub async fn in_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;

        let db = Self { pool };
        db.configure_pragmas().await?;
        db.run_migrations().await?;
        Ok(db)
    }

    async fn configure_pragmas(&self) -> Result<()> {
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA synchronous = NORMAL")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA cache_size = -64000")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn run_migrations(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS log_entries (
                id INTEGER PRIMARY KEY,
                timestamp TEXT,
                hostname TEXT,
                process_name TEXT,
                pid INTEGER,
                level TEXT NOT NULL DEFAULT 'Unknown',
                message TEXT NOT NULL,
                marked INTEGER NOT NULL DEFAULT 0,
                source_file TEXT
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS filters (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern TEXT NOT NULL,
                filter_type TEXT NOT NULL CHECK(filter_type IN ('Include', 'Exclude')),
                enabled INTEGER NOT NULL DEFAULT 1,
                fg_color TEXT,
                bg_color TEXT,
                display_order INTEGER NOT NULL DEFAULT 0,
                source_file TEXT NOT NULL DEFAULT '',
                match_only INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        // Migration: add source_file column if it doesn't exist (for existing databases)
        let _ = sqlx::query("ALTER TABLE filters ADD COLUMN source_file TEXT NOT NULL DEFAULT ''")
            .execute(&self.pool)
            .await;

        // Migration: add match_only column if it doesn't exist (for existing databases)
        let _ =
            sqlx::query("ALTER TABLE filters ADD COLUMN match_only INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS file_context (
                source_file TEXT PRIMARY KEY,
                scroll_offset INTEGER NOT NULL DEFAULT 0,
                search_query TEXT NOT NULL DEFAULT '',
                wrap INTEGER NOT NULL DEFAULT 1,
                level_colors INTEGER NOT NULL DEFAULT 1,
                show_sidebar INTEGER NOT NULL DEFAULT 1,
                horizontal_scroll INTEGER NOT NULL DEFAULT 0,
                marked_lines TEXT NOT NULL DEFAULT '[]',
                file_hash TEXT
            )",
        )
        .execute(&self.pool)
        .await?;

        // Migration: add marked_lines column if it doesn't exist (for existing databases)
        let _ = sqlx::query(
            "ALTER TABLE file_context ADD COLUMN marked_lines TEXT NOT NULL DEFAULT '[]'",
        )
        .execute(&self.pool)
        .await;

        // Migration: add file_hash column if it doesn't exist (for existing databases)
        let _ = sqlx::query("ALTER TABLE file_context ADD COLUMN file_hash TEXT")
            .execute(&self.pool)
            .await;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_log_level ON log_entries(level)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_log_process ON log_entries(process_name)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_log_source ON log_entries(source_file)")
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

fn log_level_to_str(level: &LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "Info",
        LogLevel::Warning => "Warning",
        LogLevel::Error => "Error",
        LogLevel::Debug => "Debug",
        LogLevel::Unknown => "Unknown",
    }
}

fn str_to_log_level(s: &str) -> LogLevel {
    match s {
        "Info" => LogLevel::Info,
        "Warning" => LogLevel::Warning,
        "Error" => LogLevel::Error,
        "Debug" => LogLevel::Debug,
        _ => LogLevel::Unknown,
    }
}

fn filter_type_to_str(ft: &FilterType) -> &'static str {
    match ft {
        FilterType::Include => "Include",
        FilterType::Exclude => "Exclude",
    }
}

fn str_to_filter_type(s: &str) -> FilterType {
    match s {
        "Include" => FilterType::Include,
        _ => FilterType::Exclude,
    }
}

fn row_to_log_entry(row: &sqlx::sqlite::SqliteRow) -> LogEntry {
    LogEntry {
        id: row.get::<i64, _>("id") as usize,
        timestamp: row.get("timestamp"),
        hostname: row.get("hostname"),
        process_name: row.get("process_name"),
        pid: row.get::<Option<i64>, _>("pid").map(|p| p as u32),
        level: str_to_log_level(row.get::<&str, _>("level")),
        message: row.get("message"),
        marked: row.get::<i32, _>("marked") != 0,
        source_file: row.get("source_file"),
    }
}

fn row_to_filter(row: &sqlx::sqlite::SqliteRow) -> Filter {
    let fg_str: Option<String> = row.get("fg_color");
    let bg_str: Option<String> = row.get("bg_color");
    let match_only = row.get::<i32, _>("match_only") != 0;

    let color_config = match (fg_str, bg_str) {
        (None, None) if !match_only => None,
        (fg, bg) => Some(ColorConfig {
            fg: fg.and_then(|s| s.parse().ok()),
            bg: bg.and_then(|s| s.parse().ok()),
            match_only,
        }),
    };

    Filter {
        id: row.get::<i64, _>("id") as usize,
        pattern: row.get("pattern"),
        filter_type: str_to_filter_type(row.get::<&str, _>("filter_type")),
        enabled: row.get::<i32, _>("enabled") != 0,
        color_config,
    }
}

#[async_trait]
impl LogStore for Database {
    async fn insert_logs_batch(&self, entries: &[LogEntry]) -> Result<()> {
        // 9 columns per row, SQLite limit is 999 bound params → max 111 rows per statement.
        // Use 50 for a safe margin and good performance.
        const ROWS_PER_STMT: usize = 80;
        // Group statements into larger transactions to reduce commit overhead.
        const TX_SIZE: usize = 10000;

        for tx_chunk in entries.chunks(TX_SIZE) {
            let mut tx = self.pool.begin().await?;

            for stmt_chunk in tx_chunk.chunks(ROWS_PER_STMT) {
                let placeholders: String = stmt_chunk
                    .iter()
                    .map(|_| "(?, ?, ?, ?, ?, ?, ?, ?, ?)")
                    .collect::<Vec<_>>()
                    .join(", ");

                let sql = format!(
                    "INSERT INTO log_entries (id, timestamp, hostname, process_name, pid, level, message, marked, source_file) VALUES {}",
                    placeholders
                );

                let mut query = sqlx::query(&sql);
                for entry in stmt_chunk {
                    query = query
                        .bind(entry.id as i64)
                        .bind(&entry.timestamp)
                        .bind(&entry.hostname)
                        .bind(&entry.process_name)
                        .bind(entry.pid.map(|p| p as i64))
                        .bind(log_level_to_str(&entry.level))
                        .bind(&entry.message)
                        .bind(entry.marked as i32)
                        .bind(&entry.source_file);
                }

                query.execute(&mut *tx).await?;
            }

            tx.commit().await?;
        }

        Ok(())
    }

    async fn get_all_logs(&self) -> Result<Vec<LogEntry>> {
        let rows = sqlx::query("SELECT * FROM log_entries ORDER BY id")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.iter().map(row_to_log_entry).collect())
    }

    async fn get_log_count(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM log_entries")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("count"))
    }

    async fn toggle_mark(&self, id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE log_entries SET marked = CASE WHEN marked = 0 THEN 1 ELSE 0 END WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_marked_logs(&self) -> Result<Vec<LogEntry>> {
        let rows = sqlx::query("SELECT * FROM log_entries WHERE marked = 1 ORDER BY id")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.iter().map(row_to_log_entry).collect())
    }

    async fn has_logs_for_source(&self, source: &str) -> Result<bool> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM log_entries WHERE source_file = ?")
            .bind(source)
            .fetch_one(&self.pool)
            .await?;
        let count: i64 = row.get("count");
        Ok(count > 0)
    }

    async fn clear_logs(&self) -> Result<()> {
        sqlx::query("DELETE FROM log_entries")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl FilterStore for Database {
    async fn insert_filter(
        &self,
        pattern: &str,
        filter_type: &FilterType,
        enabled: bool,
        color_config: Option<&ColorConfig>,
        source_file: Option<&str>,
    ) -> Result<i64> {
        let source = source_file.unwrap_or("");
        let max_order: Option<i64> = sqlx::query(
            "SELECT MAX(display_order) as max_order FROM filters WHERE source_file = ?",
        )
        .bind(source)
        .fetch_one(&self.pool)
        .await?
        .get("max_order");

        let next_order = max_order.unwrap_or(-1) + 1;

        let (fg, bg, match_only) = match color_config {
            Some(cc) => (
                cc.fg.map(|c| c.to_string()),
                cc.bg.map(|c| c.to_string()),
                cc.match_only,
            ),
            None => (None, None, false),
        };

        let result = sqlx::query(
            "INSERT INTO filters (pattern, filter_type, enabled, fg_color, bg_color, display_order, source_file, match_only)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(pattern)
        .bind(filter_type_to_str(filter_type))
        .bind(enabled as i32)
        .bind(&fg)
        .bind(&bg)
        .bind(next_order)
        .bind(source)
        .bind(match_only as i32)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    async fn get_filters(&self) -> Result<Vec<Filter>> {
        let rows = sqlx::query("SELECT * FROM filters ORDER BY display_order")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.iter().map(row_to_filter).collect())
    }

    async fn get_filters_for_source(&self, source_file: &str) -> Result<Vec<Filter>> {
        let rows =
            sqlx::query("SELECT * FROM filters WHERE source_file = ? ORDER BY display_order")
                .bind(source_file)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.iter().map(row_to_filter).collect())
    }

    async fn update_filter_pattern(&self, id: i64, new_pattern: &str) -> Result<()> {
        sqlx::query("UPDATE filters SET pattern = ? WHERE id = ?")
            .bind(new_pattern)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_filter_color(&self, id: i64, color_config: Option<&ColorConfig>) -> Result<()> {
        let (fg, bg, match_only) = match color_config {
            Some(cc) => (
                cc.fg.map(|c| c.to_string()),
                cc.bg.map(|c| c.to_string()),
                cc.match_only,
            ),
            None => (None, None, false),
        };

        sqlx::query("UPDATE filters SET fg_color = ?, bg_color = ?, match_only = ? WHERE id = ?")
            .bind(&fg)
            .bind(&bg)
            .bind(match_only as i32)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_filter(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM filters WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn toggle_filter(&self, id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE filters SET enabled = CASE WHEN enabled = 0 THEN 1 ELSE 0 END WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn swap_filter_order(&self, id1: i64, id2: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let row1 = sqlx::query("SELECT display_order FROM filters WHERE id = ?")
            .bind(id1)
            .fetch_optional(&mut *tx)
            .await?;

        let row2 = sqlx::query("SELECT display_order FROM filters WHERE id = ?")
            .bind(id2)
            .fetch_optional(&mut *tx)
            .await?;

        if let (Some(r1), Some(r2)) = (row1, row2) {
            let order1: i64 = r1.get("display_order");
            let order2: i64 = r2.get("display_order");

            sqlx::query("UPDATE filters SET display_order = ? WHERE id = ?")
                .bind(order2)
                .bind(id1)
                .execute(&mut *tx)
                .await?;

            sqlx::query("UPDATE filters SET display_order = ? WHERE id = ?")
                .bind(order1)
                .bind(id2)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn clear_filters(&self) -> Result<()> {
        sqlx::query("DELETE FROM filters")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn replace_all_filters(
        &self,
        filters: &[Filter],
        source_file: Option<&str>,
    ) -> Result<()> {
        let source = source_file.unwrap_or("");
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM filters WHERE source_file = ?")
            .bind(source)
            .execute(&mut *tx)
            .await?;

        for (order, filter) in filters.iter().enumerate() {
            let (fg, bg, match_only) = match &filter.color_config {
                Some(cc) => (
                    cc.fg.map(|c| c.to_string()),
                    cc.bg.map(|c| c.to_string()),
                    cc.match_only,
                ),
                None => (None, None, false),
            };

            sqlx::query(
                "INSERT INTO filters (pattern, filter_type, enabled, fg_color, bg_color, display_order, source_file, match_only)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&filter.pattern)
            .bind(filter_type_to_str(&filter.filter_type))
            .bind(filter.enabled as i32)
            .bind(&fg)
            .bind(&bg)
            .bind(order as i64)
            .bind(source)
            .bind(match_only as i32)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }
}

#[async_trait]
impl FileContextStore for Database {
    async fn save_file_context(&self, ctx: &FileContext) -> Result<()> {
        let marked_json =
            serde_json::to_string(&ctx.marked_lines).unwrap_or_else(|_| "[]".to_string());
        sqlx::query(
            "INSERT INTO file_context (source_file, scroll_offset, search_query, wrap, level_colors, show_sidebar, horizontal_scroll, marked_lines, file_hash)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(source_file) DO UPDATE SET
                scroll_offset = excluded.scroll_offset,
                search_query = excluded.search_query,
                wrap = excluded.wrap,
                level_colors = excluded.level_colors,
                show_sidebar = excluded.show_sidebar,
                horizontal_scroll = excluded.horizontal_scroll,
                marked_lines = excluded.marked_lines,
                file_hash = excluded.file_hash",
        )
        .bind(&ctx.source_file)
        .bind(ctx.scroll_offset as i64)
        .bind(&ctx.search_query)
        .bind(ctx.wrap as i32)
        .bind(ctx.level_colors as i32)
        .bind(ctx.show_sidebar as i32)
        .bind(ctx.horizontal_scroll as i64)
        .bind(&marked_json)
        .bind(&ctx.file_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_file_context(&self, source_file: &str) -> Result<Option<FileContext>> {
        let row = sqlx::query(
            "SELECT source_file, scroll_offset, search_query, wrap, level_colors, show_sidebar, horizontal_scroll, marked_lines, file_hash
             FROM file_context WHERE source_file = ?",
        )
        .bind(source_file)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let marked_json: String = r.get("marked_lines");
            let marked_lines: Vec<usize> = serde_json::from_str(&marked_json).unwrap_or_default();
            FileContext {
                source_file: r.get::<String, _>("source_file"),
                scroll_offset: r.get::<i64, _>("scroll_offset") as usize,
                search_query: r.get::<String, _>("search_query"),
                wrap: r.get::<i32, _>("wrap") != 0,
                level_colors: r.get::<i32, _>("level_colors") != 0,
                show_sidebar: r.get::<i32, _>("show_sidebar") != 0,
                horizontal_scroll: r.get::<i64, _>("horizontal_scroll") as usize,
                marked_lines,
                file_hash: r.get::<Option<String>, _>("file_hash"),
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> Database {
        Database::in_memory().await.unwrap()
    }

    fn sample_log_entries() -> Vec<LogEntry> {
        vec![
            LogEntry {
                id: 0,
                timestamp: Some("Jun 28 10:00:03".to_string()),
                hostname: Some("myhost".to_string()),
                process_name: Some("myapp".to_string()),
                pid: Some(1234),
                level: LogLevel::Info,
                message: "Application started".to_string(),
                marked: false,
                source_file: Some("test.log".to_string()),
            },
            LogEntry {
                id: 1,
                timestamp: Some("Jun 28 10:00:04".to_string()),
                hostname: Some("myhost".to_string()),
                process_name: Some("myapp".to_string()),
                pid: Some(1234),
                level: LogLevel::Error,
                message: "Something went wrong".to_string(),
                marked: false,
                source_file: Some("test.log".to_string()),
            },
            LogEntry {
                id: 2,
                timestamp: None,
                hostname: None,
                process_name: None,
                pid: None,
                level: LogLevel::Unknown,
                message: "Plain text log line".to_string(),
                marked: false,
                source_file: Some("test.log".to_string()),
            },
        ]
    }

    #[tokio::test]
    async fn test_schema_creation() {
        let db = setup_db().await;
        let count = db.get_log_count().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_insert_and_get_logs() {
        let db = setup_db().await;
        let entries = sample_log_entries();
        db.insert_logs_batch(&entries).await.unwrap();

        let logs = db.get_all_logs().await.unwrap();
        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].id, 0);
        assert_eq!(logs[0].message, "Application started");
        assert_eq!(logs[0].level, LogLevel::Info);
        assert_eq!(logs[0].hostname, Some("myhost".to_string()));
        assert_eq!(logs[0].pid, Some(1234));
        assert_eq!(logs[1].level, LogLevel::Error);
        assert_eq!(logs[2].timestamp, None);
        assert_eq!(logs[2].hostname, None);
    }

    #[tokio::test]
    async fn test_log_count() {
        let db = setup_db().await;
        let entries = sample_log_entries();
        db.insert_logs_batch(&entries).await.unwrap();

        let count = db.get_log_count().await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_toggle_mark() {
        let db = setup_db().await;
        let entries = sample_log_entries();
        db.insert_logs_batch(&entries).await.unwrap();

        // Toggle mark on
        db.toggle_mark(0).await.unwrap();
        let logs = db.get_all_logs().await.unwrap();
        assert!(logs[0].marked);
        assert!(!logs[1].marked);

        // Toggle mark off
        db.toggle_mark(0).await.unwrap();
        let logs = db.get_all_logs().await.unwrap();
        assert!(!logs[0].marked);
    }

    #[tokio::test]
    async fn test_get_marked_logs() {
        let db = setup_db().await;
        let entries = sample_log_entries();
        db.insert_logs_batch(&entries).await.unwrap();

        db.toggle_mark(0).await.unwrap();
        db.toggle_mark(2).await.unwrap();

        let marked = db.get_marked_logs().await.unwrap();
        assert_eq!(marked.len(), 2);
        assert_eq!(marked[0].id, 0);
        assert_eq!(marked[1].id, 2);
    }

    #[tokio::test]
    async fn test_has_logs_for_source() {
        let db = setup_db().await;
        let entries = sample_log_entries();
        db.insert_logs_batch(&entries).await.unwrap();

        assert!(db.has_logs_for_source("test.log").await.unwrap());
        assert!(!db.has_logs_for_source("other.log").await.unwrap());
    }

    #[tokio::test]
    async fn test_clear_logs() {
        let db = setup_db().await;
        let entries = sample_log_entries();
        db.insert_logs_batch(&entries).await.unwrap();
        assert_eq!(db.get_log_count().await.unwrap(), 3);

        db.clear_logs().await.unwrap();
        assert_eq!(db.get_log_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_insert_large_batch() {
        let db = setup_db().await;
        let entries: Vec<LogEntry> = (0..1500)
            .map(|i| LogEntry {
                id: i,
                message: format!("Log line {}", i),
                level: LogLevel::Info,
                ..Default::default()
            })
            .collect();

        db.insert_logs_batch(&entries).await.unwrap();
        assert_eq!(db.get_log_count().await.unwrap(), 1500);
    }

    #[tokio::test]
    async fn test_filter_crud() {
        let db = setup_db().await;

        // Insert
        let id1 = db
            .insert_filter("error", &FilterType::Include, true, None, None)
            .await
            .unwrap();
        let id2 = db
            .insert_filter("debug", &FilterType::Exclude, true, None, None)
            .await
            .unwrap();

        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert!(filters[0].enabled);
        assert_eq!(filters[1].pattern, "debug");
        assert_eq!(filters[1].filter_type, FilterType::Exclude);

        // Toggle
        db.toggle_filter(id1).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert!(!filters[0].enabled);

        db.toggle_filter(id1).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert!(filters[0].enabled);

        // Update pattern
        db.update_filter_pattern(id1, "warning").await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters[0].pattern, "warning");

        // Delete
        db.delete_filter(id2).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters.len(), 1);

        // Clear
        db.clear_filters().await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert!(filters.is_empty());
    }

    #[tokio::test]
    async fn test_filter_with_color() {
        let db = setup_db().await;
        let color = ColorConfig {
            fg: Some(ratatui::style::Color::Red),
            bg: Some(ratatui::style::Color::Blue),
            match_only: false,
        };

        db.insert_filter("error", &FilterType::Include, true, Some(&color), None)
            .await
            .unwrap();

        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters.len(), 1);
        let cc = filters[0].color_config.as_ref().unwrap();
        assert!(cc.fg.is_some());
        assert!(cc.bg.is_some());
    }

    #[tokio::test]
    async fn test_update_filter_color() {
        let db = setup_db().await;
        let id = db
            .insert_filter("error", &FilterType::Include, true, None, None)
            .await
            .unwrap();

        let color = ColorConfig {
            fg: Some(ratatui::style::Color::Green),
            bg: None,
            match_only: false,
        };
        db.update_filter_color(id, Some(&color)).await.unwrap();

        let filters = db.get_filters().await.unwrap();
        let cc = filters[0].color_config.as_ref().unwrap();
        assert!(cc.fg.is_some());
        assert!(cc.bg.is_none());
    }

    #[tokio::test]
    async fn test_swap_filter_order() {
        let db = setup_db().await;
        let id1 = db
            .insert_filter("first", &FilterType::Include, true, None, None)
            .await
            .unwrap();
        let id2 = db
            .insert_filter("second", &FilterType::Exclude, true, None, None)
            .await
            .unwrap();

        // Before swap: first(order=0), second(order=1)
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters[0].pattern, "first");
        assert_eq!(filters[1].pattern, "second");

        // After swap
        db.swap_filter_order(id1, id2).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters[0].pattern, "second");
        assert_eq!(filters[1].pattern, "first");
    }

    #[tokio::test]
    async fn test_replace_all_filters() {
        let db = setup_db().await;
        db.insert_filter("old1", &FilterType::Include, true, None, None)
            .await
            .unwrap();
        db.insert_filter("old2", &FilterType::Exclude, true, None, None)
            .await
            .unwrap();

        let new_filters = vec![
            Filter {
                id: 0,
                pattern: "new1".to_string(),
                filter_type: FilterType::Include,
                enabled: true,
                color_config: None,
            },
            Filter {
                id: 0,
                pattern: "new2".to_string(),
                filter_type: FilterType::Exclude,
                enabled: false,
                color_config: None,
            },
        ];

        db.replace_all_filters(&new_filters, None).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].pattern, "new1");
        assert_eq!(filters[1].pattern, "new2");
        assert!(!filters[1].enabled);
    }

    #[tokio::test]
    async fn test_empty_batch_insert() {
        let db = setup_db().await;
        db.insert_logs_batch(&[]).await.unwrap();
        assert_eq!(db.get_log_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_save_and_load_file_context() {
        let db = setup_db().await;

        let ctx = FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 42,
            search_query: "ERROR".to_string(),
            wrap: false,
            level_colors: true,
            show_sidebar: false,
            horizontal_scroll: 10,
            marked_lines: vec![1, 5, 10],
            file_hash: Some("abc123".to_string()),
        };
        db.save_file_context(&ctx).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/test.log")
            .await
            .unwrap()
            .expect("should find context");
        assert_eq!(loaded.scroll_offset, 42);
        assert_eq!(loaded.search_query, "ERROR");
        assert!(!loaded.wrap);
        assert!(loaded.level_colors);
        assert!(!loaded.show_sidebar);
        assert_eq!(loaded.horizontal_scroll, 10);
        assert_eq!(loaded.marked_lines, vec![1, 5, 10]);
        assert_eq!(loaded.file_hash, Some("abc123".to_string()));
    }

    #[tokio::test]
    async fn test_file_context_upsert() {
        let db = setup_db().await;

        let ctx1 = FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 10,
            search_query: "".to_string(),
            wrap: true,
            level_colors: true,
            show_sidebar: true,
            horizontal_scroll: 0,
            marked_lines: vec![0, 3],
            file_hash: Some("hash1".to_string()),
        };
        db.save_file_context(&ctx1).await.unwrap();

        let ctx2 = FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 99,
            search_query: "WARN".to_string(),
            wrap: false,
            level_colors: false,
            show_sidebar: false,
            horizontal_scroll: 5,
            marked_lines: vec![2, 7],
            file_hash: Some("hash2".to_string()),
        };
        db.save_file_context(&ctx2).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/test.log")
            .await
            .unwrap()
            .expect("should find context");
        assert_eq!(loaded.scroll_offset, 99);
        assert_eq!(loaded.search_query, "WARN");
        assert!(!loaded.wrap);
        assert_eq!(loaded.marked_lines, vec![2, 7]);
    }

    #[tokio::test]
    async fn test_file_context_not_found() {
        let db = setup_db().await;
        let result = db.load_file_context("/nonexistent").await.unwrap();
        assert!(result.is_none());
    }
}
