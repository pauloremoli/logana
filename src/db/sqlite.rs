use crate::db::Comment;
use crate::filters::{ColorConfig, FilterDef, FilterInsertOptions, FilterType, GroupDef};
use anyhow::Result;
use async_trait::async_trait;
use sqlx::Connection;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnection, SqlitePool, SqlitePoolOptions};
use std::collections::HashSet;

#[async_trait]
pub trait FilterStore: Send + Sync {
    async fn insert_filter(
        &self,
        pattern: &str,
        filter_type: &FilterType,
        options: FilterInsertOptions,
    ) -> Result<i64>;
    async fn get_filters(&self) -> Result<Vec<FilterDef>>;
    async fn get_filters_for_source(&self, source_file: &str) -> Result<Vec<FilterDef>>;
    async fn update_filter_pattern(&self, id: i64, new_pattern: &str) -> Result<()>;
    async fn update_filter_color(&self, id: i64, color_config: Option<&ColorConfig>) -> Result<()>;
    async fn update_filter_group(&self, id: i64, group: Option<&str>) -> Result<()>;
    #[allow(clippy::too_many_arguments)]
    async fn update_filter(
        &self,
        id: i64,
        pattern: &str,
        filter_type: &FilterType,
        color_config: Option<&ColorConfig>,
        use_regex: bool,
        ignore_case: bool,
        group: Option<&str>,
    ) -> Result<()>;
    async fn delete_filter(&self, id: i64) -> Result<()>;
    async fn toggle_filter(&self, id: i64) -> Result<()>;
    async fn set_all_filters_enabled(&self, enabled: bool) -> Result<()>;
    async fn set_filters_enabled_by_group(
        &self,
        source_file: &str,
        group: &str,
        enabled: bool,
    ) -> Result<()>;
    async fn swap_filter_order(&self, id1: i64, id2: i64) -> Result<()>;
    async fn clear_filters(&self) -> Result<()>;
    async fn clear_filters_for_source(&self, source_file: &str) -> Result<()>;
    async fn replace_all_filters(
        &self,
        filters: &[FilterDef],
        source_file: Option<&str>,
    ) -> Result<()>;
}

#[async_trait]
pub trait GroupStore: Send + Sync {
    /// Groups with no source file (the `''` bucket) — mirrors `FilterStore::get_filters`.
    async fn get_groups(&self) -> Result<Vec<GroupDef>>;
    async fn get_groups_for_source(&self, source_file: &str) -> Result<Vec<GroupDef>>;
    async fn upsert_group_style(
        &self,
        source_file: &str,
        name: &str,
        color_config: &ColorConfig,
    ) -> Result<()>;
    async fn clear_group_style(&self, source_file: &str, name: &str) -> Result<()>;
    async fn replace_all_groups(
        &self,
        groups: &[GroupDef],
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsKey {
    RestoreSession,
    RestoreFileContext,
    Theme,
    Wrap,
    ShowModeBar,
    ShowBorders,
    ShowLineNumbers,
    RelativeLineNumbers,
    ShowSidebar,
    SidebarLeft,
    SidebarWidth,
    DefaultFilterFiles,
    CollapseContinuations,
    ShowGroupsPanel,
}

impl SettingsKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RestoreSession => "restore_session",
            Self::RestoreFileContext => "restore_file_context",
            Self::Theme => "theme",
            Self::Wrap => "wrap",
            Self::ShowModeBar => "show_mode_bar",
            Self::ShowBorders => "show_borders",
            Self::ShowLineNumbers => "show_line_numbers",
            Self::RelativeLineNumbers => "relative_line_numbers",
            Self::ShowSidebar => "show_sidebar",
            Self::SidebarLeft => "sidebar_left",
            Self::SidebarWidth => "sidebar_width",
            Self::DefaultFilterFiles => "default_filter_files",
            Self::CollapseContinuations => "collapse_continuations",
            Self::ShowGroupsPanel => "show_groups_panel",
        }
    }
}

impl std::fmt::Display for SettingsKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[async_trait]
pub trait AppSettingsStore: Send + Sync {
    /// Persist a named application setting value.
    async fn save_app_setting(&self, key: SettingsKey, value: &str) -> Result<()>;
    /// Load a named application setting, returning `None` if not set.
    async fn load_app_setting(&self, key: SettingsKey) -> Result<Option<String>>;
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

        // Pragmas and migrations run on one dedicated connection instead of
        // round-tripping through the pool per statement — schema-changing
        // statements (especially the v12 table rebuild) must not be
        // interleaved with other pooled connections picking up a stale view
        // of the schema.
        let mut conn = pool.acquire().await?;
        Self::configure_pragmas(&mut conn).await?;
        Self::run_migrations(&mut conn).await?;
        drop(conn);

