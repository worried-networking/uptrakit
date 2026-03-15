use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, FromQueryResult, ModelTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, RelationTrait, Set, TransactionTrait,
};
use std::collections::HashMap;
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_core::PluginOps;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, software_item,
    update_history,
};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::PluginRole;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, HostPluginRoleAssignment, HostPluginRoleSummary,
    ListSoftwareItemsParams, SoftwareItemDetailResponse, SoftwareItemHostSummary,
    SoftwareItemResponse, UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
};
use uuid::Uuid;

use crate::queries::plugin_configs::find_raw_active_config_txn;
use crate::tenant_db::TenantDb;
use crate::token_utils::generate_uuid;

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
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<SoftwareItemQueryError>>;
impl_report_conversion!(sea_orm::DbErr => SoftwareItemQueryError::Db);

#[derive(Debug, FromQueryResult)]
struct ItemHostCount {
    software_item_id: Uuid,
    count: i64,
}

#[derive(Debug, FromQueryResult)]
struct ItemPluginType {
    software_item_id: Uuid,
    plugin_type: String,
}

// --- Private helpers ---

#[allow(clippy::too_many_arguments)]
fn build_list_response(
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

fn build_detail_response(
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
fn host_update_available(installed_version: Option<&str>, latest_version: Option<&str>) -> bool {
    match (installed_version, latest_version) {
        (Some(installed), Some(latest)) => installed != latest,
        _ => false,
    }
}

async fn count_linked_hosts(db: &sea_orm::DatabaseConnection, item_id: Uuid) -> Result<u64> {
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
async fn load_latest_version_for_item(
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

/// Bulk-load latest versions for multiple software items.
/// Returns a map of `software_item_id` to the maximum `latest_version` across hosts.
async fn bulk_load_latest_versions(
    db: &sea_orm::DatabaseConnection,
    item_ids: &[Uuid],
) -> HashMap<Uuid, String> {
    #[derive(Debug, FromQueryResult)]
    struct ItemLatestRow {
        software_item_id: Uuid,
        latest_version: Option<String>,
    }

    let rows: Vec<ItemLatestRow> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::SoftwareItemId)
        .column(host_software_item::Column::LatestVersion)
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.to_vec()))
        .filter(host_software_item::Column::LatestVersion.is_not_null())
        .filter(host::Column::DeactivatedAt.is_null())
        .into_model::<ItemLatestRow>()
        .all(db)
        .await
        .unwrap_or_default();

    let mut map: HashMap<Uuid, String> = HashMap::new();
    for row in rows {
        if let Some(v) = row.latest_version {
            map.entry(row.software_item_id)
                .and_modify(|existing| {
                    if v > *existing {
                        *existing = v.clone();
                    }
                })
                .or_insert(v);
        }
    }
    map
}

/// Load the distinct plugin types for a software item from its host plugin assignments.
async fn load_plugins(db: &sea_orm::DatabaseConnection, item_id: Uuid) -> Vec<String> {
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

/// All data needed to assemble host summaries, loaded in bulk from the database.
struct HostAssignmentData {
    links: Vec<host_software_item::Model>,
    hosts: HashMap<Uuid, host::Model>,
    active_updates: HashMap<Uuid, Uuid>,
    plugin_rows: Vec<host_software_item_plugin::Model>,
    plugin_configs: HashMap<Uuid, plugin_config::Model>,
}

/// Load the 5 bulk queries required to assemble host summaries for a software item.
/// Returns `None` if there are no links or a critical query fails.
async fn load_host_assignment_data(
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

async fn load_item_hosts(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
) -> Vec<SoftwareItemHostSummary> {
    let Some(data) = load_host_assignment_data(db, item_id).await else {
        return Vec::new();
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

    data.links
        .into_iter()
        .filter_map(|link| {
            let host = data.hosts.get(&link.host_id)?;

            let host_plugins: Vec<HostPluginRoleSummary> = plugins_by_link
                .get(&link.id)
                .map(|rows| {
                    rows.iter()
                        .map(|pr| {
                            let pc = pr
                                .plugin_config_id
                                .and_then(|pc_id| data.plugin_configs.get(&pc_id));
                            HostPluginRoleSummary {
                                role: PluginRole::from(pr.role.clone()),
                                ordinal: pr.ordinal,
                                plugin_config_id: pc.map(|c| c.id),
                                plugin_config_name: pc.map(|c| c.name.clone()),
                                plugin_type: pc
                                    .map(|c| c.plugin_type.clone())
                                    .unwrap_or_else(|| pr.plugin_type.clone()),
                                package_identifier: pr.package_identifier.clone(),
                                config_override: pr.config.clone(),
                                execution_site: pr.execution_site.clone(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            let update_avail = host_update_available(
                link.installed_version.as_deref(),
                link.latest_version.as_deref(),
            );

            Some(SoftwareItemHostSummary {
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
            })
        })
        .collect()
}

/// Find a non-deactivated software item by ID, scoped to a tenant.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn find_active_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    id: Uuid,
) -> Option<software_item::Model> {
    SoftwareItem::find_by_id(id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

/// Error returned when `config_override` validation fails.
#[derive(Debug, thiserror::Error)]
enum ConfigOverrideError {
    #[error("config_override must be a JSON object")]
    NotAnObject,
    #[error("plugin validation failed: {0}")]
    PluginValidation(String),
}

/// Validate `config_override` by merging it with the base plugin config and running
/// plugin-specific validation. The merged document must satisfy the plugin's schema.
fn validate_config_override(
    ops: &dyn PluginOps,
    plugin_type: &str,
    base_config: &serde_json::Value,
    override_config: &serde_json::Value,
) -> std::result::Result<(), ConfigOverrideError> {
    let mut merged = base_config.clone();
    if let (Some(base_obj), Some(over_obj)) = (merged.as_object_mut(), override_config.as_object())
    {
        for (k, v) in over_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    } else {
        return Err(ConfigOverrideError::NotAnObject);
    }

    ops.validate_config_str(plugin_type, &merged)
        .map_err(|e| ConfigOverrideError::PluginValidation(e.to_string()))
}

/// Validate that `execution_site` is one of the allowed values and that
/// "controller" is only used with the "fetch_releases" role.
fn validate_execution_site(execution_site: &str, role: &PluginRole) -> Result<()> {
    match execution_site {
        "auto" | "agent" => Ok(()),
        "controller" => {
            if *role == PluginRole::FetchReleases {
                Ok(())
            } else {
                Err(report!(SoftwareItemQueryError::InvalidExecutionSite(
                    format!(
                        "execution_site \"controller\" is only valid for the \"fetch_releases\" role, got \"{}\"",
                        role,
                    )
                )))
            }
        }
        other => Err(report!(SoftwareItemQueryError::InvalidExecutionSite(
            format!(
                "invalid execution_site value \"{other}\"; must be \"auto\", \"agent\", or \"controller\""
            )
        ))),
    }
}

/// Resolve plugin config from either an existing ID or an inline create request,
/// within a transaction. Returns `(plugin_config_id, plugin_config::Model)`.
async fn resolve_plugin_config_txn(
    ops: &dyn PluginOps,
    txn: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    assignment: &HostPluginRoleAssignment,
) -> Result<(Uuid, plugin_config::Model)> {
    match (&assignment.plugin_config_id, &assignment.plugin_config) {
        (Some(pcid), None) => {
            let pcid = *pcid;
            let c = find_raw_active_config_txn(txn, tenant_id, pcid)
                .await
                .map_err(|e| {
                    report!(SoftwareItemQueryError::Db(sea_orm::DbErr::Custom(
                        e.to_string()
                    )))
                })?
                .ok_or_else(|| report!(SoftwareItemQueryError::PluginConfigNotFound))?;
            Ok((pcid, c))
        }
        (None, Some(inline)) => {
            if inline.name.is_empty() {
                bail!(SoftwareItemQueryError::InvalidInlinePluginConfig(
                    "name must not be empty".to_string(),
                ));
            }
            if let Err(e) = ops.validate_config_str(inline.plugin_type.as_str(), &inline.config) {
                bail!(SoftwareItemQueryError::InvalidInlinePluginConfig(
                    e.to_string()
                ));
            }
            let now = OffsetDateTime::now_utc();
            let pcid = generate_uuid();
            let model = plugin_config::ActiveModel {
                id: Set(pcid),
                tenant_id: Set(tenant_id),
                name: Set(inline.name.clone()),
                plugin_type: Set(inline.plugin_type.to_string()),
                config: Set(inline.config.clone()),
                enabled: Set(inline.enabled),
                created_at: Set(now),
                updated_at: Set(now),
                deactivated_at: Set(None),
            };
            let inserted = model.insert(txn).await.context_to()?;
            Ok((pcid, inserted))
        }
        _ => Err(report!(SoftwareItemQueryError::PluginConfigNotFound)),
    }
}

/// Validate plugin config, package identifier, and config_override for a host assignment.
fn validate_assignment(
    ops: &dyn PluginOps,
    config: &plugin_config::Model,
    package_identifier: &str,
    config_override: Option<&serde_json::Value>,
) -> Result<()> {
    if let Err(e) = ops.validate_package_identifier_str(&config.plugin_type, package_identifier) {
        bail!(SoftwareItemQueryError::InvalidPackageIdentifier(e));
    }

    if let Some(override_val) = config_override
        && let Err(e) =
            validate_config_override(ops, &config.plugin_type, &config.config, override_val)
    {
        bail!(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
    }

    Ok(())
}

// --- Public query functions ---

/// Create a new software item (catalog entry only). Check unique name constraint.
#[tracing::instrument(skip_all)]
pub async fn create_software_item(
    tenant_db: &TenantDb,
    req: CreateSoftwareItemRequest,
) -> Result<SoftwareItemResponse> {
    let txn = tenant_db.db().begin().await.context_to()?;

    // Check uniqueness: name must be unique among active items for this tenant.
    let duplicate = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::Name.eq(&req.name))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .context_to()?;

    if duplicate.is_some() {
        bail!(SoftwareItemQueryError::DuplicateItem);
    }

    let now = OffsetDateTime::now_utc();
    let model = software_item::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name),
        featured: Set(req.featured),
        icon_url: Set(req.icon_url),
        last_checked_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let inserted = model.insert(&txn).await.context_to()?;

    txn.commit().await.context_to()?;

    Ok(build_list_response(
        &inserted,
        vec![],
        0,
        None,
        None,
        None,
        None,
        false,
    ))
}

/// Build the EXISTS subquery that checks whether a software item has at least one
/// active host assignment where `installed_version != latest_version` (both non-null).
fn build_updatable_exists_subquery() -> sea_orm::sea_query::SelectStatement {
    use sea_orm::sea_query::{BinOper, Expr, ExprTrait, Query};

    Query::select()
        .expr(Expr::val(1_i32))
        .from(host_software_item::Entity)
        .inner_join(
            host::Entity,
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::HostId,
            ))
            .equals((host::Entity, host::Column::Id)),
        )
        .and_where(
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::SoftwareItemId,
            ))
            .equals((software_item::Entity, software_item::Column::Id)),
        )
        .and_where(Expr::col((host::Entity, host::Column::DeactivatedAt)).is_null())
        .and_where(
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::InstalledVersion,
            ))
            .is_not_null(),
        )
        .and_where(
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::LatestVersion,
            ))
            .is_not_null(),
        )
        .and_where(
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::InstalledVersion,
            ))
            .binary(
                BinOper::NotEqual,
                Expr::col((
                    host_software_item::Entity,
                    host_software_item::Column::LatestVersion,
                )),
            ),
        )
        .take()
}

