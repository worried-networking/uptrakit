use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, FromQueryResult, ModelTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, RelationTrait, Set, TransactionTrait,
};
use std::collections::HashMap;
use time::OffsetDateTime;
use uptrakit_provider_registry::ProviderRegistry;
use uptrakit_shared_db::entity::{host, host_software_item, prelude::*, provider_config, software_item};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, HostSoftwareAssignment,
    ListSoftwareItemsParams, SoftwareItemDetailResponse, SoftwareItemHostSummary,
    SoftwareItemResponse, UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
};
use uuid::Uuid;

use crate::auth::token::generate_uuid;
use crate::queries::provider_configs::{find_raw_active_config_txn, validate_hooks_internal};
use crate::tenant_db::TenantDb;

/// Errors returned by software item mutation queries.
#[derive(Debug)]
pub enum SoftwareItemQueryError {
    /// Software item not found or deactivated.
    NotFound,
    /// Name must not be empty (for update).
    EmptyName,
    /// A software item with the same name already exists.
    DuplicateItem,
    /// A host in the request was not found or is deactivated.
    HostNotFound(Uuid),
    /// The referenced provider config does not exist or is inactive.
    ProviderConfigNotFound,
    /// A (host_id, provider_config_id, package_identifier) combo is already tracked.
    DuplicateHostAssignment,
    /// Package identifier failed validation (e.g. Homebrew naming rules).
    InvalidPackageIdentifier(String),
    /// `config_override` failed provider-level or hook validation.
    InvalidConfigOverride(String),
    /// Inline provider config failed name/config/hook validation.
    InvalidInlineProviderConfig(String),
    /// A database error occurred.
    Db(sea_orm::DbErr),
}

#[derive(Debug, FromQueryResult)]
struct ItemHostCount {
    software_item_id: Uuid,
    count: i64,
}

#[derive(Debug, FromQueryResult)]
struct ItemProviderType {
    software_item_id: Uuid,
    provider_type: String,
}

// --- Private helpers ---

fn build_list_response(
    item: &software_item::Model,
    provider_types: Vec<String>,
    host_count: u64,
) -> SoftwareItemResponse {
    SoftwareItemResponse {
        id: item.id,
        name: item.name.clone(),
        provider_types,
        enabled: item.enabled,
        discovery_state: item.discovery_state.clone(),
        last_checked_at: item.last_checked_at,
        host_count,
        created_at: item.created_at,
        updated_at: item.updated_at,
    }
}

fn build_detail_response(
    item: software_item::Model,
    provider_types: Vec<String>,
    host_count: u64,
    hosts: Vec<SoftwareItemHostSummary>,
) -> SoftwareItemDetailResponse {
    SoftwareItemDetailResponse {
        id: item.id,
        name: item.name,
        provider_types,
        enabled: item.enabled,
        discovery_state: item.discovery_state,
        last_checked_at: item.last_checked_at,
        host_count,
        created_at: item.created_at,
        updated_at: item.updated_at,
        hosts,
    }
}

async fn count_linked_hosts(db: &sea_orm::DatabaseConnection, item_id: Uuid) -> u64 {
    HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .count(db)
        .await
        .unwrap_or(0)
}

/// Load the distinct provider types for a software item from its host assignments.
async fn load_provider_types(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
) -> Vec<String> {
    #[derive(Debug, FromQueryResult)]
    struct PcRow {
        provider_type: String,
    }

    // Join host_software_items → provider_configs to collect distinct provider types.
    let rows: Vec<PcRow> = HostSoftwareItem::find()
        .select_only()
        .column(provider_config::Column::ProviderType)
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::ProviderConfig.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .into_model::<PcRow>()
        .all(db)
        .await
        .unwrap_or_default();

    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|r| seen.insert(r.provider_type.clone()))
        .map(|r| r.provider_type)
        .collect()
}

