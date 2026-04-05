use std::sync::Arc;

use crate::config::RestoreSessionPolicy;
use crate::db::{AppSettingsStore, FileContextStore, SessionStore, SettingsKey};

use crate::ui::TabState;

pub struct SessionManager {
    pub db: Arc<crate::db::Database>,
    pub restore_policy: RestoreSessionPolicy,
    pub restore_file_policy: RestoreSessionPolicy,
    pub pending_session_restore: Option<Vec<String>>,
    pub startup_tail: bool,
    pub startup_filters: bool,
    pub startup_warnings: Vec<String>,
}

impl SessionManager {
    pub async fn save_tab_context(&self, tab: &TabState) {
        if let Some(ctx) = tab.to_file_context() {
            let _ = self.db.save_file_context(&ctx).await;
        }
    }

    pub async fn save_all_contexts(&self, tabs: &[TabState]) {
        let tmp_dir = std::env::temp_dir();
        let source_files: Vec<String> = tabs
            .iter()
            .filter_map(|t| t.log_manager.source_file().map(|s| s.to_string()))
            .filter(|p| !std::path::Path::new(p).starts_with(&tmp_dir))
            .filter(|p| crate::ingestion::detect_archive_type(p).is_none())
            .collect();

        let contexts: Vec<crate::db::FileContext> =
            tabs.iter().filter_map(|t| t.to_file_context()).collect();

        if !source_files.is_empty() {
            let _ = self.db.save_session(&source_files).await;
        }
        for ctx in &contexts {
            let _ = self.db.save_file_context(ctx).await;
        }
    }

    pub async fn save_app_bool(&self, key: SettingsKey, value: bool) {
        let _ = self
            .db
            .save_app_setting(key, if value { "true" } else { "false" })
            .await;
    }

    pub async fn set_restore_file_policy(&mut self, policy: RestoreSessionPolicy) {
        self.restore_file_policy = policy;
        let _ = self
            .db
            .save_app_setting(SettingsKey::RestoreFileContext, &policy.to_string())
            .await;
    }

    pub async fn set_restore_policy(&mut self, policy: RestoreSessionPolicy) {
        self.restore_policy = policy;
        let _ = self
            .db
            .save_app_setting(SettingsKey::RestoreSession, &policy.to_string())
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager, SessionStore};
    use crate::ingestion::FileReader;

    async fn make_session(db: Arc<Database>) -> SessionManager {
        SessionManager {
            db,
            restore_policy: RestoreSessionPolicy::Ask,
            restore_file_policy: RestoreSessionPolicy::Ask,
            pending_session_restore: None,
            startup_tail: false,
            startup_filters: false,
            startup_warnings: Vec::new(),
        }
    }

    async fn make_tab(db: Arc<Database>, source: Option<&str>) -> TabState {
        let lm = LogManager::new(db, source.map(|s| s.to_string())).await;
        TabState::new(
            FileReader::from_bytes(b"hello\nworld\n".to_vec()),
            lm,
            "test".to_string(),
        )
    }

    #[tokio::test]
    async fn test_save_tab_context_no_source() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let session = make_session(db.clone()).await;
        let tab = make_tab(db, None).await;
        session.save_tab_context(&tab).await;
    }

    #[tokio::test]
    async fn test_save_tab_context_with_source() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let session = make_session(db.clone()).await;
        let tab = make_tab(db.clone(), Some("test.log")).await;
        session.save_tab_context(&tab).await;
        let ctx = db.load_file_context("test.log").await.unwrap();
        assert!(ctx.is_some());
    }

    #[tokio::test]
    async fn test_save_all_contexts_no_source_files() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let session = make_session(db.clone()).await;
        let tab = make_tab(db, None).await;
        session.save_all_contexts(&[tab]).await;
    }

    #[tokio::test]
    async fn test_save_all_contexts_with_source_files() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let session = make_session(db.clone()).await;
        let tab = make_tab(db.clone(), Some("test.log")).await;
        session.save_all_contexts(&[tab]).await;
        let files = db.load_session().await.unwrap();
        assert!(!files.is_empty());
    }
}
