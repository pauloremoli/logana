use std::sync::Arc;

use crate::config::RestoreSessionPolicy;
use crate::db::{AppSettingsStore, FileContextStore, SessionStore, SettingsKey};

use super::TabState;

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