async fn load_item_hosts(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
) -> Vec<SoftwareItemHostSummary> {
    let links = match HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .all(db)
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::warn!("Failed to load software item hosts: {e}");
            return Vec::new();
        }
    };

    if links.is_empty() {
        return Vec::new();
    }

    let host_ids: Vec<Uuid> = links.iter().map(|l| l.host_id).collect();
    let pc_ids: Vec<Uuid> = links.iter().map(|l| l.provider_config_id).collect();

    let hosts: HashMap<Uuid, host::Model> = match Host::find()
        .filter(host::Column::Id.is_in(host_ids))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(db)
        .await
    {
        Ok(h) => h.into_iter().map(|h| (h.id, h)).collect(),
        Err(e) => {
            tracing::warn!("Failed to load hosts for software item: {e}");
            return Vec::new();
        }
    };

    let provider_configs: HashMap<Uuid, provider_config::Model> = match ProviderConfig::find()
        .filter(provider_config::Column::Id.is_in(pc_ids))
        .all(db)
        .await
    {
        Ok(pcs) => pcs.into_iter().map(|pc| (pc.id, pc)).collect(),
        Err(e) => {
            tracing::warn!("Failed to load provider configs for software item: {e}");
            return Vec::new();
        }
    };

    links
        .into_iter()
        .filter_map(|link| {
            let host = hosts.get(&link.host_id)?;
            let pc = provider_configs.get(&link.provider_config_id)?;
            Some(SoftwareItemHostSummary {
                host_id: host.id,
                hostname: host.hostname.clone(),
                friendly_name: host.friendly_name.clone(),
                provider_config_id: pc.id,
                provider_config_name: pc.name.clone(),
                provider_type: pc.provider_type.clone(),
                package_identifier: link.package_identifier,
                config_override: link.config_override,
                installed_version: link.installed_version,
                installed_version_detected_at: link.installed_version_detected_at,
                last_updated_at: link.last_updated_at,
                linked_at: link.linked_at,
            })
        })
        .collect()
}

/// Find a non-deactivated software item by ID, scoped to a tenant.
pub(crate) async fn find_active_item(
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
    #[error("provider validation failed: {0}")]
    ProviderValidation(String),
}

/// Validate `config_override` by merging it with the base provider config and running
/// provider-specific validation. The merged document must satisfy the provider's schema.
fn validate_config_override(
    provider_type: &str,
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

    ProviderRegistry::validate_config_str(provider_type, &merged)
        .map_err(|e| ConfigOverrideError::ProviderValidation(e.to_string()))
}

/// Resolve provider config from either an existing ID or an inline create request,
/// within a transaction. Returns `(provider_config_id, provider_config::Model)`.
async fn resolve_provider_config_txn(
    txn: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    assignment: &HostSoftwareAssignment,
) -> Result<(Uuid, provider_config::Model), SoftwareItemQueryError> {
    match (&assignment.provider_config_id, &assignment.provider_config) {
        (Some(pcid), None) => {
            let pcid = *pcid;
            let c = find_raw_active_config_txn(txn, tenant_id, pcid)
                .await
                .ok_or(SoftwareItemQueryError::ProviderConfigNotFound)?;
            Ok((pcid, c))
        }
        (None, Some(inline)) => {
            if inline.name.is_empty() {
                return Err(SoftwareItemQueryError::InvalidInlineProviderConfig(
                    "name must not be empty".to_string(),
                ));
            }
            if let Err(e) =
                ProviderRegistry::validate_config_str(inline.provider_type.as_str(), &inline.config)
            {
                return Err(SoftwareItemQueryError::InvalidInlineProviderConfig(
                    e.to_string(),
                ));
            }
            if let Err(e) = validate_hooks_internal(&inline.config) {
                return Err(SoftwareItemQueryError::InvalidInlineProviderConfig(
                    e.to_string(),
                ));
            }
            let now = OffsetDateTime::now_utc();
            let pcid = generate_uuid();
            let model = provider_config::ActiveModel {
                id: Set(pcid),
                tenant_id: Set(tenant_id),
                name: Set(inline.name.clone()),
                provider_type: Set(inline.provider_type.to_string()),
                config: Set(inline.config.clone()),
                enabled: Set(inline.enabled),
                created_at: Set(now),
                updated_at: Set(now),
                deactivated_at: Set(None),
            };
            let inserted = model.insert(txn).await.map_err(SoftwareItemQueryError::Db)?;
            Ok((pcid, inserted))
        }
        _ => Err(SoftwareItemQueryError::ProviderConfigNotFound),
    }
}