// (installed_version, latest_version, installed_display_version, latest_release_metadata)
type VersionPair = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<serde_json::Value>,
);

#[derive(Debug, FromQueryResult)]
struct InstalledVersionRow {
    software_item_id: Uuid,
    installed_version: Option<String>,
    latest_version: Option<String>,
    installed_display_version: Option<String>,
    latest_release_metadata: Option<serde_json::Value>,
}

/// Pre-loaded enrichment data for the list endpoint: host counts, plugin types,
/// latest versions, and installed version pairs.
struct ListEnrichment {
    host_counts: HashMap<Uuid, u64>,
    plugins_map: HashMap<Uuid, Vec<String>>,
    latest_versions: HashMap<Uuid, String>,
    installed_map: HashMap<Uuid, Vec<VersionPair>>,
}

/// Bulk-load the four enrichment queries needed by `list_software_items`.
async fn bulk_load_list_enrichment(
    db: &sea_orm::DatabaseConnection,
    item_ids: &[Uuid],
    host_id_filter: Option<Uuid>,
) -> Result<ListEnrichment> {
    use sea_orm::sea_query::Expr;

    // Bulk-load host counts for all items in one GROUP BY query.
    // Only active (non-deactivated) hosts are counted.
    let host_counts: HashMap<Uuid, u64> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::SoftwareItemId)
        .column_as(
            {
                use sea_orm::sea_query::ExprTrait;
                Expr::col(host_software_item::Column::HostId).count()
            },
            "count",
        )
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.to_vec()))
        .filter(host::Column::DeactivatedAt.is_null())
        .group_by(host_software_item::Column::SoftwareItemId)
        .into_model::<ItemHostCount>()
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|row| (row.software_item_id, row.count as u64))
        .collect();

    // Bulk-load plugin types for all items directly from host_software_item_plugins.
    // plugin_type is stored on the row itself; no join to plugin_configs is needed, and
    // an INNER JOIN would silently drop rows where plugin_config_id IS NULL.
    let plugin_type_rows: Vec<ItemPluginType> = HostSoftwareItemPlugin::find()
        .select_only()
        .column(host_software_item_plugin::Column::SoftwareItemId)
        .column(host_software_item_plugin::Column::PluginType)
        .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(item_ids.to_vec()))
        .into_model::<ItemPluginType>()
        .all(db)
        .await
        .context_to()?;

    // Group plugin types by software item id, deduplicated.
    let mut plugins_map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in plugin_type_rows {
        let entry = plugins_map.entry(row.software_item_id).or_default();
        if !entry.contains(&row.plugin_type) {
            entry.push(row.plugin_type);
        }
    }

    // Bulk-load latest known versions from host_software_items for all items.
    let latest_versions = bulk_load_latest_versions(db, item_ids).await;

    // Bulk-load all host installed_versions for update_available computation.
    // Map: software_item_id -> list of (installed_version, latest_version) pairs.
    let mut installed_query = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::SoftwareItemId)
        .column(host_software_item::Column::InstalledVersion)
        .column(host_software_item::Column::LatestVersion)
        .column(host_software_item::Column::InstalledDisplayVersion)
        .column(host_software_item::Column::LatestReleaseMetadata)
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.to_vec()))
        .filter(host::Column::DeactivatedAt.is_null());

    if let Some(host_id) = host_id_filter {
        installed_query = installed_query.filter(host_software_item::Column::HostId.eq(host_id));
    }

    let installed_rows: Vec<InstalledVersionRow> = installed_query
        .into_model::<InstalledVersionRow>()
        .all(db)
        .await
        .context_to()?;

    let mut installed_map: HashMap<Uuid, Vec<VersionPair>> = HashMap::new();
    for row in installed_rows {
        installed_map
            .entry(row.software_item_id)
            .or_default()
            .push((
                row.installed_version,
                row.latest_version,
                row.installed_display_version,
                row.latest_release_metadata,
            ));
    }

    Ok(ListEnrichment {
        host_counts,
        plugins_map,
        latest_versions,
        installed_map,
    })
}