        Ok(Self { pool })
    }

    pub async fn in_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;

        let mut conn = pool.acquire().await?;
        Self::configure_pragmas(&mut conn).await?;
        Self::run_migrations(&mut conn).await?;
        drop(conn);

        let db = Self { pool };
        Ok(db)
    }

    async fn configure_pragmas(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&mut *conn)
            .await?;
        sqlx::query("PRAGMA synchronous = NORMAL")
            .execute(&mut *conn)
            .await?;
        sqlx::query("PRAGMA cache_size = -64000")
            .execute(&mut *conn)
            .await?;
        Ok(())
    }

    /// Runs every unapplied migration in order, on `conn` alone — schema
    /// changes must not be interleaved with other pooled connections
    /// picking up a stale view of the schema (see `open`).
    async fn run_migrations(conn: &mut SqliteConnection) -> Result<()> {
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *conn)
            .await?;

        if version < 1 {
            Self::migrate_to_v1(conn).await?;
            sqlx::query("PRAGMA user_version = 1")
                .execute(&mut *conn)
                .await?;
        }

        if version < 2 {
            Self::migrate_to_v2(conn).await?;
            sqlx::query("PRAGMA user_version = 2")
                .execute(&mut *conn)
                .await?;
        }

        if version < 3 {
            Self::migrate_to_v3(conn).await?;
            sqlx::query("PRAGMA user_version = 3")
                .execute(&mut *conn)
                .await?;
        }

        if version < 4 {
            Self::migrate_to_v4(conn).await?;
            sqlx::query("PRAGMA user_version = 4")
                .execute(&mut *conn)
                .await?;
        }

        if version < 5 {
            Self::migrate_to_v5(conn).await?;
            sqlx::query("PRAGMA user_version = 5")
                .execute(&mut *conn)
                .await?;
        }

        if version < 6 {
            Self::migrate_to_v6(conn).await?;
            sqlx::query("PRAGMA user_version = 6")
                .execute(&mut *conn)
                .await?;
        }

        if version < 7 {
            Self::migrate_to_v7(conn).await?;
            sqlx::query("PRAGMA user_version = 7")
                .execute(&mut *conn)
                .await?;
        }

        if version < 8 {
            Self::migrate_to_v8(conn).await?;
            sqlx::query("PRAGMA user_version = 8")
                .execute(&mut *conn)
                .await?;
        }

        if version < 9 {
            Self::migrate_to_v9(conn).await?;
            sqlx::query("PRAGMA user_version = 9")
                .execute(&mut *conn)
                .await?;
        }

        if version < 10 {
            Self::migrate_to_v10(conn).await?;
            sqlx::query("PRAGMA user_version = 10")
                .execute(&mut *conn)
                .await?;
        }

        if version < 11 {
            Self::migrate_to_v11(conn).await?;
            sqlx::query("PRAGMA user_version = 11")
                .execute(&mut *conn)
                .await?;
        }

        if version < 12 {
            Self::migrate_to_v12(conn).await?;
            sqlx::query("PRAGMA user_version = 12")
                .execute(&mut *conn)
                .await?;
        }

        if version < 13 {
            Self::migrate_to_v13(conn).await?;
            sqlx::query("PRAGMA user_version = 13")
                .execute(&mut *conn)
                .await?;
        }

        if version < 14 {
            Self::migrate_to_v14(conn).await?;
            sqlx::query("PRAGMA user_version = 14")
                .execute(&mut *conn)
                .await?;
        }

        if version < 15 {
            Self::migrate_to_v15(conn).await?;
            sqlx::query("PRAGMA user_version = 15")
                .execute(&mut *conn)
                .await?;
        }

        if version < 16 {
            Self::migrate_to_v16(conn).await?;
            sqlx::query("PRAGMA user_version = 16")
                .execute(&mut *conn)
                .await?;
        }

        Ok(())
    }

    async fn migrate_to_v1(conn: &mut SqliteConnection) -> Result<()> {
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
        .execute(&mut *conn)
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
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS session_tabs (
                source_file TEXT NOT NULL,
                tab_order INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&mut *conn)
        .await?;

        Ok(())
    }

    async fn migrate_to_v2(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query("ALTER TABLE file_context ADD COLUMN show_keys INTEGER NOT NULL DEFAULT 0")
            .execute(&mut *conn)
            .await
            .ok(); // column may already exist on fresh DBs created from v1 schema
        Ok(())
    }

    async fn migrate_to_v3(conn: &mut SqliteConnection) -> Result<()> {
        // Add a JSON column for per-level colour disabling.
        sqlx::query(
            "ALTER TABLE file_context ADD COLUMN level_colors_disabled TEXT NOT NULL DEFAULT '[]'",
        )
        .execute(&mut *conn)
        .await
        .ok(); // column may already exist on fresh DBs

        // Convert old rows where level_colors = 0 (all levels disabled) to the new format.
        sqlx::query(
            "UPDATE file_context SET level_colors_disabled = '[\"trace\",\"debug\",\"info\",\"notice\",\"warning\",\"error\",\"fatal\"]' WHERE level_colors = 0 AND level_colors_disabled = '[]'",
        )
        .execute(&mut *conn)
        .await
        .ok();

        Ok(())
    }

    async fn migrate_to_v4(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query("ALTER TABLE file_context ADD COLUMN raw_mode INTEGER NOT NULL DEFAULT 0")
            .execute(&mut *conn)
            .await
            .ok(); // column may already exist on fresh DBs
        Ok(())
    }

    async fn migrate_to_v5(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            "ALTER TABLE file_context ADD COLUMN sidebar_width INTEGER NOT NULL DEFAULT 30",
        )
        .execute(&mut *conn)
        .await
        .ok(); // column may already exist on fresh DBs
        Ok(())
    }

    async fn migrate_to_v6(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    async fn migrate_to_v7(conn: &mut SqliteConnection) -> Result<()> {
        for col in &[
            "show_status_bar",
            "show_borders",
            "show_sidebar",
            "show_line_numbers",
            "wrap",
        ] {
            sqlx::query(&format!("ALTER TABLE file_context DROP COLUMN {}", col))
                .execute(&mut *conn)
                .await
                .ok();
        }
        Ok(())
    }

    async fn migrate_to_v8(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query("ALTER TABLE file_context ADD COLUMN hidden_fields TEXT NOT NULL DEFAULT '[]'")
            .execute(&mut *conn)
            .await
            .ok();
        sqlx::query("ALTER TABLE file_context ADD COLUMN field_layout_columns TEXT")
            .execute(&mut *conn)
            .await
            .ok();
        Ok(())
    }

    async fn migrate_to_v9(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            "ALTER TABLE file_context ADD COLUMN filtering_enabled INTEGER NOT NULL DEFAULT 1",
        )
        .execute(&mut *conn)
        .await
        .ok();
        Ok(())
    }

    async fn migrate_to_v10(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query("ALTER TABLE filters ADD COLUMN use_regex INTEGER NOT NULL DEFAULT 0")
            .execute(&mut *conn)
            .await
            .ok();
        Ok(())
    }

    async fn migrate_to_v11(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query("ALTER TABLE filters ADD COLUMN group_name TEXT")
            .execute(&mut *conn)
            .await
            .ok();
        Ok(())
    }

    /// SQLite can't `ALTER` a `CHECK` constraint, so allowing the new
    /// `Highlight` filter type requires rebuilding the `filters` table.
    async fn migrate_to_v12(conn: &mut SqliteConnection) -> Result<()> {
        let mut tx = conn.begin().await?;
        sqlx::query(
            "CREATE TABLE filters_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern TEXT NOT NULL,
                filter_type TEXT NOT NULL CHECK(filter_type IN ('Include', 'Exclude', 'Highlight')),
                enabled INTEGER NOT NULL DEFAULT 1,
                fg_color TEXT,
                bg_color TEXT,
                display_order INTEGER NOT NULL DEFAULT 0,
                source_file TEXT NOT NULL DEFAULT '',
                match_only INTEGER NOT NULL DEFAULT 1,
                use_regex INTEGER NOT NULL DEFAULT 0,
                group_name TEXT
            )",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO filters_new (
                id, pattern, filter_type, enabled, fg_color, bg_color,
                display_order, source_file, match_only, use_regex, group_name
            )
            SELECT
                id, pattern, filter_type, enabled, fg_color, bg_color,
                display_order, source_file, match_only, use_regex, group_name
            FROM filters",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("DROP TABLE filters").execute(&mut *tx).await?;
        sqlx::query("ALTER TABLE filters_new RENAME TO filters")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn migrate_to_v13(conn: &mut SqliteConnection) -> Result<()> {
        let mut tx = conn.begin().await?;
        sqlx::query("ALTER TABLE filters ADD COLUMN ignore_case INTEGER NOT NULL DEFAULT 0")
            .execute(&mut *tx)
            .await
            .ok();
        tx.commit().await?;
        Ok(())
    }

    /// A predefined style for a filter group, independent of the `filters`
    /// table.
    async fn migrate_to_v14(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS groups (
                name TEXT PRIMARY KEY,
                fg_color TEXT,
                bg_color TEXT,
                match_only INTEGER NOT NULL DEFAULT 1
            )",
        )
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Scopes `groups` by `source_file`, mirroring `filters` — groups were
    /// originally a flat global namespace (`name` alone as primary key),
    /// which let a group's style/membership leak across unrelated tabs.
    /// SQLite can't add a column to a primary key in place, so the table is
    /// rebuilt. Pre-existing rows land in the `''` bucket — the same "no
    /// file" bucket `filters` already uses for sourceless (e.g. stdin) tabs
    /// — rather than being deleted.
    async fn migrate_to_v15(conn: &mut SqliteConnection) -> Result<()> {
        let mut tx = conn.begin().await?;
        sqlx::query(
            "CREATE TABLE groups_new (
                source_file TEXT NOT NULL DEFAULT '',
                name TEXT NOT NULL,
                fg_color TEXT,
                bg_color TEXT,
                match_only INTEGER NOT NULL DEFAULT 1,
                PRIMARY KEY (source_file, name)
            )",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO groups_new (source_file, name, fg_color, bg_color, match_only)
             SELECT '', name, fg_color, bg_color, match_only FROM groups",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("DROP TABLE groups").execute(&mut *tx).await?;
        sqlx::query("ALTER TABLE groups_new RENAME TO groups")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Sidebar width moved from a per-file setting to a single global one
    /// (see `SettingsKey::SidebarWidth`), so the per-file column is dropped.
    async fn migrate_to_v16(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query("ALTER TABLE file_context DROP COLUMN sidebar_width")
            .execute(&mut *conn)
            .await
            .ok();
        Ok(())
    }

    pub async fn reset_all(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM filters").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM file_context")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM session_tabs")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM app_settings")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

