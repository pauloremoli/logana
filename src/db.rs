//! SQLite persistence layer via sqlx.
//!
//! Three trait abstractions: [`FilterStore`], [`FileContextStore`],
//! [`SessionStore`]. Schema versioning uses `PRAGMA user_version`;
//! `run_migrations` applies each migration exactly once.
//! [`Database::in_memory`] creates an in-memory DB for tests.
//!
//! ## Tables
//!
//! - `filters`: per `source_file` filter definitions
//! - `file_context`: per-file session state (PK: `source_file`)
//! - `session_tabs`: ordered list of last-open source files
//! - `app_settings`: global key/value store for runtime preferences
//!
//! ## Schema versioning
//!
//! `PRAGMA user_version` tracks the applied schema version. `run_migrations()`
//! reads the current version and calls `migrate_to_vN()` only for versions not
//! yet applied — each migration runs exactly once. To add a new migration: add
//! `migrate_to_vN` with the required SQL and an `if version < N` block in
//! `run_migrations`. Current versions:
//! - v1: initial schema (`filters`, `file_context`, `session_tabs` tables)
//! - v2: `ALTER TABLE file_context ADD COLUMN show_keys` — persists show/hide-keys
//!   display preference per file
//! - v3: `level_colors_disabled` JSON column; migrates legacy `level_colors = 0` rows
//! - v4: `raw_mode` column on `file_context`
//! - v5: `sidebar_width` column on `file_context`
//! - v6: `app_settings` table for global runtime key/value preferences
//! - v7: drop per-file `show_status_bar`, `show_borders`, `show_sidebar`,
//!   `show_line_numbers`, `wrap` columns — these are now global app settings
//! - v8: `hidden_fields` and `field_layout_columns` columns on `file_context`

use anyhow::Result;
use async_trait::async_trait;
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

use std::collections::HashSet;

use crate::types::{ColorConfig, Comment, FilterDef, FilterType};

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
    async fn get_filters(&self) -> Result<Vec<FilterDef>>;
    async fn get_filters_for_source(&self, source_file: &str) -> Result<Vec<FilterDef>>;
    async fn update_filter_pattern(&self, id: i64, new_pattern: &str) -> Result<()>;
    async fn update_filter_color(&self, id: i64, color_config: Option<&ColorConfig>) -> Result<()>;
    async fn update_filter(
        &self,
        id: i64,
        pattern: &str,
        filter_type: &FilterType,
        color_config: Option<&ColorConfig>,
    ) -> Result<()>;
    async fn delete_filter(&self, id: i64) -> Result<()>;
    async fn toggle_filter(&self, id: i64) -> Result<()>;
    async fn swap_filter_order(&self, id1: i64, id2: i64) -> Result<()>;
    async fn clear_filters(&self) -> Result<()>;
    async fn clear_filters_for_source(&self, source_file: &str) -> Result<()>;
    async fn replace_all_filters(
        &self,
        filters: &[FilterDef],
        source_file: Option<&str>,
    ) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileContext {
    pub source_file: String,
    pub scroll_offset: usize,
    pub search_query: String,
    /// Set of log-level keys whose colour is disabled (e.g. `"trace"`, `"error"`).
    /// Stored as a JSON array in `level_colors_disabled` column.
    pub level_colors_disabled: HashSet<String>,
    pub horizontal_scroll: usize,
    pub marked_lines: Vec<usize>,
    pub file_hash: Option<String>,
    pub comments: Vec<Comment>,
    pub show_keys: bool,
    /// When true, the format parser is bypassed and lines are shown as raw bytes.
    pub raw_mode: bool,
    /// Width in terminal columns of the filter sidebar (default 30, min 10).
    pub sidebar_width: u16,
    /// Set of hidden field names (e.g. `"span.request_id"`, `"level"`).
    pub hidden_fields: HashSet<String>,
    /// Ordered list of all column names from the select-fields modal (visible + hidden).
    pub field_layout_columns: Option<Vec<String>>,
    /// Whether the global filtering toggle is enabled (default true).
    pub filtering_enabled: bool,
}

