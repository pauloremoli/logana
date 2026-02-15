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
    ) -> Result<i64>;
    async fn get_filters(&self) -> Result<Vec<Filter>>;
    async fn update_filter_pattern(&self, id: i64, new_pattern: &str) -> Result<()>;
    async fn update_filter_color(&self, id: i64, color_config: Option<&ColorConfig>) -> Result<()>;
    async fn delete_filter(&self, id: i64) -> Result<()>;
    async fn toggle_filter(&self, id: i64) -> Result<()>;
    async fn swap_filter_order(&self, id1: i64, id2: i64) -> Result<()>;
    async fn clear_filters(&self) -> Result<()>;
    async fn replace_all_filters(&self, filters: &[Filter]) -> Result<()>;
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
        db.run_migrations().await?;
        Ok(db)
    }

    pub async fn in_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;

        let db = Self { pool };
        db.run_migrations().await?;
        Ok(db)
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
                display_order INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

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

    let color_config = match (fg_str, bg_str) {
        (None, None) => None,
        (fg, bg) => Some(ColorConfig {
            fg: fg.and_then(|s| s.parse().ok()),
            bg: bg.and_then(|s| s.parse().ok()),
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
        const BATCH_SIZE: usize = 500;

        for chunk in entries.chunks(BATCH_SIZE) {
            let mut tx = self.pool.begin().await?;

            for entry in chunk {
                sqlx::query(
                    "INSERT INTO log_entries (id, timestamp, hostname, process_name, pid, level, message, marked, source_file)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(entry.id as i64)
                .bind(&entry.timestamp)
                .bind(&entry.hostname)
                .bind(&entry.process_name)
                .bind(entry.pid.map(|p| p as i64))
                .bind(log_level_to_str(&entry.level))
                .bind(&entry.message)
                .bind(entry.marked as i32)
                .bind(&entry.source_file)
                .execute(&mut *tx)
                .await?;
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
    ) -> Result<i64> {
        let max_order: Option<i64> =
            sqlx::query("SELECT MAX(display_order) as max_order FROM filters")
                .fetch_one(&self.pool)
                .await?
                .get("max_order");

        let next_order = max_order.unwrap_or(-1) + 1;

        let (fg, bg) = match color_config {
            Some(cc) => (cc.fg.map(|c| c.to_string()), cc.bg.map(|c| c.to_string())),
            None => (None, None),
        };

        let result = sqlx::query(
            "INSERT INTO filters (pattern, filter_type, enabled, fg_color, bg_color, display_order)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(pattern)
        .bind(filter_type_to_str(filter_type))
        .bind(enabled as i32)
        .bind(&fg)
        .bind(&bg)
        .bind(next_order)
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

    async fn update_filter_pattern(&self, id: i64, new_pattern: &str) -> Result<()> {
        sqlx::query("UPDATE filters SET pattern = ? WHERE id = ?")
            .bind(new_pattern)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_filter_color(&self, id: i64, color_config: Option<&ColorConfig>) -> Result<()> {
        let (fg, bg) = match color_config {
            Some(cc) => (cc.fg.map(|c| c.to_string()), cc.bg.map(|c| c.to_string())),
            None => (None, None),
        };

        sqlx::query("UPDATE filters SET fg_color = ?, bg_color = ? WHERE id = ?")
            .bind(&fg)
            .bind(&bg)
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

    async fn replace_all_filters(&self, filters: &[Filter]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM filters").execute(&mut *tx).await?;

        for (order, filter) in filters.iter().enumerate() {
            let (fg, bg) = match &filter.color_config {
                Some(cc) => (cc.fg.map(|c| c.to_string()), cc.bg.map(|c| c.to_string())),
                None => (None, None),
            };

            sqlx::query(
                "INSERT INTO filters (pattern, filter_type, enabled, fg_color, bg_color, display_order)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&filter.pattern)
            .bind(filter_type_to_str(&filter.filter_type))
            .bind(filter.enabled as i32)
            .bind(&fg)
            .bind(&bg)
            .bind(order as i64)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
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
            .insert_filter("error", &FilterType::Include, true, None)
            .await
            .unwrap();
        let id2 = db
            .insert_filter("debug", &FilterType::Exclude, true, None)
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
        };

        db.insert_filter("error", &FilterType::Include, true, Some(&color))
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
            .insert_filter("error", &FilterType::Include, true, None)
            .await
            .unwrap();

        let color = ColorConfig {
            fg: Some(ratatui::style::Color::Green),
            bg: None,
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
            .insert_filter("first", &FilterType::Include, true, None)
            .await
            .unwrap();
        let id2 = db
            .insert_filter("second", &FilterType::Exclude, true, None)
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
        db.insert_filter("old1", &FilterType::Include, true, None)
            .await
            .unwrap();
        db.insert_filter("old2", &FilterType::Exclude, true, None)
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

        db.replace_all_filters(&new_filters).await.unwrap();
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
}