#[tracing::instrument(skip_all)]
pub async fn list_software_items(
    tenant_db: &TenantDb,
    params: &ListSoftwareItemsParams,
) -> Result<PaginatedResponse<SoftwareItemResponse>> {
    use sea_orm::sea_query::Expr;

    let pagination = params.pagination().resolve();

    let mut base_query = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .order_by(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                software_item::Column::Name,
            )),
            sea_orm::sea_query::Order::Asc,
        );

    if let Some(featured) = params.featured {
        base_query = base_query.filter(software_item::Column::Featured.eq(featured));
    }

    if let Some(host_id) = params.host_id {
        base_query = base_query
            .join(
                sea_orm::JoinType::InnerJoin,
                host_software_item::Relation::SoftwareItem.def().rev(),
            )
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::DeactivatedAt.is_null());
    }

    if let Some(updatable) = params.updatable {
        let exists_sub = build_updatable_exists_subquery();
        if updatable {
            base_query = base_query.filter(Expr::exists(exists_sub));
        } else {
            base_query = base_query.filter(Expr::not_exists(exists_sub));
        }
    }

    if let Some(plugin_type) = &params.plugin_type {
        use sea_orm::sea_query::{BinOper, ExprTrait, Query};
        // EXISTS subquery: item has at least one plugin assignment of this type.
        // Supported by idx_hsip_software_item_id_plugin_type.
        let plugin_type_sub = Query::select()
            .expr(Expr::val(1_i32))
            .from(host_software_item_plugin::Entity)
            .and_where(
                Expr::col((
                    host_software_item_plugin::Entity,
                    host_software_item_plugin::Column::SoftwareItemId,
                ))
                .equals((software_item::Entity, software_item::Column::Id)),
            )
            .and_where(
                Expr::col((
                    host_software_item_plugin::Entity,
                    host_software_item_plugin::Column::PluginType,
                ))
                .binary(BinOper::Equal, Expr::val(plugin_type.as_str())),
            )
            .take();
        base_query = base_query.filter(Expr::exists(plugin_type_sub));
    }

    let total = base_query
        .clone()
        .count(tenant_db.db())
        .await
        .context_to()?;

    let items = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;

    if items.is_empty() {
        return Ok(PaginatedResponse::new(vec![], total, pagination));
    }

    let item_ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();
    let host_id_filter = params.host_id;

    let mut enrichment =
        bulk_load_list_enrichment(tenant_db.db(), &item_ids, host_id_filter).await?;

    let response: Vec<SoftwareItemResponse> = items
        .iter()
        .map(|item| {
            let plugins = enrichment.plugins_map.remove(&item.id).unwrap_or_default();
            let host_count = enrichment.host_counts.get(&item.id).copied().unwrap_or(0);
            let latest_version = enrichment.latest_versions.get(&item.id).cloned();
            let update_available = enrichment
                .installed_map
                .get(&item.id)
                .map(|pairs| {
                    pairs
                        .iter()
                        .any(|(iv, lv, _, _)| host_update_available(iv.as_deref(), lv.as_deref()))
                })
                .unwrap_or(false);
            // When filtered by host_id, expose the per-host installed version and extra fields.
            let (installed_version, installed_display_version, latest_release_metadata) =
                if host_id_filter.is_some() {
                    enrichment
                        .installed_map
                        .get(&item.id)
                        .and_then(|pairs| pairs.first())
                        .map(|(iv, _lv, idv, lrm)| (iv.clone(), idv.clone(), lrm.clone()))
                        .unwrap_or((None, None, None))
                } else {
                    (None, None, None)
                };
            build_list_response(
                item,
                plugins,
                host_count,
                installed_version,
                installed_display_version,
                latest_version,
                latest_release_metadata,
                update_available,
            )
        })
        .collect();

    Ok(PaginatedResponse::new(response, total, pagination))
}

