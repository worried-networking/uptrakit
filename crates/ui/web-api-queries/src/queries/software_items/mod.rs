//! Software item query helpers.
//!
//! Organised into three focused submodules:
//!
//! - [`crud`] — create, list, get, update, delete, batch operations
//! - [`host_assignments`] — host-level assignment management
//! - [`merge`] — manual merge preview planning and execution
//! - [`plugin_assignments`] — plugin role assignment management
//!
//! Shared error types, private helper types, and utility functions used across
//! more than one submodule are defined here in `mod.rs`.

use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QuerySelect,
    RelationTrait,
};
use std::collections::HashMap;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, software_item,
    update_history,
};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::PluginRole;
use uptrakit_web_api_types::software_items::{
    HostPluginRoleSummary, JsonObjectMap, SoftwareItemDetailResponse, SoftwareItemHostSummary,
    SoftwareItemResponse,
};
use uuid::Uuid;

mod crud;
mod host_assignments;
mod merge;
mod plugin_assignments;

// ---------------------------------------------------------------------------
// Error type — shared by all submodules via `super::`
// ---------------------------------------------------------------------------

/// Errors returned by software item queries.
#[derive(Debug, thiserror::Error)]
pub enum SoftwareItemQueryError {
    /// Software item not found or deactivated.
    #[error("software item not found")]
    NotFound,
    /// Name must not be empty (for update).
    #[error("name must not be empty")]
    EmptyName,
    /// A software item with the same name already exists.
    #[error("a software item with this name already exists")]
    DuplicateItem,
    /// A host in the request was not found or is deactivated.
    #[error("host not found: {0}")]
    HostNotFound(Uuid),
    /// The referenced plugin config does not exist or is inactive.
    #[error("plugin config not found")]
    PluginConfigNotFound,
    /// A `(host_id, software_item_id, role, ordinal)` combo already exists.
    #[error("duplicate host assignment")]
    DuplicateHostAssignment,
    /// Package identifier failed validation (e.g. Homebrew naming rules).
    #[error("invalid package identifier: {0}")]
    InvalidPackageIdentifier(String),
    /// `config_override` failed plugin-level or hook validation.
    #[error("invalid config override: {0}")]
    InvalidConfigOverride(String),
    /// Inline plugin config failed name/config/hook validation.
    #[error("invalid inline plugin config: {0}")]
    InvalidInlinePluginConfig(String),
    /// Invalid `execution_site` value.
    #[error("invalid execution site: {0}")]
    InvalidExecutionSite(String),
    /// A plugin assignment (role, ordinal) does not exist.
    #[error("plugin assignment not found")]
    PluginAssignmentNotFound,
    /// Zero-source update with no existing assignment to fall back to.
    #[error("no plugin source in request and no existing assignment: {0}")]
    MissingPluginSource(String),
    /// Merge preview/execution validation failed.
    #[error("invalid merge request: {0}")]
    InvalidMergeRequest(String),
    /// The target host is incompatible with the assigned plugin role.
    #[error("host incompatible with role: {0}")]
    IncompatibleHost(String),
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<SoftwareItemQueryError>>;
impl_report_conversion!(sea_orm::DbErr => SoftwareItemQueryError::Db);

impl SoftwareItemQueryError {
    /// Returns the audit classification `(outcome, reason_code)` for this error.
    pub fn audit_classification(&self) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::NotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "software_item.not_found",
            ),
            Self::PluginAssignmentNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "software_item.plugin_assignment_not_found",
            ),
            Self::EmptyName => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.empty_name",
            ),
            Self::DuplicateItem => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.duplicate_item",
            ),
            Self::HostNotFound(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.host_not_found",
            ),
            Self::PluginConfigNotFound => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.plugin_config_not_found",
            ),
            Self::MissingPluginSource(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.missing_plugin_source",
            ),
            Self::DuplicateHostAssignment => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.duplicate_host_assignment",
            ),
            Self::InvalidPackageIdentifier(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.invalid_package_identifier",
            ),
            Self::InvalidConfigOverride(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.invalid_config_override",
            ),
            Self::InvalidInlinePluginConfig(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.invalid_inline_plugin_config",
            ),
            Self::InvalidExecutionSite(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.invalid_execution_site",
            ),
            Self::InvalidMergeRequest(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.invalid_merge_request",
            ),
            Self::IncompatibleHost(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "software_item.incompatible_host",
            ),
            Self::Db(_) => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "software_item.database_error",
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Private helper types — available to submodules via `super::`
// ---------------------------------------------------------------------------