/// Validate provider config, package identifier, and config_override for a host assignment.
fn validate_assignment(
    config: &provider_config::Model,
    package_identifier: &str,
    config_override: Option<&serde_json::Value>,
) -> Result<(), SoftwareItemQueryError> {
    if let Ok(pt) = config.provider_type.parse::<uptrakit_provider_registry::ProviderType>()
        && let Err(e) = ProviderRegistry::validate_package_identifier(pt, package_identifier)
    {
        return Err(SoftwareItemQueryError::InvalidPackageIdentifier(e));
    }

    if let Some(override_val) = config_override {
        if let Err(e) = validate_config_override(&config.provider_type, &config.config, override_val)
        {
            return Err(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
        }
        if let Err(e) = validate_hooks_internal(override_val) {
            return Err(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
        }
    }

    Ok(())
}

// --- Public query functions ---

/// Create a new software item (catalog entry only). Check unique name constraint.
pub async fn create_software_item(
    tenant_db: &TenantDb,
    req: CreateSoftwareItemRequest,
) -> Result<SoftwareItemResponse, SoftwareItemQueryError> {
    let txn = tenant_db
        .db()
        .begin()
        .await
        .map_err(SoftwareItemQueryError::Db)?;

    // Check uniqueness: name must be unique among active items for this tenant.
    let duplicate = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::Name.eq(&req.name))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .map_err(SoftwareItemQueryError::Db)?;

    if duplicate.is_some() {
        return Err(SoftwareItemQueryError::DuplicateItem);
    }

    let now = OffsetDateTime::now_utc();
    let model = software_item::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name),
        enabled: Set(req.enabled),
        discovery_state: Set(None),
        last_checked_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let inserted = model.insert(&txn).await.map_err(SoftwareItemQueryError::Db)?;

    txn.commit().await.map_err(SoftwareItemQueryError::Db)?;

    Ok(build_list_response(&inserted, vec![], 0))
}

pub async fn list_software_items(
    tenant_db: &TenantDb,
    params: &ListSoftwareItemsParams,
) -> Result<PaginatedResponse<SoftwareItemResponse>, sea_orm::DbErr> {
    use sea_orm::sea_query::Expr;

    let pagination = params.pagination().resolve();

    let mut base_query = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .order_by_asc(software_item::Column::Name);

    if let Some(state) = &params.discovery_state {
        base_query = base_query.filter(software_item::Column::DiscoveryState.eq(state.clone()));
    }

    let total = base_query.clone().count(tenant_db.db()).await?;

    let items = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    if items.is_empty() {
        return Ok(PaginatedResponse::new(vec![], total, pagination));
    }

    let item_ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();

    // Bulk-load host counts for all items in one GROUP BY query.
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
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.clone()))
        .group_by(host_software_item::Column::SoftwareItemId)
        .into_model::<ItemHostCount>()
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|row| (row.software_item_id, row.count as u64))
        .collect();

    // Bulk-load provider types for all items via JOIN (one query).
    let provider_type_rows: Vec<ItemProviderType> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::SoftwareItemId)
        .column(provider_config::Column::ProviderType)
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::ProviderConfig.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids))
        .into_model::<ItemProviderType>()
        .all(tenant_db.db())
        .await?;

    // Group provider types by software item id, deduplicated.
    let mut provider_types_map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in provider_type_rows {
        let entry = provider_types_map.entry(row.software_item_id).or_default();
        if !entry.contains(&row.provider_type) {
            entry.push(row.provider_type);
        }
    }

    let response: Vec<SoftwareItemResponse> = items
        .iter()
        .map(|item| {
            let provider_types = provider_types_map
                .remove(&item.id)
                .unwrap_or_default();
            let host_count = host_counts.get(&item.id).copied().unwrap_or(0);
            build_list_response(item, provider_types, host_count)
        })
        .collect();

    Ok(PaginatedResponse::new(response, total, pagination))
}

