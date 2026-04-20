//! Database query helpers for the discovery plugin allowlist feature.
//!
//! Two tables are managed here:
//! - `tenant_discovery_allowlist`: tenant-wide allowed plugin types.
//! - `host_discovery_allowlist`: per-host overrides.
//!
//! The effective allowlist for a given host during discovery is determined as:
//!
//! 1. If host-specific entries exist → use those exclusively (host overrides tenant).
//! 2. Else if tenant-wide entries exist → use those.
//! 3. Else → no restriction (all discovery plugin types run).

use std::collections::HashSet;

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::{PluginCapability, PluginMetadataOps};
use uptrakit_shared_db::entity::{host_discovery_allowlist, tenant_discovery_allowlist};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_shared_types::PluginTypeId;
use uptrakit_web_api_types::discovery_allowlist::{
    HostDiscoveryAllowlistEntry, TenantDiscoveryAllowlistEntry,
};
use uuid::Uuid;

// ── Error type ────────────────────────────────────────────────────────────────

/// Error returned by discovery allowlist query helpers.
#[derive(Debug, thiserror::Error)]
pub enum AllowlistError {
    /// The provided plugin type does not support the `DiscoverLocalSoftware` capability,
    /// or is an `Other`/unknown plugin type.
    #[error("plugin type does not support discovery")]
    InvalidPluginType,
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<AllowlistError>>;
impl_report_conversion!(sea_orm::DbErr => AllowlistError::Db);

impl AllowlistError {
    /// Returns the audit classification `(outcome, reason_code)` for a tenant-wide
    /// allowlist create failure.
    pub fn tenant_create_audit_classification(
        &self,
    ) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::InvalidPluginType => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "invalid_plugin_type",
            ),
            Self::Db(_) => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "tenant_discovery_allowlist_create_failed",
            ),
        }
    }

    /// Returns the audit classification `(outcome, reason_code)` for a host-level
    /// allowlist create failure.
    pub fn host_create_audit_classification(
        &self,
    ) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::InvalidPluginType => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "invalid_plugin_type",
            ),
            Self::Db(_) => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "host_discovery_allowlist_create_failed",
            ),
        }
    }
}

// ── Internal validation ───────────────────────────────────────────────────────

/// Returns `true` if `plugin_type` is a known type with `DiscoverLocalSoftware`.
fn is_valid_discovery_plugin(ops: &dyn PluginMetadataOps, plugin_type: &PluginTypeId) -> bool {
    ops.capabilities(plugin_type)
        .contains(&PluginCapability::DiscoverLocalSoftware)
}

/// Returns `true` if the database error represents a unique constraint violation.
fn is_unique_violation(e: &sea_orm::DbErr) -> bool {
    matches!(
        e.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    )
}

// ── Tenant-wide allowlist ─────────────────────────────────────────────────────

/// List all tenant-wide discovery allowlist entries.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn list_tenant_allowlist(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<TenantDiscoveryAllowlistEntry>> {
    let rows = tenant_discovery_allowlist::Entity::find()
        .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .context_to()?;

    Ok(rows
        .into_iter()
        .map(|r| TenantDiscoveryAllowlistEntry {
            id: r.id,
            plugin_type: r.plugin_type,
            created_at: r.created_at,
        })
        .collect())
}

/// Add a plugin type to the tenant-wide discovery allowlist.
///
/// Idempotent: if the entry already exists (including via a concurrent insert),
/// returns the existing entry. Unique constraint violations from concurrent
/// inserts are handled gracefully via a follow-up SELECT.
/// Rejects `Other`/unknown plugin types and types without `DiscoverLocalSoftware`.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn add_tenant_allowlist_entry(
    ops: &dyn PluginMetadataOps,
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_type: PluginTypeId,
) -> Result<TenantDiscoveryAllowlistEntry> {
    if !is_valid_discovery_plugin(ops, &plugin_type) {
        bail!(AllowlistError::InvalidPluginType);
    }

    let type_str = plugin_type.to_string();

    // Fast path: check for an existing entry before attempting the insert.
    if let Some(existing) = tenant_discovery_allowlist::Entity::find()
        .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .filter(tenant_discovery_allowlist::Column::PluginType.eq(&type_str))
        .one(db)
        .await
        .context_to()?
    {
        return Ok(TenantDiscoveryAllowlistEntry {
            id: existing.id,
            plugin_type: existing.plugin_type,
            created_at: existing.created_at,
        });
    }

    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();

    let model = tenant_discovery_allowlist::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        plugin_type: Set(type_str.clone()),
        created_at: Set(now),
    };

    match model.insert(db).await {
        Ok(_) => Ok(TenantDiscoveryAllowlistEntry {
            id,
            plugin_type: type_str,
            created_at: now,
        }),
        Err(e) if is_unique_violation(&e) => {
            // Lost a concurrent race — the DB constraint prevented a duplicate.
            // Fetch and return the winner's entry.
            tenant_discovery_allowlist::Entity::find()
                .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_id))
                .filter(tenant_discovery_allowlist::Column::PluginType.eq(&type_str))
                .one(db)
                .await
                .context_to()?
                .map(|existing| TenantDiscoveryAllowlistEntry {
                    id: existing.id,
                    plugin_type: existing.plugin_type,
                    created_at: existing.created_at,
                })
                .ok_or_else(|| {
                    report!(AllowlistError::Db(sea_orm::DbErr::Custom(
                        "concurrent insert race: entry vanished after unique violation".to_string()
                    )))
                })
        }
        Err(e) => Err(e).context_to(),
    }
}