/// All data needed to assemble host summaries, loaded in bulk from the database.
pub(super) struct HostAssignmentData {
    pub(super) links: Vec<host_software_item::Model>,
    pub(super) hosts: HashMap<Uuid, host::Model>,
    pub(super) active_updates: HashMap<Uuid, (Uuid, String)>,
    pub(super) plugin_rows: Vec<host_software_item_plugin::Model>,
    pub(super) plugin_configs: HashMap<Uuid, plugin_config::Model>,
}

// ---------------------------------------------------------------------------
// Shared private helper functions — used by crud, host_assignments, plugin_assignments
// ---------------------------------------------------------------------------

#[expect(
    clippy::too_many_arguments,
    reason = "query function requires all filter parameters; extracting a struct would obscure the call site"
)]
pub(super) fn build_list_response(
    item: &software_item::Model,
    plugins: Vec<String>,
    host_count: u64,
    installed_version: Option<String>,
    installed_display_version: Option<String>,
    latest_version: Option<String>,
    latest_release_metadata: Option<serde_json::Value>,
    update_available: bool,
) -> SoftwareItemResponse {
    SoftwareItemResponse {
        id: item.id,
        name: item.name.clone(),
        plugins,
        featured: item.featured,
        last_checked_at: item.last_checked_at,
        host_count,
        installed_version,
        installed_display_version,
        latest_version,
        latest_release_metadata,
        update_available,
        created_at: item.created_at,
        updated_at: item.updated_at,
        icon_url: item.icon_url.clone(),
    }
}

pub(super) fn build_detail_response(
    item: software_item::Model,
    plugins: Vec<String>,
    host_count: u64,
    latest_version: Option<String>,
    update_available: bool,
    hosts: Vec<SoftwareItemHostSummary>,
) -> SoftwareItemDetailResponse {
    SoftwareItemDetailResponse {
        id: item.id,
        name: item.name.clone(),
        plugins,
        featured: item.featured,
        last_checked_at: item.last_checked_at,
        host_count,
        latest_version,
        update_available,
        created_at: item.created_at,
        updated_at: item.updated_at,
        icon_url: item.icon_url.clone(),
        hosts,
    }
}

/// Compute `update_available` for a single host: both values must be `Some` and differ.
pub(super) fn host_update_available(
    installed_version: Option<&str>,
    latest_version: Option<&str>,
) -> bool {
    match (installed_version, latest_version) {
        (Some(installed), Some(latest)) => installed != latest,
        _ => false,
    }
}

pub(super) async fn count_linked_hosts(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
) -> Result<u64> {
    HostSoftwareItem::find()
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .count(db)
        .await
        .context_to()
}

/// Load the latest version for a single software item across all hosts.
/// Returns the maximum `latest_version` value among all host assignments.
pub(super) async fn load_latest_version_for_item(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
) -> Option<String> {
    #[derive(Debug, FromQueryResult)]
    struct LatestVersionRow {
        latest_version: Option<String>,
    }

    let rows: Vec<LatestVersionRow> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::LatestVersion)
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item::Column::LatestVersion.is_not_null())
        .filter(host::Column::DeactivatedAt.is_null())
        .into_model::<LatestVersionRow>()
        .all(db)
        .await
        .unwrap_or_default();

    rows.into_iter().filter_map(|r| r.latest_version).max()
}