#[async_trait]
pub trait FileContextStore: Send + Sync {
    async fn save_file_context(&self, ctx: &FileContext) -> Result<()>;
    async fn load_file_context(&self, source_file: &str) -> Result<Option<FileContext>>;
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Persist the ordered list of open source files as the last session.
    async fn save_session(&self, files: &[String]) -> Result<()>;
    /// Load the ordered list of source files from the last saved session.
    async fn load_session(&self) -> Result<Vec<String>>;
}

#[async_trait]
pub trait AppSettingsStore: Send + Sync {
    /// Persist a named application setting value.
    async fn save_app_setting(&self, key: &str, value: &str) -> Result<()>;
    /// Load a named application setting, returning `None` if not set.
    async fn load_app_setting(&self, key: &str) -> Result<Option<String>>;
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
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        if let Ok(db) = Self::open(path).await {
            return Ok(db);
        }

        // Opening failed (corrupted file, stale WAL, etc.) — remove all
        // SQLite-related files for this path and try once more from scratch.
        for suffix in &["", "-wal", "-shm"] {
            let candidate = format!("{}{}", path, suffix);
            let _ = std::fs::remove_file(&candidate);
        }
        Self::open(path).await
    }

    async fn open(path: &str) -> Result<Self> {
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
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await?;

        if version < 1 {
            self.migrate_to_v1().await?;
            sqlx::query("PRAGMA user_version = 1")
                .execute(&self.pool)
                .await?;
        }

        if version < 2 {
            self.migrate_to_v2().await?;
            sqlx::query("PRAGMA user_version = 2")
                .execute(&self.pool)
                .await?;
        }

        if version < 3 {
            self.migrate_to_v3().await?;
            sqlx::query("PRAGMA user_version = 3")
                .execute(&self.pool)
                .await?;
        }

        if version < 4 {
            self.migrate_to_v4().await?;
            sqlx::query("PRAGMA user_version = 4")
                .execute(&self.pool)
                .await?;
        }

        if version < 5 {
            self.migrate_to_v5().await?;
            sqlx::query("PRAGMA user_version = 5")
                .execute(&self.pool)
                .await?;
        }

        if version < 6 {
            self.migrate_to_v6().await?;
            sqlx::query("PRAGMA user_version = 6")
                .execute(&self.pool)
                .await?;
        }

        if version < 7 {
            self.migrate_to_v7().await?;
            sqlx::query("PRAGMA user_version = 7")
                .execute(&self.pool)
                .await?;
        }

        if version < 8 {
            self.migrate_to_v8().await?;
            sqlx::query("PRAGMA user_version = 8")
                .execute(&self.pool)
                .await?;
        }

        if version < 9 {
            self.migrate_to_v9().await?;
            sqlx::query("PRAGMA user_version = 9")
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    async fn migrate_to_v1(&self) -> Result<()> {
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
                match_only INTEGER NOT NULL DEFAULT 1
            )",
        )
        .execute(&self.pool)
        .await?;

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
                file_hash TEXT,
                show_line_numbers INTEGER NOT NULL DEFAULT 1,
                annotations_json TEXT NOT NULL DEFAULT '[]',
                show_status_bar INTEGER NOT NULL DEFAULT 1,
                show_borders INTEGER NOT NULL DEFAULT 1
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS session_tabs (
                source_file TEXT NOT NULL,
                tab_order INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn migrate_to_v2(&self) -> Result<()> {
        sqlx::query("ALTER TABLE file_context ADD COLUMN show_keys INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await
            .ok(); // column may already exist on fresh DBs created from v1 schema
        Ok(())
    }

    async fn migrate_to_v3(&self) -> Result<()> {
        // Add a JSON column for per-level colour disabling.
        sqlx::query(
            "ALTER TABLE file_context ADD COLUMN level_colors_disabled TEXT NOT NULL DEFAULT '[]'",
        )
        .execute(&self.pool)
        .await
        .ok(); // column may already exist on fresh DBs

        // Convert old rows where level_colors = 0 (all levels disabled) to the new format.
        sqlx::query(
            "UPDATE file_context SET level_colors_disabled = '[\"trace\",\"debug\",\"info\",\"notice\",\"warning\",\"error\",\"fatal\"]' WHERE level_colors = 0 AND level_colors_disabled = '[]'",
        )
        .execute(&self.pool)
        .await
        .ok();

        Ok(())
    }

    async fn migrate_to_v4(&self) -> Result<()> {
        sqlx::query("ALTER TABLE file_context ADD COLUMN raw_mode INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await
            .ok(); // column may already exist on fresh DBs
        Ok(())
    }

    async fn migrate_to_v5(&self) -> Result<()> {
        sqlx::query(
            "ALTER TABLE file_context ADD COLUMN sidebar_width INTEGER NOT NULL DEFAULT 30",
        )
        .execute(&self.pool)
        .await
        .ok(); // column may already exist on fresh DBs
        Ok(())
    }

    async fn migrate_to_v6(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn migrate_to_v7(&self) -> Result<()> {
        for col in &[
            "show_status_bar",
            "show_borders",
            "show_sidebar",
            "show_line_numbers",
            "wrap",
        ] {
            sqlx::query(&format!("ALTER TABLE file_context DROP COLUMN {}", col))
                .execute(&self.pool)
                .await
                .ok();
        }
        Ok(())
    }

    async fn migrate_to_v8(&self) -> Result<()> {
        sqlx::query("ALTER TABLE file_context ADD COLUMN hidden_fields TEXT NOT NULL DEFAULT '[]'")
            .execute(&self.pool)
            .await
            .ok();
        sqlx::query("ALTER TABLE file_context ADD COLUMN field_layout_columns TEXT")
            .execute(&self.pool)
            .await
            .ok();
        Ok(())
    }

    async fn migrate_to_v9(&self) -> Result<()> {
        sqlx::query(
            "ALTER TABLE file_context ADD COLUMN filtering_enabled INTEGER NOT NULL DEFAULT 1",
        )
        .execute(&self.pool)
        .await
        .ok();
        Ok(())
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

fn row_to_filter_def(row: &sqlx::sqlite::SqliteRow) -> FilterDef {
    let fg_str: Option<String> = row.get("fg_color");
    let bg_str: Option<String> = row.get("bg_color");
    let match_only = row.get::<i32, _>("match_only") != 0;

    let color_config = match (fg_str, bg_str) {
        (None, None) if match_only => None,
        (fg, bg) => Some(ColorConfig {
            fg: fg.and_then(|s| s.parse().ok()),
            bg: bg.and_then(|s| s.parse().ok()),
            match_only,
        }),
    };

    FilterDef {
        id: row.get::<i64, _>("id") as usize,
        pattern: row.get("pattern"),
        filter_type: str_to_filter_type(row.get::<&str, _>("filter_type")),
        enabled: row.get::<i32, _>("enabled") != 0,
        color_config,
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

        let next_order = max_order.unwrap_or(0) + 1;

        let (fg, bg, match_only) = match color_config {
            Some(cc) => (
                cc.fg.map(|c| c.to_string()),
                cc.bg.map(|c| c.to_string()),
                cc.match_only,
            ),
            None => (None, None, true),
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

    async fn get_filters(&self) -> Result<Vec<FilterDef>> {
        // Returns "global" filters (source_file = '').
        self.get_filters_for_source("").await
    }

    async fn get_filters_for_source(&self, source_file: &str) -> Result<Vec<FilterDef>> {
        let rows =
            sqlx::query("SELECT * FROM filters WHERE source_file = ? ORDER BY display_order")
                .bind(source_file)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.iter().map(row_to_filter_def).collect())
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
            None => (None, None, true),
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

    async fn update_filter(
        &self,
        id: i64,
        pattern: &str,
        filter_type: &FilterType,
        color_config: Option<&ColorConfig>,
    ) -> Result<()> {
        let (fg, bg, match_only) = match color_config {
            Some(cc) => (
                cc.fg.map(|c| c.to_string()),
                cc.bg.map(|c| c.to_string()),
                cc.match_only,
            ),
            None => (None, None, true),
        };
        sqlx::query(
            "UPDATE filters SET pattern = ?, filter_type = ?, fg_color = ?, bg_color = ?, match_only = ? WHERE id = ?",
        )
        .bind(pattern)
        .bind(filter_type_to_str(filter_type))
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

    async fn clear_filters_for_source(&self, source_file: &str) -> Result<()> {
        sqlx::query("DELETE FROM filters WHERE source_file = ?")
            .bind(source_file)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn replace_all_filters(
        &self,
        filters: &[FilterDef],
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
                None => (None, None, true),
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
        let comments_json =
            serde_json::to_string(&ctx.comments).unwrap_or_else(|_| "[]".to_string());
        let level_colors_disabled_json =
            serde_json::to_string(&ctx.level_colors_disabled.iter().collect::<Vec<_>>())
                .unwrap_or_else(|_| "[]".to_string());
        let hidden_fields_json =
            serde_json::to_string(&ctx.hidden_fields.iter().collect::<Vec<_>>())
                .unwrap_or_else(|_| "[]".to_string());
        let field_layout_columns_json = ctx
            .field_layout_columns
            .as_ref()
            .and_then(|cols| serde_json::to_string(cols).ok());
        // Also keep the legacy `level_colors` column up-to-date for any old readers.
        let level_colors_legacy = ctx.level_colors_disabled.is_empty() as i32;
        sqlx::query(
            "INSERT INTO file_context (source_file, scroll_offset, search_query, level_colors, horizontal_scroll, marked_lines, file_hash, annotations_json, show_keys, level_colors_disabled, raw_mode, sidebar_width, hidden_fields, field_layout_columns, filtering_enabled)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(source_file) DO UPDATE SET
                scroll_offset = excluded.scroll_offset,
                search_query = excluded.search_query,
                level_colors = excluded.level_colors,
                horizontal_scroll = excluded.horizontal_scroll,
                marked_lines = excluded.marked_lines,
                file_hash = excluded.file_hash,
                annotations_json = excluded.annotations_json,
                show_keys = excluded.show_keys,
                level_colors_disabled = excluded.level_colors_disabled,
                raw_mode = excluded.raw_mode,
                sidebar_width = excluded.sidebar_width,
                hidden_fields = excluded.hidden_fields,
                field_layout_columns = excluded.field_layout_columns,
                filtering_enabled = excluded.filtering_enabled",
        )
        .bind(&ctx.source_file)
        .bind(ctx.scroll_offset as i64)
        .bind(&ctx.search_query)
        .bind(level_colors_legacy)
        .bind(ctx.horizontal_scroll as i64)
        .bind(&marked_json)
        .bind(&ctx.file_hash)
        .bind(&comments_json)
        .bind(ctx.show_keys as i32)
        .bind(&level_colors_disabled_json)
        .bind(ctx.raw_mode as i32)
        .bind(ctx.sidebar_width as i64)
        .bind(&hidden_fields_json)
        .bind(&field_layout_columns_json)
        .bind(ctx.filtering_enabled as i32)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_file_context(&self, source_file: &str) -> Result<Option<FileContext>> {
        let row = sqlx::query(
            "SELECT source_file, scroll_offset, search_query, level_colors, horizontal_scroll, marked_lines, file_hash, annotations_json, show_keys, level_colors_disabled, raw_mode, sidebar_width, hidden_fields, field_layout_columns, filtering_enabled
             FROM file_context WHERE source_file = ?",
        )
        .bind(source_file)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let marked_json: String = r.get("marked_lines");
            let marked_lines: Vec<usize> = serde_json::from_str(&marked_json).unwrap_or_default();
            let comments_json: String = r.try_get("annotations_json").unwrap_or_default();
            let comments: Vec<Comment> = serde_json::from_str(&comments_json).unwrap_or_default();
            let level_colors_disabled: HashSet<String> = r
                .try_get::<String, _>("level_colors_disabled")
                .ok()
                .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                .map(|v| v.into_iter().collect())
                .unwrap_or_else(|| {
                    // Legacy fallback: old level_colors = 0 meant all levels disabled.
                    if r.get::<i32, _>("level_colors") == 0 {
                        ["trace", "debug", "notice", "warning", "error", "fatal"]
                            .iter()
                            .map(|s| s.to_string())
                            .collect()
                    } else {
                        HashSet::new()
                    }
                });
            let hidden_fields: HashSet<String> = r
                .try_get::<String, _>("hidden_fields")
                .ok()
                .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                .map(|v| v.into_iter().collect())
                .unwrap_or_default();
            let field_layout_columns: Option<Vec<String>> = r
                .try_get::<Option<String>, _>("field_layout_columns")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
            FileContext {
                source_file: r.get::<String, _>("source_file"),
                scroll_offset: r.get::<i64, _>("scroll_offset") as usize,
                search_query: r.get::<String, _>("search_query"),
                level_colors_disabled,
                horizontal_scroll: r.get::<i64, _>("horizontal_scroll") as usize,
                marked_lines,
                file_hash: r.get::<Option<String>, _>("file_hash"),
                comments,
                show_keys: r.try_get::<i32, _>("show_keys").unwrap_or(0) != 0,
                raw_mode: r.try_get::<i32, _>("raw_mode").unwrap_or(0) != 0,
                sidebar_width: r
                    .try_get::<i64, _>("sidebar_width")
                    .unwrap_or(30)
                    .clamp(10, 200) as u16,
                hidden_fields,
                field_layout_columns,
                filtering_enabled: r.try_get::<i32, _>("filtering_enabled").unwrap_or(1) != 0,
            }
        }))
    }
}

#[async_trait]
impl SessionStore for Database {
    async fn save_session(&self, files: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM session_tabs")
            .execute(&mut *tx)
            .await?;
        for (order, file) in files.iter().enumerate() {
            sqlx::query("INSERT INTO session_tabs (source_file, tab_order) VALUES (?, ?)")
                .bind(file)
                .bind(order as i64)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn load_session(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT source_file FROM session_tabs ORDER BY tab_order")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| r.get::<String, _>("source_file"))
            .collect())
    }
}

#[async_trait]
impl AppSettingsStore for Database {
    async fn save_app_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_app_setting(&self, key: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM app_settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("value")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> Database {
        Database::in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn test_filter_crud() {
        let db = setup_db().await;

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
        // Oldest first: "error" was inserted first, so it has the lower display_order
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert!(filters[0].enabled);
        assert_eq!(filters[1].pattern, "debug");
        assert_eq!(filters[1].filter_type, FilterType::Exclude);

        // Toggle id1 ("error", at index 0)
        db.toggle_filter(id1).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert!(!filters[0].enabled);

        db.toggle_filter(id1).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert!(filters[0].enabled);

        // Update pattern of id1 ("error" → "warning", still at index 0)
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

        let filters = db.get_filters().await.unwrap();
        // Oldest first: "first" was inserted first so it has the lower display_order
        assert_eq!(filters[0].pattern, "first");
        assert_eq!(filters[1].pattern, "second");

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
            FilterDef {
                id: 0,
                pattern: "new1".to_string(),
                filter_type: FilterType::Include,
                enabled: true,
                color_config: None,
            },
            FilterDef {
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
    async fn test_clear_filters_for_source() {
        let db = setup_db().await;
        db.insert_filter("global", &FilterType::Include, true, None, None)
            .await
            .unwrap();
        db.insert_filter(
            "file-specific",
            &FilterType::Include,
            true,
            None,
            Some("test.log"),
        )
        .await
        .unwrap();

        db.clear_filters_for_source("test.log").await.unwrap();
        let global = db.get_filters().await.unwrap();
        let file_filters = db.get_filters_for_source("test.log").await.unwrap();
        assert_eq!(global.len(), 1);
        assert_eq!(file_filters.len(), 0);
    }

    #[tokio::test]
    async fn test_get_filters_for_source() {
        let db = setup_db().await;
        db.insert_filter("global", &FilterType::Include, true, None, None)
            .await
            .unwrap();
        db.insert_filter(
            "file1",
            &FilterType::Exclude,
            true,
            None,
            Some("/var/log/syslog"),
        )
        .await
        .unwrap();
        db.insert_filter(
            "file2",
            &FilterType::Include,
            true,
            None,
            Some("/var/log/syslog"),
        )
        .await
        .unwrap();

        let global = db.get_filters().await.unwrap();
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].pattern, "global");

        let syslog_filters = db.get_filters_for_source("/var/log/syslog").await.unwrap();
        assert_eq!(syslog_filters.len(), 2);
    }

    #[tokio::test]
    async fn test_save_and_load_file_context() {
        let db = setup_db().await;

        let ctx = FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 42,
            search_query: "ERROR".to_string(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 10,
            marked_lines: vec![1, 5, 10],
            file_hash: Some("abc123".to_string()),
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/test.log")
            .await
            .unwrap()
            .expect("should find context");
        assert_eq!(loaded.scroll_offset, 42);
        assert_eq!(loaded.search_query, "ERROR");
        assert!(loaded.level_colors_disabled.is_empty());
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
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![0, 3],
            file_hash: Some("hash1".to_string()),
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx1).await.unwrap();

        let ctx2 = FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 99,
            search_query: "WARN".to_string(),
            level_colors_disabled: [
                "trace", "debug", "info", "notice", "warning", "error", "fatal",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            horizontal_scroll: 5,
            marked_lines: vec![2, 7],
            file_hash: Some("hash2".to_string()),
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx2).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/test.log")
            .await
            .unwrap()
            .expect("should find context");
        assert_eq!(loaded.scroll_offset, 99);
        assert_eq!(loaded.search_query, "WARN");
        assert_eq!(loaded.marked_lines, vec![2, 7]);
    }

    #[tokio::test]
    async fn test_file_context_not_found() {
        let db = setup_db().await;
        let result = db.load_file_context("/nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_file_context_saves_and_loads_comments() {
        use crate::types::Comment;
        let db = setup_db().await;

        let ctx = FileContext {
            source_file: "/tmp/commented.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![
                Comment {
                    text: "First comment\nspanning two lines".to_string(),
                    line_indices: vec![1, 2, 3],
                },
                Comment {
                    text: "Second comment".to_string(),
                    line_indices: vec![10],
                },
            ],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/commented.log")
            .await
            .unwrap()
            .expect("context should exist");

        assert_eq!(loaded.comments.len(), 2);
        assert_eq!(loaded.comments[0].text, "First comment\nspanning two lines");
        assert_eq!(loaded.comments[0].line_indices, vec![1, 2, 3]);
        assert_eq!(loaded.comments[1].text, "Second comment");
        assert_eq!(loaded.comments[1].line_indices, vec![10]);
    }

    #[tokio::test]
    async fn test_file_context_round_trip_with_show_keys() {
        let db = setup_db().await;

        let ctx = FileContext {
            source_file: "/tmp/display.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: true,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/display.log")
            .await
            .unwrap()
            .expect("context should exist");

        assert!(loaded.show_keys);
    }

    #[tokio::test]
    async fn test_file_context_show_keys_persisted() {
        let db = setup_db().await;

        let ctx = FileContext {
            source_file: "/tmp/show_keys.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: true,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/show_keys.log")
            .await
            .unwrap()
            .expect("context should exist");

        assert!(loaded.show_keys);
    }

    #[tokio::test]
    async fn test_sidebar_width_round_trips() {
        let db = setup_db().await;
        let ctx = FileContext {
            source_file: "/tmp/sidebar.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 45,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/sidebar.log")
            .await
            .unwrap()
            .expect("context should exist");

        assert_eq!(loaded.sidebar_width, 45);
    }

    #[tokio::test]
    async fn test_hidden_fields_and_field_layout_columns_round_trip() {
        let db = setup_db().await;
        let mut hidden = HashSet::new();
        hidden.insert("span.request_id".to_string());
        hidden.insert("level".to_string());
        let columns = Some(vec![
            "timestamp".to_string(),
            "level".to_string(),
            "span".to_string(),
        ]);
        let ctx = FileContext {
            source_file: "/tmp/layout.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: hidden.clone(),
            field_layout_columns: columns.clone(),
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/layout.log")
            .await
            .unwrap()
            .expect("context should exist");

        assert_eq!(loaded.hidden_fields, hidden);
        assert_eq!(loaded.field_layout_columns, columns);
    }

    #[tokio::test]
    async fn test_filtering_enabled_round_trips() {
        let db = setup_db().await;

        let ctx = FileContext {
            source_file: "/tmp/filtering.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: false,
        };
        db.save_file_context(&ctx).await.unwrap();

        let loaded = db
            .load_file_context("/tmp/filtering.log")
            .await
            .unwrap()
            .expect("context should exist");

        assert!(!loaded.filtering_enabled);

        let ctx2 = FileContext {
            filtering_enabled: true,
            ..ctx
        };
        db.save_file_context(&ctx2).await.unwrap();

        let loaded2 = db
            .load_file_context("/tmp/filtering.log")
            .await
            .unwrap()
            .expect("context should exist");

        assert!(loaded2.filtering_enabled);
    }

    // ── AppSettingsStore ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_app_setting_load_returns_none_when_not_set() {
        let db = setup_db().await;
        let result = db.load_app_setting("restore_session").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_app_setting_save_and_load() {
        let db = setup_db().await;
        db.save_app_setting("restore_session", "always")
            .await
            .unwrap();
        let value = db.load_app_setting("restore_session").await.unwrap();
        assert_eq!(value.as_deref(), Some("always"));
    }

    #[tokio::test]
    async fn test_app_setting_save_overwrites() {
        let db = setup_db().await;
        db.save_app_setting("restore_session", "always")
            .await
            .unwrap();
        db.save_app_setting("restore_session", "never")
            .await
            .unwrap();
        let value = db.load_app_setting("restore_session").await.unwrap();
        assert_eq!(value.as_deref(), Some("never"));
    }

    #[tokio::test]
    async fn test_app_setting_file_policy_independent_of_session_policy() {
        let db = setup_db().await;
        db.save_app_setting("restore_file_context", "never")
            .await
            .unwrap();
        let session = db.load_app_setting("restore_session").await.unwrap();
        let file = db.load_app_setting("restore_file_context").await.unwrap();
        assert!(session.is_none());
        assert_eq!(file.as_deref(), Some("never"));
    }

    #[tokio::test]
    async fn test_app_setting_session_policy_independent_of_file_policy() {
        let db = setup_db().await;
        db.save_app_setting("restore_session", "always")
            .await
            .unwrap();
        let session = db.load_app_setting("restore_session").await.unwrap();
        let file = db.load_app_setting("restore_file_context").await.unwrap();
        assert_eq!(session.as_deref(), Some("always"));
        assert!(file.is_none());
    }
}