fn filter_type_to_str(ft: &FilterType) -> &'static str {
    match ft {
        FilterType::Include => "Include",
        FilterType::Exclude => "Exclude",
        FilterType::Highlight => "Highlight",
    }
}

fn str_to_filter_type(s: &str) -> FilterType {
    match s {
        "Include" => FilterType::Include,
        "Highlight" => FilterType::Highlight,
        _ => FilterType::Exclude,
    }
}

fn row_to_group_def(row: &sqlx::sqlite::SqliteRow) -> GroupDef {
    let fg_str: Option<String> = row.get("fg_color");
    let bg_str: Option<String> = row.get("bg_color");
    let match_only = row.get::<i32, _>("match_only") != 0;
    GroupDef {
        name: row.get("name"),
        color_config: Some(ColorConfig {
            fg: fg_str.and_then(|s| s.parse().ok()),
            bg: bg_str.and_then(|s| s.parse().ok()),
            match_only,
        }),
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
        use_regex: row.try_get::<i32, _>("use_regex").unwrap_or(0) != 0,
        ignore_case: row.try_get::<i32, _>("ignore_case").unwrap_or(0) != 0,
        group: row
            .try_get::<Option<String>, _>("group_name")
            .ok()
            .flatten(),
    }
}

#[async_trait]
impl FilterStore for Database {
    async fn insert_filter(
        &self,
        pattern: &str,
        filter_type: &FilterType,
        options: FilterInsertOptions,
    ) -> Result<i64> {
        let source = options.source_file.as_deref().unwrap_or("");
        let max_order: Option<i64> = sqlx::query(
            "SELECT MAX(display_order) as max_order FROM filters WHERE source_file = ?",
        )
        .bind(source)
        .fetch_one(&self.pool)
        .await?
        .get("max_order");

        let next_order = max_order.unwrap_or(0) + 1;

        let (fg, bg, match_only) = match &options.color_config {
            Some(cc) => (
                cc.fg.map(|c| c.to_string()),
                cc.bg.map(|c| c.to_string()),
                cc.match_only,
            ),
            None => (None, None, true),
        };

        let result = sqlx::query(
            "INSERT INTO filters (pattern, filter_type, enabled, fg_color, bg_color, display_order, source_file, match_only, use_regex, ignore_case, group_name)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(pattern)
        .bind(filter_type_to_str(filter_type))
        .bind(options.enabled as i32)
        .bind(&fg)
        .bind(&bg)
        .bind(next_order)
        .bind(source)
        .bind(match_only as i32)
        .bind(options.use_regex as i32)
        .bind(options.ignore_case as i32)
        .bind(&options.group)
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

    async fn update_filter_group(&self, id: i64, group: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE filters SET group_name = ? WHERE id = ?")
            .bind(group)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_filter(
        &self,
        id: i64,
        pattern: &str,
        filter_type: &FilterType,
        color_config: Option<&ColorConfig>,
        use_regex: bool,
        ignore_case: bool,
        group: Option<&str>,
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
            "UPDATE filters SET pattern = ?, filter_type = ?, fg_color = ?, bg_color = ?, match_only = ?, use_regex = ?, ignore_case = ?, group_name = ? WHERE id = ?",
        )
        .bind(pattern)
        .bind(filter_type_to_str(filter_type))
        .bind(&fg)
        .bind(&bg)
        .bind(match_only as i32)
        .bind(use_regex as i32)
        .bind(ignore_case as i32)
        .bind(group)
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

    async fn set_all_filters_enabled(&self, enabled: bool) -> Result<()> {
        sqlx::query("UPDATE filters SET enabled = ?")
            .bind(enabled)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_filters_enabled_by_group(
        &self,
        source_file: &str,
        group: &str,
        enabled: bool,
    ) -> Result<()> {
        sqlx::query("UPDATE filters SET enabled = ? WHERE group_name = ? AND source_file = ?")
            .bind(enabled)
            .bind(group)
            .bind(source_file)
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
                "INSERT INTO filters (pattern, filter_type, enabled, fg_color, bg_color, display_order, source_file, match_only, use_regex, group_name)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&filter.pattern)
            .bind(filter_type_to_str(&filter.filter_type))
            .bind(filter.enabled as i32)
            .bind(&fg)
            .bind(&bg)
            .bind(order as i64)
            .bind(source)
            .bind(match_only as i32)
            .bind(filter.use_regex as i32)
            .bind(&filter.group)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }
}

#[async_trait]
impl GroupStore for Database {
    async fn get_groups(&self) -> Result<Vec<GroupDef>> {
        self.get_groups_for_source("").await
    }

    async fn get_groups_for_source(&self, source_file: &str) -> Result<Vec<GroupDef>> {
        let rows = sqlx::query("SELECT * FROM groups WHERE source_file = ? ORDER BY name")
            .bind(source_file)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_group_def).collect())
    }

    async fn upsert_group_style(
        &self,
        source_file: &str,
        name: &str,
        color_config: &ColorConfig,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO groups (source_file, name, fg_color, bg_color, match_only) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(source_file, name) DO UPDATE SET
                fg_color = excluded.fg_color,
                bg_color = excluded.bg_color,
                match_only = excluded.match_only",
        )
        .bind(source_file)
        .bind(name)
        .bind(color_config.fg.map(|c| c.to_string()))
        .bind(color_config.bg.map(|c| c.to_string()))
        .bind(color_config.match_only as i32)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn clear_group_style(&self, source_file: &str, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM groups WHERE source_file = ? AND name = ?")
            .bind(source_file)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn replace_all_groups(
        &self,
        groups: &[GroupDef],
        source_file: Option<&str>,
    ) -> Result<()> {
        let source = source_file.unwrap_or("");
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM groups WHERE source_file = ?")
            .bind(source)
            .execute(&mut *tx)
            .await?;
        for group in groups {
            let Some(cc) = &group.color_config else {
                continue;
            };
            sqlx::query(
                "INSERT INTO groups (source_file, name, fg_color, bg_color, match_only) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(source)
            .bind(&group.name)
            .bind(cc.fg.map(|c| c.to_string()))
            .bind(cc.bg.map(|c| c.to_string()))
            .bind(cc.match_only as i32)
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
            "INSERT INTO file_context (source_file, scroll_offset, search_query, level_colors, horizontal_scroll, marked_lines, file_hash, annotations_json, show_keys, level_colors_disabled, raw_mode, hidden_fields, field_layout_columns, filtering_enabled)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(&hidden_fields_json)
        .bind(&field_layout_columns_json)
        .bind(ctx.filtering_enabled as i32)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_file_context(&self, source_file: &str) -> Result<Option<FileContext>> {
        let row = sqlx::query(
            "SELECT source_file, scroll_offset, search_query, level_colors, horizontal_scroll, marked_lines, file_hash, annotations_json, show_keys, level_colors_disabled, raw_mode, hidden_fields, field_layout_columns, filtering_enabled
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
    async fn save_app_setting(&self, key: SettingsKey, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key.as_str())
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_app_setting(&self, key: SettingsKey) -> Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM app_settings WHERE key = ?")
            .bind(key.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("value")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RestoreSessionPolicy;

    async fn setup_db() -> Database {
        Database::in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn test_filter_crud() {
        let db = setup_db().await;

        let id1 = db
            .insert_filter("error", &FilterType::Include, FilterInsertOptions::new())
            .await
            .unwrap();
        let id2 = db
            .insert_filter("debug", &FilterType::Exclude, FilterInsertOptions::new())
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

        // Bulk disable
        db.set_all_filters_enabled(false).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert!(filters.iter().all(|f| !f.enabled));

        // Bulk enable
        db.set_all_filters_enabled(true).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert!(filters.iter().all(|f| f.enabled));

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
    async fn test_insert_and_load_highlight_filter() {
        let db = setup_db().await;
        db.insert_filter("pat", &FilterType::Highlight, FilterInsertOptions::new())
            .await
            .unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Highlight);
    }

    #[tokio::test]
    async fn test_filter_type_round_trip_all_variants() {
        let db = setup_db().await;
        for ft in [
            FilterType::Include,
            FilterType::Exclude,
            FilterType::Highlight,
        ] {
            db.insert_filter("pat", &ft, FilterInsertOptions::new())
                .await
                .unwrap();
        }
        let filters = db.get_filters().await.unwrap();
        let types: Vec<FilterType> = filters.iter().map(|f| f.filter_type.clone()).collect();
        assert_eq!(
            types,
            vec![
                FilterType::Include,
                FilterType::Exclude,
                FilterType::Highlight
            ]
        );
    }

    /// Simulates a real user upgrading: opens a database file whose schema
    /// predates the v12 `filters` table rebuild (CHECK constraint without
    /// 'Highlight', no `use_regex`/`group_name` columns applied yet as
    /// `ALTER TABLE`s from v10/v11), with existing filters already saved,
    /// and confirms the migration preserves them and unlocks Highlight.
    #[tokio::test]
    async fn test_migrate_to_v12_preserves_pre_existing_filters() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        // Seed a v11-schema database directly, bypassing the app's own
        // migration runner, then set user_version so run_migrations() only
        // needs to apply v12.
        let seed_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{path}?mode=rwc"))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE filters (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern TEXT NOT NULL,
                filter_type TEXT NOT NULL CHECK(filter_type IN ('Include', 'Exclude')),
                enabled INTEGER NOT NULL DEFAULT 1,
                fg_color TEXT,
                bg_color TEXT,
                display_order INTEGER NOT NULL DEFAULT 0,
                source_file TEXT NOT NULL DEFAULT '',
                match_only INTEGER NOT NULL DEFAULT 1,
                use_regex INTEGER NOT NULL DEFAULT 0,
                group_name TEXT
            )",
        )
        .execute(&seed_pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO filters (pattern, filter_type, enabled, fg_color, group_name)
             VALUES ('ERROR', 'Include', 1, 'Red', 'errors')",
        )
        .execute(&seed_pool)
        .await
        .unwrap();
        sqlx::query("PRAGMA user_version = 11")
            .execute(&seed_pool)
            .await
            .unwrap();
        seed_pool.close().await;

        // Now open it the same way the app does — this must run migrate_to_v12
        // and leave the pre-existing filter intact.
        let db = Database::new(&path).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].pattern, "ERROR");
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[0].group.as_deref(), Some("errors"));

        // And the CHECK constraint must now accept Highlight.
        db.insert_filter("WARN", &FilterType::Highlight, FilterInsertOptions::new())
            .await
            .unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters.len(), 2);
        assert!(
            filters
                .iter()
                .any(|f| f.filter_type == FilterType::Highlight)
        );
    }

    #[tokio::test]
    async fn test_migrate_to_v13_defaults_ignore_case_false_for_pre_existing_filters() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        // Seed a v12-schema database (no `ignore_case` column yet) directly,
        // then set user_version so run_migrations() only needs to apply v13.
        let seed_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{path}?mode=rwc"))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE filters (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern TEXT NOT NULL,
                filter_type TEXT NOT NULL CHECK(filter_type IN ('Include', 'Exclude', 'Highlight')),
                enabled INTEGER NOT NULL DEFAULT 1,
                fg_color TEXT,
                bg_color TEXT,
                display_order INTEGER NOT NULL DEFAULT 0,
                source_file TEXT NOT NULL DEFAULT '',
                match_only INTEGER NOT NULL DEFAULT 1,
                use_regex INTEGER NOT NULL DEFAULT 0,
                group_name TEXT
            )",
        )
        .execute(&seed_pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO filters (pattern, filter_type, enabled) VALUES ('ERROR', 'Include', 1)",
        )
        .execute(&seed_pool)
        .await
        .unwrap();
        sqlx::query("PRAGMA user_version = 12")
            .execute(&seed_pool)
            .await
            .unwrap();
        seed_pool.close().await;

        // Opening it the same way the app does must run migrate_to_v13 and
        // leave the pre-existing filter intact with ignore_case = false.
        let db = Database::new(&path).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].pattern, "ERROR");
        assert!(!filters[0].ignore_case);

        // A newly inserted case-insensitive filter round-trips correctly.
        db.insert_filter(
            "WARN",
            &FilterType::Include,
            FilterInsertOptions::new().ignore_case(),
        )
        .await
        .unwrap();
        let filters = db.get_filters().await.unwrap();
        let warn = filters.iter().find(|f| f.pattern == "WARN").unwrap();
        assert!(warn.ignore_case);
    }

    #[tokio::test]
    async fn test_migrate_to_v14_creates_groups_table() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        // Seed a v13-schema database (no `groups` table yet), then set
        // user_version so run_migrations() only needs to apply v14.
        let seed_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{path}?mode=rwc"))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE filters (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern TEXT NOT NULL,
                filter_type TEXT NOT NULL CHECK(filter_type IN ('Include', 'Exclude', 'Highlight')),
                enabled INTEGER NOT NULL DEFAULT 1,
                fg_color TEXT,
                bg_color TEXT,
                display_order INTEGER NOT NULL DEFAULT 0,
                source_file TEXT NOT NULL DEFAULT '',
                match_only INTEGER NOT NULL DEFAULT 1,
                use_regex INTEGER NOT NULL DEFAULT 0,
                group_name TEXT,
                ignore_case INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&seed_pool)
        .await
        .unwrap();
        sqlx::query("PRAGMA user_version = 13")
            .execute(&seed_pool)
            .await
            .unwrap();
        seed_pool.close().await;

        let db = Database::new(&path).await.unwrap();
        assert!(db.get_groups().await.unwrap().is_empty());
        db.upsert_group_style(
            "",
            "errors",
            &ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: None,
                match_only: true,
            },
        )
        .await
        .unwrap();
        let groups = db.get_groups().await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "errors");
    }

    #[tokio::test]
    async fn test_migrate_to_v15_scopes_groups_by_source_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        // Seed a v14-schema database (pre-scoping `groups` table, `name` as
        // the bare primary key), then set user_version so run_migrations()
        // only needs to apply v15.
        let seed_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{path}?mode=rwc"))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE groups (
                name TEXT PRIMARY KEY,
                fg_color TEXT,
                bg_color TEXT,
                match_only INTEGER NOT NULL DEFAULT 1
            )",
        )
        .execute(&seed_pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO groups (name, fg_color, bg_color, match_only) VALUES ('errors', 'Red', NULL, 1)",
        )
        .execute(&seed_pool)
        .await
        .unwrap();
        sqlx::query("PRAGMA user_version = 14")
            .execute(&seed_pool)
            .await
            .unwrap();
        seed_pool.close().await;

        // Opening it the same way the app does must run migrate_to_v15 and
        // land the pre-existing group in the '' (no source file) bucket —
        // the same bucket filters already use for sourceless tabs — rather
        // than deleting it.
        let db = Database::new(&path).await.unwrap();
        let groups = db.get_groups().await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "errors");
        assert!(
            db.get_groups_for_source("some_file.log")
                .await
                .unwrap()
                .is_empty(),
            "the migrated group must not leak into an unrelated source"
        );

        // A newly inserted group scoped to a real file round-trips
        // independently of the migrated '' bucket group.
        db.upsert_group_style(
            "some_file.log",
            "errors",
            &ColorConfig {
                fg: Some(ratatui::style::Color::Green),
                bg: None,
                match_only: true,
            },
        )
        .await
        .unwrap();
        let scoped = db.get_groups_for_source("some_file.log").await.unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(
            scoped[0].color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Green)
        );
        // The '' bucket group is untouched by the source-scoped insert.
        let global = db.get_groups().await.unwrap();
        assert_eq!(
            global[0].color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Red)
        );
    }

    #[tokio::test]
    async fn test_get_groups_empty_by_default() {
        let db = setup_db().await;
        assert!(db.get_groups().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_insert_and_get_group_style() {
        let db = setup_db().await;
        let cc = ColorConfig {
            fg: Some(ratatui::style::Color::Red),
            bg: Some(ratatui::style::Color::Blue),
            match_only: false,
        };
        db.upsert_group_style("", "errors", &cc).await.unwrap();
        let groups = db.get_groups().await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "errors");
        let stored = groups[0].color_config.as_ref().unwrap();
        assert_eq!(stored.fg, cc.fg);
        assert_eq!(stored.bg, cc.bg);
        assert_eq!(stored.match_only, cc.match_only);
    }

    #[tokio::test]
    async fn test_upsert_group_style_updates_existing() {
        let db = setup_db().await;
        db.upsert_group_style(
            "",
            "errors",
            &ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: None,
                match_only: true,
            },
        )
        .await
        .unwrap();
        db.upsert_group_style(
            "",
            "errors",
            &ColorConfig {
                fg: Some(ratatui::style::Color::Green),
                bg: None,
                match_only: true,
            },
        )
        .await
        .unwrap();
        let groups = db.get_groups().await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Green)
        );
    }

    #[tokio::test]
    async fn test_clear_group_style_removes_row() {
        let db = setup_db().await;
        db.upsert_group_style(
            "",
            "errors",
            &ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: None,
                match_only: true,
            },
        )
        .await
        .unwrap();
        db.clear_group_style("", "errors").await.unwrap();
        assert!(db.get_groups().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_upsert_group_style_same_name_different_sources_do_not_collide() {
        let db = setup_db().await;
        db.upsert_group_style(
            "a.log",
            "errors",
            &ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: None,
                match_only: true,
            },
        )
        .await
        .unwrap();
        db.upsert_group_style(
            "b.log",
            "errors",
            &ColorConfig {
                fg: Some(ratatui::style::Color::Blue),
                bg: None,
                match_only: true,
            },
        )
        .await
        .unwrap();

        let a_groups = db.get_groups_for_source("a.log").await.unwrap();
        let b_groups = db.get_groups_for_source("b.log").await.unwrap();
        assert_eq!(a_groups.len(), 1);
        assert_eq!(b_groups.len(), 1);
        assert_eq!(
            a_groups[0].color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Red)
        );
        assert_eq!(
            b_groups[0].color_config.as_ref().unwrap().fg,
            Some(ratatui::style::Color::Blue)
        );
    }

    #[tokio::test]
    async fn test_clear_group_style_only_affects_its_source() {
        let db = setup_db().await;
        let cc = ColorConfig {
            fg: Some(ratatui::style::Color::Red),
            bg: None,
            match_only: true,
        };
        db.upsert_group_style("a.log", "errors", &cc).await.unwrap();
        db.upsert_group_style("b.log", "errors", &cc).await.unwrap();
        db.clear_group_style("a.log", "errors").await.unwrap();
        assert!(db.get_groups_for_source("a.log").await.unwrap().is_empty());
        assert_eq!(db.get_groups_for_source("b.log").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_replace_all_groups_only_touches_its_source() {
        let db = setup_db().await;
        let cc = ColorConfig {
            fg: Some(ratatui::style::Color::Red),
            bg: None,
            match_only: true,
        };
        db.upsert_group_style("a.log", "errors", &cc).await.unwrap();
        db.upsert_group_style("b.log", "kept", &cc).await.unwrap();

        db.replace_all_groups(
            &[GroupDef {
                name: "replaced".to_string(),
                color_config: Some(cc.clone()),
            }],
            Some("a.log"),
        )
        .await
        .unwrap();

        let a_groups = db.get_groups_for_source("a.log").await.unwrap();
        assert_eq!(a_groups.len(), 1);
        assert_eq!(a_groups[0].name, "replaced");
        // b.log's groups are untouched by a replace scoped to a.log.
        let b_groups = db.get_groups_for_source("b.log").await.unwrap();
        assert_eq!(b_groups.len(), 1);
        assert_eq!(b_groups[0].name, "kept");
    }

    #[tokio::test]
    async fn test_set_filters_enabled_by_group_only_affects_its_source() {
        let db = setup_db().await;
        db.insert_filter(
            "error",
            &FilterType::Include,
            FilterInsertOptions::new().source("a.log").group("errors"),
        )
        .await
        .unwrap();
        db.insert_filter(
            "error",
            &FilterType::Include,
            FilterInsertOptions::new().source("b.log").group("errors"),
        )
        .await
        .unwrap();

        db.set_filters_enabled_by_group("a.log", "errors", false)
            .await
            .unwrap();

        let a_filters = db.get_filters_for_source("a.log").await.unwrap();
        let b_filters = db.get_filters_for_source("b.log").await.unwrap();
        assert!(!a_filters[0].enabled, "a.log's filter should be disabled");
        assert!(b_filters[0].enabled, "b.log's filter must be unaffected");
    }

    #[tokio::test]
    async fn test_filter_with_color() {
        let db = setup_db().await;
        let color = ColorConfig {
            fg: Some(ratatui::style::Color::Red),
            bg: Some(ratatui::style::Color::Blue),
            match_only: false,
        };

        db.insert_filter(
            "error",
            &FilterType::Include,
            FilterInsertOptions::new().color(color),
        )
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
            .insert_filter("error", &FilterType::Include, FilterInsertOptions::new())
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
    async fn test_insert_filter_persists_group() {
        let db = setup_db().await;
        db.insert_filter(
            "error",
            &FilterType::Include,
            FilterInsertOptions::new().group("errors"),
        )
        .await
        .unwrap();
        db.insert_filter("debug", &FilterType::Include, FilterInsertOptions::new())
            .await
            .unwrap();

        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters[0].group.as_deref(), Some("errors"));
        assert_eq!(filters[1].group, None);
    }

    #[tokio::test]
    async fn test_update_filter_group() {
        let db = setup_db().await;
        let id = db
            .insert_filter("error", &FilterType::Include, FilterInsertOptions::new())
            .await
            .unwrap();

        db.update_filter_group(id, Some("errors")).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters[0].group.as_deref(), Some("errors"));

        db.update_filter_group(id, None).await.unwrap();
        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters[0].group, None);
    }

    #[tokio::test]
    async fn test_update_filter_updates_group() {
        let db = setup_db().await;
        let id = db
            .insert_filter(
                "error",
                &FilterType::Include,
                FilterInsertOptions::new().group("old-group"),
            )
            .await
            .unwrap();

        db.update_filter(
            id,
            "error",
            &FilterType::Include,
            None,
            false,
            false,
            Some("new-group"),
        )
        .await
        .unwrap();

        let filters = db.get_filters().await.unwrap();
        assert_eq!(filters[0].group.as_deref(), Some("new-group"));
    }

    #[tokio::test]
    async fn test_set_filters_enabled_by_group() {
        let db = setup_db().await;
        db.insert_filter(
            "error",
            &FilterType::Include,
            FilterInsertOptions::new().group("errors"),
        )
        .await
        .unwrap();
        db.insert_filter(
            "warn",
            &FilterType::Include,
            FilterInsertOptions::new().group("errors"),
        )
        .await
        .unwrap();
        db.insert_filter(
            "debug",
            &FilterType::Include,
            FilterInsertOptions::new().group("other"),
        )
        .await
        .unwrap();

        db.set_filters_enabled_by_group("", "errors", false)
            .await
            .unwrap();

        let filters = db.get_filters().await.unwrap();
        let by_pattern = |p: &str| filters.iter().find(|f| f.pattern == p).unwrap();
        assert!(!by_pattern("error").enabled);
        assert!(!by_pattern("warn").enabled);
        assert!(by_pattern("debug").enabled);
    }

    #[tokio::test]
    async fn test_replace_all_filters_persists_group() {
        let db = setup_db().await;
        let filters = vec![FilterDef {
            id: 0,
            pattern: "error".to_string(),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: Some("errors".to_string()),
        }];
        db.replace_all_filters(&filters, None).await.unwrap();

        let loaded = db.get_filters().await.unwrap();
        assert_eq!(loaded[0].group.as_deref(), Some("errors"));
    }

    #[tokio::test]
    async fn test_swap_filter_order() {
        let db = setup_db().await;
        let id1 = db
            .insert_filter("first", &FilterType::Include, FilterInsertOptions::new())
            .await
            .unwrap();
        let id2 = db
            .insert_filter("second", &FilterType::Exclude, FilterInsertOptions::new())
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
        db.insert_filter("old1", &FilterType::Include, FilterInsertOptions::new())
            .await
            .unwrap();
        db.insert_filter("old2", &FilterType::Exclude, FilterInsertOptions::new())
            .await
            .unwrap();

        let new_filters = vec![
            FilterDef {
                id: 0,
                pattern: "new1".to_string(),
                filter_type: FilterType::Include,
                enabled: true,
                color_config: None,
                use_regex: false,
                ignore_case: false,
                group: None,
            },
            FilterDef {
                id: 0,
                pattern: "new2".to_string(),
                filter_type: FilterType::Exclude,
                enabled: false,
                color_config: None,
                use_regex: false,
                ignore_case: false,
                group: None,
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
        db.insert_filter("global", &FilterType::Include, FilterInsertOptions::new())
            .await
            .unwrap();
        db.insert_filter(
            "file-specific",
            &FilterType::Include,
            FilterInsertOptions::new().source("test.log"),
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
        db.insert_filter("global", &FilterType::Include, FilterInsertOptions::new())
            .await
            .unwrap();
        db.insert_filter(
            "file1",
            &FilterType::Exclude,
            FilterInsertOptions::new().source("/var/log/syslog"),
        )
        .await
        .unwrap();
        db.insert_filter(
            "file2",
            &FilterType::Include,
            FilterInsertOptions::new().source("/var/log/syslog"),
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
        use crate::db::Comment;
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
    async fn test_sidebar_width_setting_round_trips() {
        let db = setup_db().await;
        db.save_app_setting(SettingsKey::SidebarWidth, "45")
            .await
            .unwrap();

        let loaded = db
            .load_app_setting(SettingsKey::SidebarWidth)
            .await
            .unwrap();

        assert_eq!(loaded, Some("45".to_string()));
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

    #[tokio::test]
    async fn test_app_setting_load_returns_none_when_not_set() {
        let db = setup_db().await;
        let result = db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_app_setting_save_and_load() {
        let db = setup_db().await;
        db.save_app_setting(
            SettingsKey::RestoreSession,
            &RestoreSessionPolicy::Always.to_string(),
        )
        .await
        .unwrap();
        let value = db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        assert_eq!(
            value.as_deref(),
            Some(RestoreSessionPolicy::Always.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn test_app_setting_save_overwrites() {
        let db = setup_db().await;
        db.save_app_setting(
            SettingsKey::RestoreSession,
            &RestoreSessionPolicy::Always.to_string(),
        )
        .await
        .unwrap();
        db.save_app_setting(
            SettingsKey::RestoreSession,
            &RestoreSessionPolicy::Never.to_string(),
        )
        .await
        .unwrap();
        let value = db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        assert_eq!(
            value.as_deref(),
            Some(RestoreSessionPolicy::Never.to_string().as_str())
        );
    }

    #[test]
    fn test_default_filter_files_as_str() {
        assert_eq!(
            SettingsKey::DefaultFilterFiles.as_str(),
            "default_filter_files"
        );
    }

    #[tokio::test]
    async fn test_default_filter_files_save_and_load() {
        let db = setup_db().await;
        db.save_app_setting(SettingsKey::DefaultFilterFiles, r#"{"acme":"a.json"}"#)
            .await
            .unwrap();
        let value = db
            .load_app_setting(SettingsKey::DefaultFilterFiles)
            .await
            .unwrap();
        assert_eq!(value.as_deref(), Some(r#"{"acme":"a.json"}"#));
    }

    #[tokio::test]
    async fn test_default_filter_files_load_returns_none_when_unset() {
        let db = setup_db().await;
        let value = db
            .load_app_setting(SettingsKey::DefaultFilterFiles)
            .await
            .unwrap();
        assert!(value.is_none());
    }

    #[tokio::test]
    async fn test_app_setting_file_policy_independent_of_session_policy() {
        let db = setup_db().await;
        db.save_app_setting(
            SettingsKey::RestoreFileContext,
            &RestoreSessionPolicy::Never.to_string(),
        )
        .await
        .unwrap();
        let session = db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        let file = db
            .load_app_setting(SettingsKey::RestoreFileContext)
            .await
            .unwrap();
        assert!(session.is_none());
        assert_eq!(
            file.as_deref(),
            Some(RestoreSessionPolicy::Never.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn test_app_setting_session_policy_independent_of_file_policy() {
        let db = setup_db().await;
        db.save_app_setting(
            SettingsKey::RestoreSession,
            &RestoreSessionPolicy::Always.to_string(),
        )
        .await
        .unwrap();
        let session = db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        let file = db
            .load_app_setting(SettingsKey::RestoreFileContext)
            .await
            .unwrap();
        assert_eq!(
            session.as_deref(),
            Some(RestoreSessionPolicy::Always.to_string().as_str())
        );
        assert!(file.is_none());
    }

    #[tokio::test]
    async fn test_reset_all_clears_all_tables() {
        let db = setup_db().await;

        db.insert_filter(
            "error",
            &FilterType::Include,
            FilterInsertOptions::new().source("app.log"),
        )
        .await
        .unwrap();
        db.insert_filter("debug", &FilterType::Exclude, FilterInsertOptions::new())
            .await
            .unwrap();

        db.save_file_context(&FileContext {
            source_file: "app.log".into(),
            scroll_offset: 42,
            search_query: String::new(),
            marked_lines: vec![1, 2, 3],
            file_hash: None,
            comments: Vec::new(),
            show_keys: false,
            raw_mode: false,
            level_colors_disabled: HashSet::new(),
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            horizontal_scroll: 0,
            filtering_enabled: true,
        })
        .await
        .unwrap();

        db.save_session(&["app.log".into(), "server.log".into()])
            .await
            .unwrap();

        db.save_app_setting(
            SettingsKey::RestoreSession,
            &RestoreSessionPolicy::Always.to_string(),
        )
        .await
        .unwrap();

        db.reset_all().await.unwrap();

        assert!(db.get_filters().await.unwrap().is_empty());
        assert!(db.load_file_context("app.log").await.unwrap().is_none());
        assert!(db.load_session().await.unwrap().is_empty());
        assert!(
            db.load_app_setting(SettingsKey::RestoreSession)
                .await
                .unwrap()
                .is_none()
        );
    }
}