/// Returns `None` if not found.
pub async fn get_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<SoftwareItemDetailResponse>, sea_orm::DbErr> {
    let Some(item) = find_active_item(tenant_db.db(), tenant_db.tenant_id, id).await else {
        return Ok(None);
    };

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let provider_types = load_provider_types(tenant_db.db(), id).await;
    Ok(Some(build_detail_response(item, provider_types, host_count, hosts)))
}

/// Partial update — only `name` and `enabled` are updatable.
/// Returns `Err(NotFound)` if the item does not exist or is deactivated.
pub async fn update_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
    req: UpdateSoftwareItemRequest,
) -> Result<SoftwareItemResponse, SoftwareItemQueryError> {
    let existing = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or(SoftwareItemQueryError::NotFound)?;

    if let Some(ref name) = req.name
        && name.is_empty()
    {
        return Err(SoftwareItemQueryError::EmptyName);
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
            .map_err(SoftwareItemQueryError::Db)?;

        if duplicate.is_some() {
            return Err(SoftwareItemQueryError::DuplicateItem);
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    if let Some(enabled) = req.enabled {
        model.enabled = Set(enabled);
    }
    model.updated_at = Set(now);

    let updated = model.update(tenant_db.db()).await.map_err(SoftwareItemQueryError::Db)?;
    let provider_types = load_provider_types(tenant_db.db(), id).await;
    let host_count = count_linked_hosts(tenant_db.db(), id).await;
    Ok(build_list_response(&updated, provider_types, host_count))
}

/// Soft-delete a software item. Returns `true` if deleted, `false` if not found.
pub async fn delete_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let Some(item) = find_active_item(tenant_db.db(), tenant_db.tenant_id, id).await else {
        return Ok(false);
    };

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = item.into();
    model.deactivated_at = Set(Some(now));
    model.enabled = Set(false);
    model.updated_at = Set(now);
    model.update(tenant_db.db()).await?;
    Ok(true)
}

/// Assign hosts to a software item. Each host carries its own provider info.
/// Returns the updated detail response, or an error if the item or a host is not found.
pub async fn assign_hosts(
    tenant_db: &TenantDb,
    id: Uuid,
    req: AssignHostsRequest,
) -> Result<SoftwareItemDetailResponse, SoftwareItemQueryError> {
    find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or(SoftwareItemQueryError::NotFound)?;

    let txn = tenant_db
        .db()
        .begin()
        .await
        .map_err(SoftwareItemQueryError::Db)?;

    let now = OffsetDateTime::now_utc();

    for assignment in &req.host_assignments {
        let host_id = assignment.host_id;

        let host_exists = Host::find_by_id(host_id)
            .filter(host::Column::DeactivatedAt.is_null())
            .one(&txn)
            .await
            .map_err(SoftwareItemQueryError::Db)?;

        if host_exists.is_none() {
            return Err(SoftwareItemQueryError::HostNotFound(host_id));
        }

        let (provider_config_id, config) =
            resolve_provider_config_txn(&txn, tenant_db.tenant_id, assignment).await?;

        let package_identifier = assignment.package_identifier.as_deref().unwrap_or("");

        validate_assignment(
            &config,
            package_identifier,
            assignment.config_override.as_ref(),
        )?;

        // Check global uniqueness of (host_id, provider_config_id, package_identifier).
        // This prevents the same provider+package appearing under two different software items.
        let global_conflict = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::ProviderConfigId.eq(provider_config_id))
            .filter(host_software_item::Column::PackageIdentifier.eq(package_identifier))
            .filter(host_software_item::Column::SoftwareItemId.ne(id))
            .one(&txn)
            .await
            .map_err(SoftwareItemQueryError::Db)?;

        if global_conflict.is_some() {
            return Err(SoftwareItemQueryError::DuplicateHostAssignment);
        }

        let existing_link = HostSoftwareItem::find_by_id((host_id, id))
            .one(&txn)
            .await
            .map_err(SoftwareItemQueryError::Db)?;

        match existing_link {
            Some(link) => {
                // Update provider info on existing link.
                let mut active: host_software_item::ActiveModel = link.into();
                active.provider_config_id = Set(provider_config_id);
                active.package_identifier = Set(package_identifier.to_string());
                active.config_override = Set(assignment.config_override.clone());
                active.update(&txn).await.map_err(SoftwareItemQueryError::Db)?;
            }
            None => {
                let link = host_software_item::ActiveModel {
                    host_id: Set(host_id),
                    software_item_id: Set(id),
                    provider_config_id: Set(provider_config_id),
                    package_identifier: Set(package_identifier.to_string()),
                    config_override: Set(assignment.config_override.clone()),
                    installed_version: Set(None),
                    installed_version_detected_at: Set(None),
                    last_updated_at: Set(None),
                    linked_at: Set(now),
                };
                link.insert(&txn).await.map_err(SoftwareItemQueryError::Db)?;
            }
        }
    }

    txn.commit().await.map_err(SoftwareItemQueryError::Db)?;

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or(SoftwareItemQueryError::NotFound)?;

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let provider_types = load_provider_types(tenant_db.db(), id).await;
    Ok(build_detail_response(item, provider_types, host_count, hosts))
}