/// Load the distinct plugin types for a software item from its host plugin assignments.
pub(super) async fn load_plugins(db: &sea_orm::DatabaseConnection, item_id: Uuid) -> Vec<String> {
    #[derive(Debug, FromQueryResult)]
    struct PcRow {
        plugin_type: String,
    }

    // Read plugin_type directly from host_software_item_plugins; no join to plugin_configs
    // is needed, and an INNER JOIN would silently drop rows where plugin_config_id IS NULL.
    let rows: Vec<PcRow> = HostSoftwareItemPlugin::find()
        .select_only()
        .column(host_software_item_plugin::Column::PluginType)
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .into_model::<PcRow>()
        .all(db)
        .await
        .unwrap_or_default();

    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|r| seen.insert(r.plugin_type.clone()))
        .map(|r| r.plugin_type)
        .collect()
}

/// Load the 5 bulk queries required to assemble host summaries for a software item.
/// Returns `None` if there are no links or a critical query fails.
pub(super) async fn load_host_assignment_data(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    item_id: Uuid,
) -> Option<HostAssignmentData> {
    let links = match HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .all(db)
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::warn!("Failed to load software item hosts: {e}");
            return None;
        }
    };

    if links.is_empty() {
        return None;
    }

    let host_ids: Vec<Uuid> = links.iter().map(|l| l.host_id).collect();

    let hosts: HashMap<Uuid, host::Model> = match Host::find()
        .filter(host::Column::Id.is_in(host_ids.clone()))
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(db)
        .await
    {
        Ok(h) => h.into_iter().map(|h| (h.id, h)).collect(),
        Err(e) => {
            tracing::warn!("Failed to load hosts for software item: {e}");
            return None;
        }
    };

    // Batch-load active update IDs for all hosts. One query total (no N+1).
    let active_updates: HashMap<Uuid, (Uuid, String)> = match UpdateHistory::find()
        .filter(update_history::Column::SoftwareItemId.eq(item_id))
        .filter(update_history::Column::HostId.is_in(host_ids.clone()))
        .filter(update_history::Column::Status.is_in(UpdateStatus::unfinished()))
        .all(db)
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|u| (u.host_id, (u.id, u.status.to_string())))
            .collect(),
        Err(e) => {
            tracing::warn!("Failed to load active updates for software item: {e}");
            HashMap::new()
        }
    };

    // Bulk-load all plugin role assignments for this software item.
    let plugin_rows = match HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::HostId.is_in(host_ids))
        .all(db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to load host plugin assignments: {e}");
            return None;
        }
    };

    // Collect all plugin config IDs and bulk-load the configs.
    let pc_ids: Vec<Uuid> = plugin_rows
        .iter()
        .filter_map(|r| r.plugin_config_id)
        .collect();
    let plugin_configs: HashMap<Uuid, plugin_config::Model> = match PluginConfig::find()
        .filter(plugin_config::Column::Id.is_in(pc_ids))
        .all(db)
        .await
    {
        Ok(pcs) => pcs.into_iter().map(|pc| (pc.id, pc)).collect(),
        Err(e) => {
            tracing::warn!("Failed to load plugin configs for software item: {e}");
            return None;
        }
    };

    Some(HostAssignmentData {
        links,
        hosts,
        active_updates,
        plugin_rows,
        plugin_configs,
    })
}

pub(super) async fn load_item_hosts(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    item_id: Uuid,
) -> Vec<SoftwareItemHostSummary> {
    try_load_item_hosts_inner(db, tenant_id, item_id, false)
        .await
        .unwrap_or_default()
}

pub(super) async fn try_load_item_hosts(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    item_id: Uuid,
) -> Result<Vec<SoftwareItemHostSummary>> {
    try_load_item_hosts_inner(db, tenant_id, item_id, true).await
}