/// Returns `None` if not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn get_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<SoftwareItemDetailResponse>> {
    let Some(item) = find_active_item(tenant_db.db(), tenant_db.tenant_id, id).await else {
        return Ok(None);
    };

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let plugins = load_plugins(tenant_db.db(), id).await;

    // Latest version for the item is the max across all hosts.
    let latest_version = load_latest_version_for_item(tenant_db.db(), id).await;
    let update_available = hosts.iter().any(|h| h.update_available);

    Ok(Some(build_detail_response(
        item,
        plugins,
        host_count,
        latest_version,
        update_available,
        hosts,
    )))
}

/// Partial update -- only `name` and `enabled` are updatable.
/// Returns `Err(NotFound)` if the item does not exist or is deactivated.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn update_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
    req: UpdateSoftwareItemRequest,
) -> Result<SoftwareItemResponse> {
    let existing = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    if let Some(ref name) = req.name
        && name.is_empty()
    {
        bail!(SoftwareItemQueryError::EmptyName);
    }

    // Check for name collision when renaming.
    if let Some(ref new_name) = req.name
        && new_name != &existing.name
    {
        let duplicate = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
            .filter(software_item::Column::Name.eq(new_name))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .filter(software_item::Column::Id.ne(id))
            .one(tenant_db.db())
            .await
            .context_to()?;

        if duplicate.is_some() {
            bail!(SoftwareItemQueryError::DuplicateItem);
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    if let Some(featured) = req.featured {
        model.featured = Set(featured);
    }
    match req.icon_url {
        Some(serde_json::Value::Null) => model.icon_url = Set(None),
        Some(serde_json::Value::String(s)) => model.icon_url = Set(Some(s)),
        _ => {}
    }
    model.updated_at = Set(now);

    let updated = model.update(tenant_db.db()).await.context_to()?;
    let plugins = load_plugins(tenant_db.db(), id).await;
    let host_count = count_linked_hosts(tenant_db.db(), id).await?;
    let latest_version = load_latest_version_for_item(tenant_db.db(), id).await;

    // For update_available we do a quick per-host check (active hosts only).
    let update_available = if latest_version.is_some() {
        HostSoftwareItem::find()
            .join(
                sea_orm::JoinType::InnerJoin,
                host_software_item::Relation::Host.def(),
            )
            .filter(host_software_item::Column::SoftwareItemId.eq(id))
            .filter(host_software_item::Column::InstalledVersion.is_not_null())
            .filter(host::Column::DeactivatedAt.is_null())
            .all(tenant_db.db())
            .await
            .unwrap_or_default()
            .iter()
            .any(|h| {
                host_update_available(h.installed_version.as_deref(), h.latest_version.as_deref())
            })
    } else {
        false
    };
    Ok(build_list_response(
        &updated,
        plugins,
        host_count,
        None,
        None,
        latest_version,
        None,
        update_available,
    ))
}

/// Soft-delete a software item. Returns `true` if deleted, `false` if not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn delete_software_item(tenant_db: &TenantDb, id: Uuid) -> Result<bool> {
    let Some(item) = find_active_item(tenant_db.db(), tenant_db.tenant_id, id).await else {
        return Ok(false);
    };

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = item.into();
    model.deactivated_at = Set(Some(now));
    model.updated_at = Set(now);
    model.update(tenant_db.db()).await.context_to()?;
    Ok(true)
}

/// Ensure a `host_software_item` link row exists for the given host and item.
/// Returns the link's ID (existing or newly created).
async fn ensure_host_link(
    txn: &impl sea_orm::ConnectionTrait,
    host_id: Uuid,
    item_id: Uuid,
    now: OffsetDateTime,
) -> Result<Uuid> {
    let existing_link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .one(txn)
        .await
        .context_to()?;

    if let Some(ref link) = existing_link {
        return Ok(link.id);
    }

    let new_hsi_id = Uuid::now_v7();
    let link = host_software_item::ActiveModel {
        id: Set(new_hsi_id),
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
    };
    link.insert(txn).await.context_to()?;
    Ok(new_hsi_id)
}

/// Validate, resolve, and upsert a single role assignment for a host-software-item pair.
#[allow(clippy::too_many_arguments)]
async fn upsert_role_assignment(
    ops: &dyn PluginOps,
    txn: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    host_id: Uuid,
    item_id: Uuid,
    hsi_id: Uuid,
    role_assignment: &HostPluginRoleAssignment,
    now: OffsetDateTime,
) -> Result<()> {
    let role = &role_assignment.role;
    let execution_site = &role_assignment.execution_site;

    // Validate execution_site.
    validate_execution_site(execution_site, role)?;

    let (plugin_config_id, config) =
        resolve_plugin_config_txn(ops, txn, tenant_id, role_assignment).await?;

    validate_assignment(
        ops,
        &config,
        &role_assignment.package_identifier,
        role_assignment.config_override.as_ref(),
    )?;

    // Check for existing assignment at the same (host_id, software_item_id, role, ordinal).
    let existing_plugin = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role.as_str()))
        .filter(host_software_item_plugin::Column::Ordinal.eq(role_assignment.ordinal))
        .one(txn)
        .await
        .context_to()?;

    match existing_plugin {
        Some(existing) => {
            // Update existing plugin assignment for this role.
            let mut active: host_software_item_plugin::ActiveModel = existing.into();
            active.plugin_config_id = Set(Some(plugin_config_id));
            active.plugin_type = Set(config.plugin_type.clone());
            active.package_identifier = Set(role_assignment.package_identifier.clone());
            active.config = Set(role_assignment.config_override.clone());
            active.execution_site = Set(execution_site.clone());
            active.updated_at = Set(now);
            active.update(txn).await.context_to()?;
        }
        None => {
            let plugin_row = host_software_item_plugin::ActiveModel {
                id: Set(generate_uuid()),
                host_id: Set(host_id),
                software_item_id: Set(item_id),
                host_software_item_id: Set(hsi_id),
                plugin_config_id: Set(Some(plugin_config_id)),
                plugin_type: Set(config.plugin_type.clone()),
                role: Set(role.as_str().to_string()),
                ordinal: Set(role_assignment.ordinal),
                package_identifier: Set(role_assignment.package_identifier.clone()),
                config: Set(role_assignment.config_override.clone()),
                execution_site: Set(execution_site.clone()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            plugin_row.insert(txn).await.map_err(|e| {
                // Check if this is a unique constraint violation
                // (host_id, software_item_id, role, ordinal).
                if matches!(e, sea_orm::DbErr::Query(..))
                    || e.to_string().contains("UNIQUE")
                    || e.to_string().contains("duplicate")
                {
                    report!(SoftwareItemQueryError::DuplicateHostAssignment)
                } else {
                    report!(SoftwareItemQueryError::Db(e))
                }
            })?;
        }
    }

    Ok(())
}

/// Assign hosts to a software item. Each host carries its own list of role-specific
/// plugin assignments. Returns the updated detail response, or an error if the item
/// or a host is not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn assign_hosts(
    ops: &dyn PluginOps,
    tenant_db: &TenantDb,
    id: Uuid,
    req: AssignHostsRequest,
) -> Result<SoftwareItemDetailResponse> {
    find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let txn = tenant_db.db().begin().await.context_to()?;

    let now = OffsetDateTime::now_utc();

    for assignment in &req.host_assignments {
        let host_id = assignment.host_id;

        let host_exists = Host::find_by_id(host_id)
            .filter(host::Column::DeactivatedAt.is_null())
            .one(&txn)
            .await
            .context_to()?;

        if host_exists.is_none() {
            bail!(SoftwareItemQueryError::HostNotFound(host_id));
        }

        let hsi_id = ensure_host_link(&txn, host_id, id, now).await?;

        for role_assignment in &assignment.plugins {
            upsert_role_assignment(
                ops,
                &txn,
                tenant_db.tenant_id,
                host_id,
                id,
                hsi_id,
                role_assignment,
                now,
            )
            .await?;
        }
    }

    txn.commit().await.context_to()?;

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let plugins = load_plugins(tenant_db.db(), id).await;
    let latest_version = load_latest_version_for_item(tenant_db.db(), id).await;
    let update_available = hosts.iter().any(|h| h.update_available);
    Ok(build_detail_response(
        item,
        plugins,
        host_count,
        latest_version,
        update_available,
        hosts,
    ))
}

