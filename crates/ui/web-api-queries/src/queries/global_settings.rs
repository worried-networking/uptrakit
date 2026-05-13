//! Audit view for the `global_setting` entity.
//!
//! `GlobalSettingView` is implemented manually (not via `#[derive(AuditView)]`)
//! because the `global_setting` table uses a `key: String` primary key rather
//! than a `Uuid` `id` field.

use uptrakit_audit_log::AuditView;

/// Audit snapshot for a global setting entry.
///
/// The `value` field is included in the audit snapshot intentionally so that
/// diff-aware tooling can compare before and after states.  Sensitive values
/// (e.g. encrypted NATS URLs) are stored in their encrypted form and are never
/// decrypted before snapshotting.
pub struct GlobalSettingView {
    /// The setting key (e.g. `"nats.url"`, `"zeroconf.enabled"`).
    pub key: String,
    /// The raw JSON value stored in the database (may be an encrypted string).
    pub value: serde_json::Value,
}

impl AuditView for GlobalSettingView {
    const TARGET_TYPE: &'static str = "global_setting";

    fn audit_target_id(&self) -> String {
        self.key.clone()
    }

    fn audit_target_display(&self) -> Option<String> {
        Some(self.key.clone())
    }

    fn audit_view(&self) -> serde_json::Value {
        serde_json::json!({
            "key": self.key,
            "value": self.value,
        })
    }
}

impl From<&uptrakit_shared_db::entity::global_setting::Model> for GlobalSettingView {
    fn from(m: &uptrakit_shared_db::entity::global_setting::Model) -> Self {
        Self {
            key: m.key.clone(),
            value: m.value.clone(),
        }
    }
}