/// Update the provider info for an existing host–software-item assignment.
pub async fn update_host_assignment(
    tenant_db: &TenantDb,
    id: Uuid,
    host_id: Uuid,
    req: UpdateHostAssignmentRequest,
) -> Result<SoftwareItemDetailResponse, SoftwareItemQueryError> {
    find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or(SoftwareItemQueryError::NotFound)?;

    let link = HostSoftwareItem::find_by_id((host_id, id))
        .one(tenant_db.db())
        .await
        .map_err(SoftwareItemQueryError::Db)?
        .ok_or(SoftwareItemQueryError::NotFound)?;

    // Build a synthetic assignment struct so we can reuse resolve_provider_config_txn.
    let synthetic = uptrakit_web_api_types::software_items::HostSoftwareAssignment {
        host_id,
        provider_config_id: req.provider_config_id.or(Some(link.provider_config_id)),
        provider_config: req.provider_config,
        package_identifier: req.package_identifier.clone().or(Some(link.package_identifier.clone())),
        config_override: req.config_override.clone().or(link.config_override.clone()),
    };

    let txn = tenant_db
        .db()
        .begin()
        .await
        .map_err(SoftwareItemQueryError::Db)?;

    let (provider_config_id, config) =
        resolve_provider_config_txn(&txn, tenant_db.tenant_id, &synthetic).await?;

    let package_identifier = synthetic.package_identifier.as_deref().unwrap_or("");

    validate_assignment(&config, package_identifier, synthetic.config_override.as_ref())?;

    // Check for conflicts in other software items.
    let global_conflict = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::ProviderConfigId.eq(provider_config_id))
        .filter(host_software_item::Column::PackageIdentifier.eq(package_identifier))
        .filter(host_software_item::Column::SoftwareItemId.ne(id))
        .one(&txn)
        .await
        .map_err(SoftwareItemQueryError::Db)?;

    if global_conflict.is_some() {
        return Err(SoftwareItemQueryError::DuplicateHostAssignment);
    }

    let mut active: host_software_item::ActiveModel = link.into();
    active.provider_config_id = Set(provider_config_id);
    active.package_identifier = Set(package_identifier.to_string());

    // Handle config_override: explicit null in request clears it.
    if let Some(ref override_val) = req.config_override {
        if override_val.is_null() {
            active.config_override = Set(None);
        } else {
            active.config_override = Set(Some(override_val.clone()));
        }
    }

    active.update(&txn).await.map_err(SoftwareItemQueryError::Db)?;

    txn.commit().await.map_err(SoftwareItemQueryError::Db)?;

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or(SoftwareItemQueryError::NotFound)?;

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let provider_types = load_provider_types(tenant_db.db(), id).await;
    Ok(build_detail_response(item, provider_types, host_count, hosts))
}