/// Update a single role assignment for an existing host-software-item pair.
#[tracing::instrument(skip_all, fields(%id, %host_id))]
pub async fn update_host_assignment(
    ops: &dyn PluginOps,
    tenant_db: &TenantDb,
    id: Uuid,
    host_id: Uuid,
    req: UpdateHostAssignmentRequest,
) -> Result<SoftwareItemDetailResponse> {
    find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    // Verify the host_software_item link exists.
    let hsi_link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(id))
        .one(tenant_db.db())
        .await
        .context_to()?
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;
    let hsi_id = hsi_link.id;

    // Load the existing plugin assignment for this role.
    let existing_plugin = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(id))
        .filter(host_software_item_plugin::Column::Role.eq(req.role.as_str()))
        .filter(host_software_item_plugin::Column::Ordinal.eq(req.ordinal))
        .one(tenant_db.db())
        .await
        .context_to()?;

    // Build a synthetic role assignment to reuse resolve_plugin_config_txn.
    let (existing_pcid, existing_pkg, existing_override, existing_exec_site) =
        if let Some(ref ep) = existing_plugin {
            (
                ep.plugin_config_id,
                Some(ep.package_identifier.clone()),
                ep.config.clone(),
                Some(ep.execution_site.clone()),
            )
        } else {
            (None, None, None, None)
        };

    let synthetic = HostPluginRoleAssignment {
        role: req.role.clone(),
        ordinal: req.ordinal,
        plugin_config_id: req.plugin_config_id.or(existing_pcid),
        plugin_config: req.plugin_config,
        package_identifier: req
            .package_identifier
            .clone()
            .or(existing_pkg.clone())
            .unwrap_or_default(),
        config_override: req.config_override.clone().or(existing_override),
        execution_site: req
            .execution_site
            .clone()
            .or(existing_exec_site)
            .unwrap_or_else(|| "auto".to_string()),
    };

    // Validate execution_site.
    validate_execution_site(&synthetic.execution_site, &req.role)?;

    let txn = tenant_db.db().begin().await.context_to()?;

    let (plugin_config_id, config) =
        resolve_plugin_config_txn(ops, &txn, tenant_db.tenant_id, &synthetic).await?;

    validate_assignment(
        ops,
        &config,
        &synthetic.package_identifier,
        synthetic.config_override.as_ref(),
    )?;

    let now = OffsetDateTime::now_utc();

    match existing_plugin {
        Some(existing) => {
            let mut active: host_software_item_plugin::ActiveModel = existing.into();
            active.plugin_config_id = Set(Some(plugin_config_id));
            active.plugin_type = Set(config.plugin_type.clone());
            active.package_identifier = Set(synthetic.package_identifier);

            // Handle config: explicit null in request clears it.
            if let Some(ref override_val) = req.config_override {
                if override_val.is_null() {
                    active.config = Set(None);
                } else {
                    active.config = Set(Some(override_val.clone()));
                }
            }

            active.execution_site = Set(synthetic.execution_site);
            active.updated_at = Set(now);

            active.update(&txn).await.context_to()?;
        }
        None => {
            // No existing plugin for this role -- create a new one.
            let plugin_row = host_software_item_plugin::ActiveModel {
                id: Set(generate_uuid()),
                host_id: Set(host_id),
                software_item_id: Set(id),
                host_software_item_id: Set(hsi_id),
                plugin_config_id: Set(Some(plugin_config_id)),
                plugin_type: Set(config.plugin_type.clone()),
                role: Set(req.role.as_str().to_string()),
                ordinal: Set(req.ordinal),
                package_identifier: Set(synthetic.package_identifier),
                config: Set(synthetic.config_override),
                execution_site: Set(synthetic.execution_site),
                created_at: Set(now),
                updated_at: Set(now),
            };
            plugin_row.insert(&txn).await.context_to()?;
        }
    }

    txn.commit().await.context_to()?;

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let plugins = load_plugins(tenant_db.db(), id).await;
    let latest_version = load_latest_version_for_item(tenant_db.db(), id).await;
    let update_available = hosts.iter().any(|h| h.update_available);
    Ok(build_detail_response(
        item,
        plugins,
        host_count,
        latest_version,
        update_available,
        hosts,
    ))
}