/// Remove a tenant-wide discovery allowlist entry by ID.
///
/// Returns `true` if deleted, `false` if not found.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn remove_tenant_allowlist_entry(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<bool> {
    let result = tenant_discovery_allowlist::Entity::delete_many()
        .filter(tenant_discovery_allowlist::Column::Id.eq(id))
        .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .exec(db)
        .await
        .context_to()?;

    Ok(result.rows_affected > 0)
}

/// Load the tenant-wide allowlist as a `HashSet<String>` for efficient lookup.
///
/// Returns an empty set when no entries exist (unconfigured → all allowed).
/// On database failure, logs a warning and falls back to an empty set so that
/// discovery proceeds unfiltered rather than silently failing.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn load_tenant_allowlist_set(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> HashSet<String> {
    match tenant_discovery_allowlist::Entity::find()
        .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
    {
        Ok(rows) => rows.into_iter().map(|r| r.plugin_type).collect(),
        Err(e) => {
            tracing::warn!(error = %e, %tenant_id, "failed to load tenant discovery allowlist; falling back to empty set (all plugins allowed)");
            HashSet::new()
        }
    }
}

// ── Host-specific allowlist ───────────────────────────────────────────────────

/// List all host-specific discovery allowlist entries.
///
/// `tenant_id` is used for access scoping — only entries belonging to that tenant
/// are returned.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn list_host_allowlist(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
) -> Result<Vec<HostDiscoveryAllowlistEntry>> {
    let rows = host_discovery_allowlist::Entity::find()
        .filter(host_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .filter(host_discovery_allowlist::Column::HostId.eq(host_id))
        .all(db)
        .await
        .context_to()?;

    Ok(rows
        .into_iter()
        .map(|r| HostDiscoveryAllowlistEntry {
            id: r.id,
            host_id: r.host_id,
            plugin_type: r.plugin_type,
            created_at: r.created_at,
        })
        .collect())
}

/// Add a plugin type to a host's discovery allowlist.
///
/// Idempotent: if the entry already exists (including via a concurrent insert),
/// returns the existing entry. Unique constraint violations from concurrent
/// inserts are handled gracefully via a follow-up SELECT.
/// Rejects `Other`/unknown plugin types and types without `DiscoverLocalSoftware`.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn add_host_allowlist_entry(
    ops: &dyn PluginMetadataOps,
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    plugin_type: PluginTypeId,
) -> Result<HostDiscoveryAllowlistEntry> {
    if !is_valid_discovery_plugin(ops, &plugin_type) {
        bail!(AllowlistError::InvalidPluginType);
    }

    let type_str = plugin_type.to_string();

    // Fast path: check for an existing entry before attempting the insert.
    if let Some(existing) = host_discovery_allowlist::Entity::find()
        .filter(host_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .filter(host_discovery_allowlist::Column::HostId.eq(host_id))
        .filter(host_discovery_allowlist::Column::PluginType.eq(&type_str))
        .one(db)
        .await
        .context_to()?
    {
        return Ok(HostDiscoveryAllowlistEntry {
            id: existing.id,
            host_id: existing.host_id,
            plugin_type: existing.plugin_type,
            created_at: existing.created_at,
        });
    }

    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();

    let model = host_discovery_allowlist::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        plugin_type: Set(type_str.clone()),
        created_at: Set(now),
    };

    match model.insert(db).await {
        Ok(_) => Ok(HostDiscoveryAllowlistEntry {
            id,
            host_id,
            plugin_type: type_str,
            created_at: now,
        }),
        Err(e) if is_unique_violation(&e) => {
            // Lost a concurrent race — the DB constraint prevented a duplicate.
            // Fetch and return the winner's entry.
            host_discovery_allowlist::Entity::find()
                .filter(host_discovery_allowlist::Column::TenantId.eq(tenant_id))
                .filter(host_discovery_allowlist::Column::HostId.eq(host_id))
                .filter(host_discovery_allowlist::Column::PluginType.eq(&type_str))
                .one(db)
                .await
                .context_to()?
                .map(|existing| HostDiscoveryAllowlistEntry {
                    id: existing.id,
                    host_id: existing.host_id,
                    plugin_type: existing.plugin_type,
                    created_at: existing.created_at,
                })
                .ok_or_else(|| {
                    report!(AllowlistError::Db(sea_orm::DbErr::Custom(
                        "concurrent insert race: entry vanished after unique violation".to_string()
                    )))
                })
        }
        Err(e) => Err(e).context_to(),
    }
}