/// Unassign a host from a software item.
/// Returns `true` if removed, `false` if the software item or link was not found.
pub async fn unassign_host(
    tenant_db: &TenantDb,
    id: Uuid,
    host_id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    if find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .is_none()
    {
        return Ok(false);
    }

    let link = HostSoftwareItem::find_by_id((host_id, id))
        .one(tenant_db.db())
        .await?;

    match link {
        Some(l) => {
            l.delete(tenant_db.db()).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Load the provider config ID and package identifier for a specific host assignment.
/// Used by route handlers to resolve provider info for version checks and updates.
pub(crate) async fn load_host_assignment(
    db: &sea_orm::DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
) -> Option<host_software_item::Model> {
    HostSoftwareItem::find_by_id((host_id, software_item_id))
        .one(db)
        .await
        .ok()
        .flatten()
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
            enabled: true,
            discovery_state: None,
            last_checked_at: Some(now),
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(&item, vec!["github_releases".to_string()], 3);

        assert_eq!(resp.name, "Node.js");
        assert_eq!(resp.provider_types, vec!["github_releases"]);
        assert_eq!(resp.host_count, 3);
        assert!(resp.last_checked_at.is_some());
    }

    #[test]
    fn build_detail_response_includes_hosts() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Redis".to_string(),
            enabled: true,
            discovery_state: None,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let hosts = vec![SoftwareItemHostSummary {
            host_id: uuid::Uuid::now_v7(),
            hostname: "web-01".to_string(),
            friendly_name: "Web Server 1".to_string(),
            provider_config_id: uuid::Uuid::now_v7(),
            provider_config_name: "GitHub Releases".to_string(),
            provider_type: "github_releases".to_string(),
            package_identifier: "redis/redis".to_string(),
            config_override: Some(serde_json::json!({"asset_patterns": ["redis.*linux"]})),
            installed_version: Some("7.2.4".to_string()),
            installed_version_detected_at: Some(now),
            last_updated_at: None,
            linked_at: now,
        }];

        let resp = build_detail_response(item, vec!["github_releases".to_string()], 1, hosts);

        assert_eq!(resp.name, "Redis");
        assert_eq!(resp.provider_types, vec!["github_releases"]);
        assert_eq!(resp.hosts.len(), 1);
        assert_eq!(resp.hosts[0].hostname, "web-01");
        assert_eq!(resp.hosts[0].package_identifier, "redis/redis");
        assert_eq!(resp.hosts[0].installed_version, Some("7.2.4".to_string()));
    }

    #[test]
    fn build_list_response_null_last_checked_at() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            enabled: false,
            discovery_state: None,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(&item, vec![], 0);

        assert!(!resp.enabled);
        assert!(resp.last_checked_at.is_none());
        assert_eq!(resp.host_count, 0);
        assert!(resp.provider_types.is_empty());
    }

    #[test]
    fn validate_config_override_valid_merge() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let override_val = serde_json::json!({
            "tag_strip_prefix": "release-"
        });

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_config_override_invalid_merge() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        // Override that clears a required field.
        let override_val = serde_json::json!({
            "owner": ""
        });

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_override_non_object_rejected() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let override_val = serde_json::json!("not an object");

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigOverrideError::NotAnObject
        ));
    }

    #[test]
    fn validate_homebrew_package_identifier_accepts_valid() {
        use uptrakit_provider_registry::ProviderType;
        let cases = [
            "wget",
            "node@18",
            "homebrew/cask/firefox",
            "custom-tap/tool",
            "pkg.name",
            "pkg_name",
            "pkg+name",
        ];

        for case in cases {
            assert!(
                ProviderRegistry::validate_package_identifier(ProviderType::Homebrew, case).is_ok(),
                "expected valid: {case}"
            );
        }
    }

    #[test]
    fn validate_homebrew_package_identifier_rejects_invalid() {
        use uptrakit_provider_registry::ProviderType;
        let cases = [
            "",
            " ",
            " leading",
            "trailing ",
            "has space",
            "tap//pkg",
            "tap/../pkg",
            "tap/./pkg",
            "pkg$",
        ];

        for case in cases {
            assert!(
                ProviderRegistry::validate_package_identifier(ProviderType::Homebrew, case).is_err(),
                "expected invalid: {case}"
            );
        }
    }
}