/// Unassign a host from a software item.
/// Returns `true` if removed, `false` if the software item or link was not found.
/// Cascade deletes will remove the associated `host_software_item_plugins` rows.
#[tracing::instrument(skip_all, fields(%id, %host_id))]
pub async fn unassign_host(tenant_db: &TenantDb, id: Uuid, host_id: Uuid) -> Result<bool> {
    if find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .is_none()
    {
        return Ok(false);
    }

    let link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(id))
        .one(tenant_db.db())
        .await
        .context_to()?;

    match link {
        Some(l) => {
            l.delete(tenant_db.db()).await.context_to()?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Remove a specific plugin assignment identified by `(item_id, host_id, role, ordinal)`.
///
/// Returns the updated [`SoftwareItemDetailResponse`] on success. Returns
/// `SoftwareItemQueryError::NotFound` when the software item does not exist or
/// is deactivated, and `SoftwareItemQueryError::PluginAssignmentNotFound` when
/// no matching plugin row exists.
#[tracing::instrument(skip_all, fields(%item_id, %host_id, %role, %ordinal))]
pub async fn delete_plugin_assignment(
    tenant_db: &TenantDb,
    item_id: Uuid,
    host_id: Uuid,
    role: PluginRole,
    ordinal: i32,
) -> Result<SoftwareItemDetailResponse> {
    find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let deleted = HostSoftwareItemPlugin::delete_many()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role.as_str()))
        .filter(host_software_item_plugin::Column::Ordinal.eq(ordinal))
        .exec(tenant_db.db())
        .await
        .context_to()?;

    if deleted.rows_affected == 0 {
        bail!(SoftwareItemQueryError::PluginAssignmentNotFound);
    }

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let hosts = load_item_hosts(tenant_db.db(), item_id).await;
    let host_count = hosts.len() as u64;
    let plugins = load_plugins(tenant_db.db(), item_id).await;
    let latest_version = load_latest_version_for_item(tenant_db.db(), item_id).await;
    let update_available = hosts.iter().any(|h| h.update_available);
    Ok(build_detail_response(
        item,
        plugins,
        host_count,
        latest_version,
        update_available,
        hosts,
    ))
}