/// Remove a host-specific discovery allowlist entry by ID.
///
/// `tenant_id` and `host_id` are used for access scoping.
/// Returns `true` if deleted, `false` if not found.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn remove_host_allowlist_entry(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    entry_id: Uuid,
) -> Result<bool> {
    let result = host_discovery_allowlist::Entity::delete_many()
        .filter(host_discovery_allowlist::Column::Id.eq(entry_id))
        .filter(host_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .filter(host_discovery_allowlist::Column::HostId.eq(host_id))
        .exec(db)
        .await
        .context_to()?;

    Ok(result.rows_affected > 0)
}

/// Load the host-specific allowlist as a `HashSet<String>` for efficient lookup.
///
/// Returns an empty set when no entries exist.
/// On database failure, logs a warning and falls back to an empty set so that
/// discovery proceeds unfiltered rather than silently failing.
#[tracing::instrument(skip_all, fields(%host_id))]
pub async fn load_host_allowlist_set(db: &DatabaseConnection, host_id: Uuid) -> HashSet<String> {
    match host_discovery_allowlist::Entity::find()
        .filter(host_discovery_allowlist::Column::HostId.eq(host_id))
        .all(db)
        .await
    {
        Ok(rows) => rows.into_iter().map(|r| r.plugin_type).collect(),
        Err(e) => {
            tracing::warn!(error = %e, %host_id, "failed to load host discovery allowlist; falling back to empty set (tenant allowlist applies)");
            HashSet::new()
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use uptrakit_plugin_infrastructure_registry::PluginDescriptor;
    use uptrakit_shared_types::plugin_ids;

    use super::*;

    // ── Mock PluginMetadataOps ────────────────────────────────────────────────

    struct MockOps;

    impl MockOps {
        fn new_with_homebrew_apt() -> Self {
            Self
        }
    }

    impl PluginMetadataOps for MockOps {
        fn get(&self, _id: &PluginTypeId) -> Option<&PluginDescriptor> {
            None
        }

        fn all(&self) -> Vec<&PluginDescriptor> {
            vec![]
        }

        fn capabilities(&self, id: &PluginTypeId) -> Vec<PluginCapability> {
            let discovery = [
                "package_manager_homebrew",
                "package_manager_apt",
                "discovery_proxmox_helper_scripts",
            ];
            if discovery.contains(&id.as_ref()) {
                vec![PluginCapability::DiscoverLocalSoftware]
            } else {
                vec![]
            }
        }
    }

    // ── is_valid_discovery_plugin ─────────────────────────────────────────────

    #[test]
    fn valid_discovery_plugin_homebrew() {
        let ops = MockOps::new_with_homebrew_apt();
        assert!(is_valid_discovery_plugin(
            &ops,
            &plugin_ids::PACKAGE_MANAGER_HOMEBREW
        ));
    }

    #[test]
    fn valid_discovery_plugin_apt() {
        let ops = MockOps::new_with_homebrew_apt();
        assert!(is_valid_discovery_plugin(
            &ops,
            &plugin_ids::PACKAGE_MANAGER_APT
        ));
    }

    #[test]
    fn invalid_discovery_plugin_github() {
        let ops = MockOps::new_with_homebrew_apt();
        // ReleasesGithub does not have DiscoverLocalSoftware
        assert!(!is_valid_discovery_plugin(
            &ops,
            &plugin_ids::RELEASES_GITHUB
        ));
    }

    #[test]
    fn invalid_discovery_plugin_unknown() {
        let ops = MockOps::new_with_homebrew_apt();
        let unknown = PluginTypeId::new("unknown_plugin");
        assert!(!is_valid_discovery_plugin(&ops, &unknown));
    }

    #[test]
    fn invalid_discovery_plugin_docker() {
        let ops = MockOps::new_with_homebrew_apt();
        assert!(!is_valid_discovery_plugin(
            &ops,
            &plugin_ids::RELEASES_DOCKER
        ));
    }
}