async fn try_load_item_hosts_inner(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    item_id: Uuid,
    strict_config_override: bool,
) -> Result<Vec<SoftwareItemHostSummary>> {
    let Some(data) = load_host_assignment_data(db, tenant_id, item_id).await else {
        return Ok(Vec::new());
    };

    // Group plugin rows by host_software_item_id so each link only sees its own plugins.
    // Grouping by host_id would merge plugins from sibling links (e.g. two Docker container
    // entries for the same image on the same host), causing duplicate `role` keys.
    let mut plugins_by_link: HashMap<Uuid, Vec<&host_software_item_plugin::Model>> = HashMap::new();
    for row in &data.plugin_rows {
        plugins_by_link
            .entry(row.host_software_item_id)
            .or_default()
            .push(row);
    }

    let mut hosts = Vec::new();

    for link in data.links {
        let Some(host) = data.hosts.get(&link.host_id) else {
            continue;
        };

        let host_plugins = plugins_by_link
            .get(&link.id)
            .map(|rows| {
                rows.iter()
                    .map(|pr| {
                        let pc = pr
                            .plugin_config_id
                            .and_then(|pc_id| data.plugin_configs.get(&pc_id));
                        let config_override = map_response_config_override(
                            pr.config.clone(),
                            strict_config_override,
                        )?;
                        Ok(HostPluginRoleSummary {
                            role: PluginRole::from(pr.role.clone()),
                            ordinal: pr.ordinal,
                            plugin_config_id: pc.map(|c| c.id),
                            plugin_config_name: pc.map(|c| c.name.clone()),
                            plugin_type: pc
                                .map(|c| c.plugin_type.clone())
                                .unwrap_or_else(|| pr.plugin_type.clone()),
                            package_identifier: pr.package_identifier.clone(),
                            config_override,
                            execution_site: pr.execution_site.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .unwrap_or_else(|| Ok(Vec::new()))?;

        let update_avail = host_update_available(
            link.installed_version.as_deref(),
            link.latest_version.as_deref(),
        );

        let active = data.active_updates.get(&host.id);
        hosts.push(SoftwareItemHostSummary {
            id: link.id,
            host_id: host.id,
            hostname: host.hostname.clone(),
            friendly_name: host.friendly_name.clone(),
            qualifier: link.qualifier.clone(),
            plugins: host_plugins,
            installed_version: link.installed_version,
            installed_version_detected_at: link.installed_version_detected_at,
            installed_display_version: link.installed_display_version,
            latest_version: link.latest_version,
            latest_release_metadata: link.latest_release_metadata,
            update_available: update_avail,
            active_update_history_id: active.map(|(id, _)| *id),
            active_update_status: active.map(|(_, s)| s.clone()),
            update_category: link.update_category,
            last_updated_at: link.last_updated_at,
            linked_at: link.linked_at,
        });
    }

    Ok(hosts)
}

fn map_response_config_override(
    value: Option<serde_json::Value>,
    strict: bool,
) -> Result<Option<JsonObjectMap>> {
    match value {
        None => Ok(None),
        Some(value) => match JsonObjectMap::try_from(value) {
            Ok(value) => Ok(Some(value)),
            Err(err) if strict => {
                bail!(SoftwareItemQueryError::InvalidConfigOverride(
                    err.message.to_string()
                ));
            }
            Err(err) => {
                tracing::warn!(
                    message = "ignoring malformed stored config_override while building software item response",
                    error = %err.message
                );
                Ok(None)
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Re-exports for external consumers
// ---------------------------------------------------------------------------

pub use crud::{
    SoftwareItemView, apply_software_item_patch, batch_delete_software_items,
    batch_feature_software_items, create_software_item, create_software_item_in_tx,
    delete_software_item, delete_software_item_in_tx, find_active_item, get_software_item,
    list_software_items, load_items_needing_enrichment, update_software_item,
    update_software_item_in_tx,
};

pub use crud::approve_software_item_in_tx;

pub use host_assignments::{
    assign_hosts, assign_hosts_in_tx, load_host_assignment, unassign_host, unassign_host_in_tx,
    update_host_assignment_in_tx,
};

pub use merge::{execute_merge_software_items, preview_merge_software_items};
pub use plugin_assignments::{delete_plugin_assignment, delete_plugin_assignment_in_tx};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod active_update_status_tests {
    use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{
        host, host_software_item, software_item, tenant, update_history,
    };
    use uuid::Uuid;

    async fn make_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    /// Insert the minimum parent rows required by FK constraints.
    /// Returns `(tenant_id, host_id, software_item_id, host_software_item_id)`.
    async fn insert_parents(db: &DatabaseConnection) -> (Uuid, Uuid, Uuid, Uuid) {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let hsi_id = Uuid::now_v7();

        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set(format!("t-{tenant_id}")),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");

        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{host_id}")),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("Test Host".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host");

        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("test-item".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software_item");

        host_software_item::ActiveModel {
            id: Set(hsi_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
            last_discovered_at: Set(None),
            discovery_source: Set(None),
            missing_since: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host_software_item");

        (tenant_id, host_id, item_id, hsi_id)
    }

    #[tokio::test]
    async fn awaiting_restart_row_populates_active_update_status() {
        let db = make_db().await;
        let now = OffsetDateTime::now_utc();
        let (tenant_id, host_id, item_id, _hsi_id) = insert_parents(&db).await;

        let update_id = Uuid::now_v7();
        update_history::ActiveModel {
            id: Set(update_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            host_software_item_id: Set(None),
            from_version: Set(None),
            to_version: Set(Some("1.0.0".to_string())),
            status: Set(update_history::UpdateStatus::AwaitingRestart),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            awaiting_restart_since: Set(Some(now)),
            created_at: Set(now),
            update_category: Set("unknown".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert update_history");

        let hosts = super::load_item_hosts(&db, tenant_id, item_id).await;

        assert_eq!(hosts.len(), 1, "expected one host summary");
        let summary = &hosts[0];
        assert_eq!(
            summary.active_update_history_id,
            Some(update_id),
            "active_update_history_id should match the AwaitingRestart row"
        );
        assert_eq!(
            summary.active_update_status.as_deref(),
            Some("awaiting_restart"),
            "active_update_status should be 'awaiting_restart'"
        );
    }

    #[tokio::test]
    async fn completed_row_only_leaves_active_update_status_empty() {
        let db = make_db().await;
        let now = OffsetDateTime::now_utc();
        let (tenant_id, host_id, item_id, _hsi_id) = insert_parents(&db).await;

        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            host_software_item_id: Set(None),
            from_version: Set(None),
            to_version: Set(Some("1.0.0".to_string())),
            status: Set(update_history::UpdateStatus::Completed),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(Some(now)),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("unknown".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert update_history");

        let hosts = super::load_item_hosts(&db, tenant_id, item_id).await;

        assert_eq!(hosts.len(), 1, "expected one host summary");
        let summary = &hosts[0];
        assert!(
            summary.active_update_history_id.is_none(),
            "active_update_history_id should be None for a Completed row"
        );
        assert!(
            summary.active_update_status.is_none(),
            "active_update_status should be None for a Completed row"
        );
    }

    #[tokio::test]
    async fn load_item_hosts_excludes_foreign_tenant_host() {
        use uptrakit_shared_db::entity::host_software_item;
        let db = make_db().await;
        let now = OffsetDateTime::now_utc();

        // Tenant A owns the software item; tenant B owns a host.
        let (tenant_a, host_a, item_a, _hsi_a) = insert_parents(&db).await;

        let tenant_b = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(tenant_b),
            name: Set("tenant-b".to_string()),
            slug: Set(format!("t-{tenant_b}")),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert tenant b");

        let host_b = Uuid::now_v7();
        host::ActiveModel {
            id: Set(host_b),
            tenant_id: Set(tenant_b),
            machine_id: Set(format!("machine-{host_b}")),
            hostname: Set("host-b".to_string()),
            friendly_name: Set("Host B".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert host b");

        // A rogue link: tenant A's item points at tenant B's host.
        host_software_item::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_b),
            software_item_id: Set(item_a),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
            last_discovered_at: Set(None),
            discovery_source: Set(None),
            missing_since: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert rogue link");

        // Scoped to tenant A: the foreign host_b must not surface, only the item's own host.
        // Assert positive presence of host_a too — a filter that wrongly dropped ALL
        // hosts would still satisfy a bare `!= host_b` check (vacuous pass).
        let hosts = super::load_item_hosts(&db, tenant_a, item_a).await;
        assert_eq!(
            hosts.len(),
            1,
            "tenant A must see exactly its own host, got {hosts:?}"
        );
        assert_eq!(
            hosts[0].host_id, host_a,
            "the surfaced host must be tenant A's own host_a, not the foreign host_b, got {hosts:?}"
        );
    }
}
