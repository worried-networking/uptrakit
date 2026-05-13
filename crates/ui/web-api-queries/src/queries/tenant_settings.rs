//! Audit view for the `tenant_setting` entity.
//!
//! `TenantSettingView` is implemented manually (not via `#[derive(AuditView)]`)
//! because the `setting` table uses a composite `(tenant_id, key)` primary key
//! rather than a `Uuid` `id` field.

use uptrakit_audit_log::AuditView;

/// Audit snapshot for a per-tenant setting entry.
///
/// The `value` field is included in the audit snapshot intentionally so that
/// diff-aware tooling can compare before and after states.  Sensitive values
/// (e.g. hashed tokens) are stored in their stored form and are never decrypted
/// before snapshotting.
pub struct TenantSettingView {
    /// The setting key (e.g. `"registration.mode"`, `"authentication.password_auth_enabled"`).
    pub key: String,
    /// The raw JSON value stored in the database (may be a hashed or encrypted string).
    pub value: serde_json::Value,
}

impl AuditView for TenantSettingView {
    const TARGET_TYPE: &'static str = "tenant_setting";

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
