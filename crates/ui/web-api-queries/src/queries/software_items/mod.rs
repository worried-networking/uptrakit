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
    pub(super) active_updates: HashMap<Uuid, Uuid>,
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
    let active_updates: HashMap<Uuid, Uuid> = match UpdateHistory::find()
        .filter(update_history::Column::SoftwareItemId.eq(item_id))
        .filter(update_history::Column::HostId.is_in(host_ids.clone()))
        .filter(update_history::Column::Status.is_in([
            UpdateStatus::Queued,
            UpdateStatus::Pending,
            UpdateStatus::InProgress,
        ]))
        .all(db)
        .await
    {
        Ok(rows) => rows.into_iter().map(|u| (u.host_id, u.id)).collect(),
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
    item_id: Uuid,
) -> Vec<SoftwareItemHostSummary> {
    try_load_item_hosts_inner(db, item_id, false)
        .await
        .unwrap_or_default()
}

pub(super) async fn try_load_item_hosts(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
) -> Result<Vec<SoftwareItemHostSummary>> {
    try_load_item_hosts_inner(db, item_id, true).await
}

async fn try_load_item_hosts_inner(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
    strict_config_override: bool,
) -> Result<Vec<SoftwareItemHostSummary>> {
    let Some(data) = load_host_assignment_data(db, item_id).await else {
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
            active_update_history_id: data.active_updates.get(&host.id).copied(),
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
    apply_software_item_patch, batch_delete_software_items, batch_feature_software_items,
    create_software_item, delete_software_item, find_active_item, get_software_item,
    list_software_items, load_items_needing_enrichment, update_software_item,
};

pub use host_assignments::{
    assign_hosts, load_host_assignment, unassign_host, update_host_assignment,
};

pub use merge::{execute_merge_software_items, preview_merge_software_items};
pub use plugin_assignments::delete_plugin_assignment;