/// Load the host_software_item link for a specific host assignment.
/// Used by route handlers to verify the assignment exists.
#[tracing::instrument(skip_all, fields(%host_id, %software_item_id))]
pub async fn load_host_assignment(
    db: &sea_orm::DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
) -> Option<host_software_item::Model> {
    HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
        .one(db)
        .await
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Feature multiple software items (set `featured = true`).
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_feature_software_items(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let items = software_item::Entity::find()
        .filter(software_item::Column::Id.is_in(ids.iter().copied()))
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, software_item::Model> =
        items.into_iter().map(|i| (i.id, i)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    for (id, item) in &found {
        let mut active: software_item::ActiveModel = item.clone().into();
        active.featured = Set(true);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

/// Soft-delete multiple software items.
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_delete_software_items(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let items = software_item::Entity::find()
        .filter(software_item::Column::Id.is_in(ids.iter().copied()))
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, software_item::Model> =
        items.into_iter().map(|i| (i.id, i)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    for (id, item) in &found {
        let mut active: software_item::ActiveModel = item.clone().into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn build_list_response_formats_timestamps() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Node.js".to_string(),
            featured: true,
            icon_url: None,
            last_checked_at: Some(now),
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(
            &item,
            vec!["releases_github".to_string()],
            3,
            None,
            None,
            Some("22.0.0".to_string()),
            None,
            true,
        );

        assert_eq!(resp.name, "Node.js");
        assert_eq!(resp.plugins, vec!["releases_github"]);
        assert_eq!(resp.host_count, 3);
        assert!(resp.last_checked_at.is_some());
        assert!(resp.installed_version.is_none());
        assert_eq!(resp.latest_version.as_deref(), Some("22.0.0"));
        assert!(resp.update_available);
    }

    #[test]
    fn build_list_response_update_available_false_no_latest() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            featured: true,
            icon_url: None,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(&item, vec![], 0, None, None, None, None, false);

        assert!(!resp.update_available);
        assert!(resp.installed_version.is_none());
        assert!(resp.latest_version.is_none());
    }

    #[test]
    fn build_list_response_with_installed_version() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            featured: true,
            icon_url: None,
            last_checked_at: Some(now),
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(
            &item,
            vec!["package_manager_apt".to_string()],
            1,
            Some("1.24.0".to_string()),
            None,
            Some("1.26.0".to_string()),
            None,
            true,
        );

        assert_eq!(resp.installed_version.as_deref(), Some("1.24.0"));
        assert_eq!(resp.latest_version.as_deref(), Some("1.26.0"));
        assert!(resp.update_available);
    }

    #[test]
    fn host_update_available_semantics() {
        // Both known and differ -> true
        assert!(host_update_available(Some("1.0.0"), Some("2.0.0")));
        // Same version -> false
        assert!(!host_update_available(Some("2.0.0"), Some("2.0.0")));
        // Missing installed -> false
        assert!(!host_update_available(None, Some("2.0.0")));
        // Missing latest -> false
        assert!(!host_update_available(Some("1.0.0"), None));
        // Both missing -> false
        assert!(!host_update_available(None, None));
    }

    #[test]
    fn build_detail_response_includes_hosts() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Redis".to_string(),
            featured: true,
            icon_url: None,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let hosts = vec![SoftwareItemHostSummary {
            id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            hostname: "web-01".to_string(),
            friendly_name: "Web Server 1".to_string(),
            qualifier: None,
            plugins: vec![HostPluginRoleSummary {
                role: PluginRole::FetchReleases,
                ordinal: 0,
                plugin_config_id: Some(uuid::Uuid::now_v7()),
                plugin_config_name: Some("GitHub Releases".to_string()),
                plugin_type: "releases_github".to_string(),
                package_identifier: "redis/redis".to_string(),
                config_override: Some(serde_json::json!({"asset_patterns": ["redis.*linux"]})),
                execution_site: "auto".to_string(),
            }],
            installed_version: Some("7.2.4".to_string()),
            installed_version_detected_at: Some(now),
            installed_display_version: None,
            latest_version: Some("7.4.0".to_string()),
            latest_release_metadata: None,
            update_available: true,
            active_update_history_id: None,
            update_category: "unknown".to_string(),
            last_updated_at: None,
            linked_at: now,
        }];

        let resp = build_detail_response(
            item,
            vec!["releases_github".to_string()],
            1,
            Some("7.4.0".to_string()),
            true,
            hosts,
        );

        assert_eq!(resp.name, "Redis");
        assert_eq!(resp.plugins, vec!["releases_github"]);
        assert_eq!(resp.hosts.len(), 1);
        assert_eq!(resp.hosts[0].hostname, "web-01");
        assert_eq!(resp.hosts[0].plugins.len(), 1);
        assert_eq!(resp.hosts[0].plugins[0].role, PluginRole::FetchReleases);
        assert_eq!(resp.hosts[0].plugins[0].package_identifier, "redis/redis");
        assert_eq!(resp.hosts[0].installed_version, Some("7.2.4".to_string()));
        assert_eq!(resp.hosts[0].latest_version.as_deref(), Some("7.4.0"));
        assert!(resp.hosts[0].update_available);
        assert!(resp.update_available);
    }

    #[test]
    fn build_list_response_null_last_checked_at() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            featured: false,
            icon_url: None,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(&item, vec![], 0, None, None, None, None, false);

        assert!(!resp.featured);
        assert!(resp.last_checked_at.is_none());
        assert_eq!(resp.host_count, 0);
        assert!(resp.plugins.is_empty());
        assert!(!resp.update_available);
    }

    /// Mock [`PluginOps`] for config-override tests.
    ///
    /// Rejects configs containing `"api_base_url"` with an `"http://"` value
    /// (mimics the real GitHub plugin's HTTPS-only check). Accepts everything
    /// else.
    struct MockPluginOps;

    impl PluginOps for MockPluginOps {
        fn validate_config_str(
            &self,
            _plugin_type: &str,
            config: &serde_json::Value,
        ) -> uptrakit_plugin_infrastructure_core::plugin_ops::Result<()> {
            if let Some(url) = config.get("api_base_url").and_then(|v| v.as_str())
                && url.starts_with("http://")
            {
                return Err(rootcause::report!(
                    uptrakit_plugin_infrastructure_core::PluginOpsError::ConfigValidation(
                        "api_base_url must use HTTPS".to_string()
                    )
                ));
            }
            Ok(())
        }

        fn mask_config_secrets_str(
            &self,
            _plugin_type: &str,
            config: &serde_json::Value,
        ) -> serde_json::Value {
            config.clone()
        }

        fn restore_config_secrets_str(
            &self,
            _plugin_type: &str,
            _incoming: &mut serde_json::Value,
            _existing: &serde_json::Value,
        ) {
        }

        fn known_plugin_types(&self) -> Vec<uptrakit_plugin_infrastructure_core::PluginType> {
            vec![]
        }

        fn discovery_plugins(&self) -> Vec<uptrakit_plugin_infrastructure_core::PluginType> {
            vec![]
        }

        fn validate_package_identifier_str(
            &self,
            _plugin_type: &str,
            _value: &str,
        ) -> std::result::Result<(), String> {
            Ok(())
        }

        fn capabilities_for_str(
            &self,
            _plugin_type: &str,
        ) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
            vec![]
        }

        fn sample_config_for_str(&self, _plugin_type: &str) -> serde_json::Value {
            serde_json::Value::Object(serde_json::Map::new())
        }

        fn config_form_schema_str(
            &self,
            _plugin_type: &str,
        ) -> Option<Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef>> {
            None
        }
    }

    #[test]
    fn validate_config_override_valid_merge() {
        let ops = MockPluginOps;
        let base = serde_json::json!({});
        let override_val = serde_json::json!({
            "tag_strip_prefix": "release-"
        });

        let result = validate_config_override(&ops, "releases_github", &base, &override_val);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_config_override_invalid_merge() {
        let ops = MockPluginOps;
        let base = serde_json::json!({});
        // Override that introduces an invalid api_base_url (http, not https).
        let override_val = serde_json::json!({
            "api_base_url": "http://api.github.com"
        });

        let result = validate_config_override(&ops, "releases_github", &base, &override_val);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_override_non_object_rejected() {
        let ops = MockPluginOps;
        let base = serde_json::json!({});
        let override_val = serde_json::json!("not an object");

        let result = validate_config_override(&ops, "releases_github", &base, &override_val);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigOverrideError::NotAnObject
        ));
    }

    #[test]
    fn validate_execution_site_allows_auto() {
        assert!(validate_execution_site("auto", &PluginRole::DetectVersion).is_ok());
        assert!(validate_execution_site("auto", &PluginRole::FetchReleases).is_ok());
        assert!(validate_execution_site("auto", &PluginRole::ExecuteUpdate).is_ok());
    }

    #[test]
    fn validate_execution_site_allows_agent() {
        assert!(validate_execution_site("agent", &PluginRole::DetectVersion).is_ok());
        assert!(validate_execution_site("agent", &PluginRole::FetchReleases).is_ok());
        assert!(validate_execution_site("agent", &PluginRole::ExecuteUpdate).is_ok());
    }

    #[test]
    fn validate_execution_site_controller_only_for_fetch_releases() {
        assert!(validate_execution_site("controller", &PluginRole::FetchReleases).is_ok());
        assert!(validate_execution_site("controller", &PluginRole::DetectVersion).is_err());
        assert!(validate_execution_site("controller", &PluginRole::ExecuteUpdate).is_err());
    }

    #[test]
    fn validate_execution_site_rejects_invalid() {
        assert!(validate_execution_site("cloud", &PluginRole::DetectVersion).is_err());
        assert!(validate_execution_site("", &PluginRole::FetchReleases).is_err());
        assert!(validate_execution_site("SERVER", &PluginRole::ExecuteUpdate).is_err());
    }
}
