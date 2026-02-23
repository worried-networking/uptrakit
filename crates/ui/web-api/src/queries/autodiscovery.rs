//! Database query helpers for the autodiscovery feature.
//!
//! Covers:
//! - Ignore rule management (create / list / delete)
//! - Pending-item bulk-discard
//! - Auto-creation of default provider configs from discovery `extra` metadata
//! - Processing incoming `DiscoveryResults` payloads (creating pending software
//!   items and upserting host-software-item links)

use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use time::OffsetDateTime;
use uptrakit_internal_wire::{DiscoveryProviderResult, DiscoveryResultsPayload, ProviderType};
use uptrakit_shared_db::entity::{
    autodiscovery_ignore, host_software_item, prelude::*, provider_config, software_item,
};
use uptrakit_shared_db::SoftwareDiscoveryState;
use uptrakit_web_api_types::autodiscovery::{
    AutodiscoveryIgnoreResponse, DiscardDiscoveredResponse,
};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uuid::Uuid;

/// Error returned by autodiscovery query helpers.
#[derive(Debug, thiserror::Error)]
pub enum AutodiscoveryError {
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),
}

// ── Ignore rules ─────────────────────────────────────────────────────────────

/// Insert an autodiscovery ignore rule, silently ignoring duplicates (idempotent).
pub async fn create_or_ignore_ignore_rule(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    provider_config_id: Uuid,
    package_identifier: &str,
) -> Result<(), AutodiscoveryError> {
    // Verify the rule does not already exist to avoid the conflict entirely.
    let exists = AutodiscoveryIgnore::find()
        .filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id))
        .filter(autodiscovery_ignore::Column::ProviderConfigId.eq(provider_config_id))
        .filter(autodiscovery_ignore::Column::PackageIdentifier.eq(package_identifier))
        .count(db)
        .await?;

    if exists > 0 {
        return Ok(());
    }

    let record = autodiscovery_ignore::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        provider_config_id: Set(provider_config_id),
        package_identifier: Set(package_identifier.to_string()),
        created_at: Set(OffsetDateTime::now_utc()),
    };

    AutodiscoveryIgnore::insert(record)
        .exec(db)
        .await
        .map_err(|e| {
            // Suppress unique-constraint violations (race condition between the
            // read above and the insert).
            if is_unique_violation(&e) {
                return AutodiscoveryError::Db(e);
            }
            AutodiscoveryError::Db(e)
        })?;

    Ok(())
}

/// List autodiscovery ignore rules for a tenant, with optional provider-config filter.
pub async fn list_ignore_rules(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    provider_config_id_filter: Option<Uuid>,
    params: &PaginationParams,
) -> Result<PaginatedResponse<AutodiscoveryIgnoreResponse>, AutodiscoveryError> {
    use sea_orm::PaginatorTrait;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 1000);

    let mut query = AutodiscoveryIgnore::find()
        .filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id));

    if let Some(pc_id) = provider_config_id_filter {
        query = query.filter(autodiscovery_ignore::Column::ProviderConfigId.eq(pc_id));
    }

    let paginator = query
        .order_by_desc(autodiscovery_ignore::Column::CreatedAt)
        .paginate(db, per_page);

    let total = paginator.num_items().await?;
    let items_raw = paginator.fetch_page(page - 1).await?;

    // Collect all provider_config IDs we need to join.
    let pc_ids: Vec<Uuid> = items_raw
        .iter()
        .map(|r| r.provider_config_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let configs = ProviderConfig::find()
        .filter(provider_config::Column::Id.is_in(pc_ids))
        .all(db)
        .await?;

    let config_map: std::collections::HashMap<Uuid, provider_config::Model> =
        configs.into_iter().map(|c| (c.id, c)).collect();

    let items = items_raw
        .into_iter()
        .filter_map(|r| {
            let cfg = config_map.get(&r.provider_config_id)?;
            Some(AutodiscoveryIgnoreResponse {
                id: r.id,
                provider_config_id: r.provider_config_id,
                provider_config_name: cfg.name.clone(),
                provider_type: cfg.provider_type.clone(),
                package_identifier: r.package_identifier,
                created_at: r.created_at,
            })
        })
        .collect::<Vec<_>>();

    let total_pages = total.div_ceil(per_page);

    Ok(PaginatedResponse {
        items,
        total,
        page,
        per_page,
        total_pages,
    })
}

/// Hard-delete an autodiscovery ignore rule.
///
/// Returns `true` if a row was deleted, `false` if the rule was not found
/// for this tenant.
pub async fn delete_ignore_rule(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<bool, AutodiscoveryError> {
    let result = AutodiscoveryIgnore::delete_many()
        .filter(autodiscovery_ignore::Column::Id.eq(id))
        .filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id))
        .exec(db)
        .await?;
    Ok(result.rows_affected > 0)
}

// ── Discard pending items ─────────────────────────────────────────────────────

/// Soft-delete all `pending` software items for a tenant, optionally filtered by
/// host or provider config.
///
/// No ignore rules are created — deleted items can be re-discovered later.
pub async fn discard_pending_items(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id_filter: Option<Uuid>,
    provider_config_id_filter: Option<Uuid>,
) -> Result<DiscardDiscoveredResponse, AutodiscoveryError> {
    let now = OffsetDateTime::now_utc();

    // Gather IDs to delete so we can apply host filter via join.
    let mut id_query = SoftwareItem::find()
        .select_only()
        .column(software_item::Column::Id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .filter(software_item::Column::DiscoveryState.eq("pending"));

    if let Some(pc_id) = provider_config_id_filter {
        id_query = id_query.filter(software_item::Column::ProviderConfigId.eq(pc_id));
    }

    // If filtering by host, we need to find items linked to that host.
    let ids: Vec<Uuid> = if let Some(host_id) = host_id_filter {
        let linked_item_ids: Vec<Uuid> = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(db)
            .await?
            .into_iter()
            .map(|l| l.software_item_id)
            .collect();

        if linked_item_ids.is_empty() {
            return Ok(DiscardDiscoveredResponse { discarded_count: 0 });
        }

        id_query = id_query.filter(software_item::Column::Id.is_in(linked_item_ids));

        id_query
            .into_model::<IdRow>()
            .all(db)
            .await?
            .into_iter()
            .map(|r| r.id)
            .collect()
    } else {
        id_query
            .into_model::<IdRow>()
            .all(db)
            .await?
            .into_iter()
            .map(|r| r.id)
            .collect()
    };

    if ids.is_empty() {
        return Ok(DiscardDiscoveredResponse { discarded_count: 0 });
    }

    let count = ids.len() as u32;

    // Soft-delete in bulk.
    SoftwareItem::update_many()
        .col_expr(
            software_item::Column::DeactivatedAt,
            sea_orm::sea_query::Expr::value(Some(now)),
        )
        .filter(software_item::Column::Id.is_in(ids))
        .exec(db)
        .await?;

    Ok(DiscardDiscoveredResponse {
        discarded_count: count,
    })
}

// ── Auto-create default provider configs ─────────────────────────────────────

/// Find or create a provider config with a specific JSON config object.
///
/// Searches active provider configs for this tenant/provider-type whose
/// JSON config equals `config_json`. Creates a new one with `display_name`
/// if none exists. Returns the config ID.
///
/// Idempotent and safe under concurrent calls: the `uq_provider_configs_active_name`
/// partial unique index (`WHERE deactivated_at IS NULL`) guarantees that at most one
/// active config with a given `(tenant_id, name)` pair exists at any time. On a
/// unique-constraint violation (two concurrent auto-creates racing), the function
/// re-queries and returns the winner's ID.
pub async fn find_or_create_default_provider_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    provider_type: &str,
    config_json: &serde_json::Value,
    display_name: &str,
) -> Result<Uuid, AutodiscoveryError> {
    // First pass: search for an existing config with matching JSON.
    if let Some(id) =
        find_matching_provider_config(db, tenant_id, provider_type, config_json).await?
    {
        return Ok(id);
    }

    // None found — try to create one.
    let now = OffsetDateTime::now_utc();
    let new_id = Uuid::now_v7();
    let record = provider_config::ActiveModel {
        id: Set(new_id),
        tenant_id: Set(tenant_id),
        name: Set(display_name.to_string()),
        provider_type: Set(provider_type.to_string()),
        config: Set(config_json.clone()),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    match ProviderConfig::insert(record).exec(db).await {
        Ok(_) => Ok(new_id),
        Err(e) if is_unique_violation(&e) => {
            // A concurrent task created this config at the same time.
            // Re-query to get the winner's ID.
            find_matching_provider_config(db, tenant_id, provider_type, config_json)
                .await?
                .ok_or(AutodiscoveryError::Db(e))
        }
        Err(e) => Err(AutodiscoveryError::Db(e)),
    }
}

/// Query active provider configs for a tenant/type combination and return
/// the ID of the first one whose JSON config matches `config_json`.
async fn find_matching_provider_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    provider_type: &str,
    config_json: &serde_json::Value,
) -> Result<Option<Uuid>, AutodiscoveryError> {
    let configs = ProviderConfig::find()
        .filter(provider_config::Column::TenantId.eq(tenant_id))
        .filter(provider_config::Column::ProviderType.eq(provider_type))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .all(db)
        .await?;

    Ok(configs
        .into_iter()
        .find(|c| &c.config == config_json)
        .map(|c| c.id))
}

// ── Process discovery results ─────────────────────────────────────────────────

/// Process a `DiscoveryResultsPayload` received from an agent.
///
/// For each result:
/// 1. Resolves (or auto-creates) the target `provider_config_id`.
/// 2. Skips items on the ignore list.
/// 3. Upserts `host_software_item` for existing active items.
/// 4. Creates a new `pending` `SoftwareItem` + `HostSoftwareItem` for new discoveries.
pub async fn process_discovery_results(
    db: &sea_orm::DatabaseConnection,
    agent_id: Uuid,
    tenant_id: Uuid,
    host_id: Uuid,
    payload: DiscoveryResultsPayload,
) -> Result<(), AutodiscoveryError> {
    let now = OffsetDateTime::now_utc();

    for result in payload.results {
        if let Some(ref err) = result.error {
            tracing::warn!(
                %agent_id,
                provider_type = %result.provider_type,
                error = %err,
                "discovery provider reported an error, skipping"
            );
            continue;
        }

        if result.discoveries.is_empty() {
            tracing::debug!(
                %agent_id,
                provider_type = %result.provider_type,
                "discovery provider returned no items"
            );
            continue;
        }

        // Resolve or auto-create provider configs, grouped by their "config key"
        // derived from the extra metadata on each discovery item.
        process_provider_result(db, tenant_id, host_id, now, &result).await?;
    }

    Ok(())
}

async fn process_provider_result(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    now: OffsetDateTime,
    result: &DiscoveryProviderResult,
) -> Result<(), AutodiscoveryError> {
    let provider_type_str = result.provider_type.to_string();

    // If the assignment already had a provider_config_id, use it directly for
    // all discoveries in this result. Otherwise, group by config key.
    if let Some(existing_pc_id) = result.provider_config_id {
        for item in &result.discoveries {
            process_one_discovery(
                db,
                tenant_id,
                host_id,
                existing_pc_id,
                &provider_type_str,
                &item.package_identifier,
                &item.name,
                &item.installed_version,
                now,
            )
            .await?;
        }
        return Ok(());
    }

    // Default/auto assignment (no pre-existing config) — group by config key
    // derived from the `extra` metadata on each discovery item.
    match result.provider_type {
        ProviderType::Homebrew => {
            process_homebrew_default(db, tenant_id, host_id, now, result).await?;
        }
        ProviderType::ProxmoxHelperScripts => {
            let config_json = serde_json::json!({});
            let pc_id = find_or_create_default_provider_config(
                db,
                tenant_id,
                &provider_type_str,
                &config_json,
                "Proxmox Helper Scripts",
            )
            .await?;
            for item in &result.discoveries {
                process_one_discovery(
                    db,
                    tenant_id,
                    host_id,
                    pc_id,
                    &provider_type_str,
                    &item.package_identifier,
                    &item.name,
                    &item.installed_version,
                    now,
                )
                .await?;
            }
        }
        _ => {
            tracing::warn!(
                provider_type = %result.provider_type,
                "received discovery results for a provider type that does not support auto-config creation; skipping"
            );
        }
    }

    Ok(())
}

async fn process_homebrew_default(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    now: OffsetDateTime,
    result: &DiscoveryProviderResult,
) -> Result<(), AutodiscoveryError> {
    let provider_type_str = result.provider_type.to_string();

    // Split items by package_type from their extra metadata.
    let mut formulae = Vec::new();
    let mut casks = Vec::new();
    let mut unknown = Vec::new();

    for item in &result.discoveries {
        let pkg_type = item
            .extra
            .as_ref()
            .and_then(|e| e.get("package_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match pkg_type {
            "formula" => formulae.push(item),
            "cask" => casks.push(item),
            _ => {
                tracing::warn!(
                    package_identifier = %item.package_identifier,
                    "Homebrew discovery item missing package_type in extra metadata; skipping"
                );
                unknown.push(item);
            }
        }
    }

    if !formulae.is_empty() {
        let config_json = serde_json::json!({"package_type": "formula"});
        let pc_id = find_or_create_default_provider_config(
            db,
            tenant_id,
            &provider_type_str,
            &config_json,
            "Homebrew (Formulae)",
        )
        .await?;
        for item in formulae {
            process_one_discovery(
                db,
                tenant_id,
                host_id,
                pc_id,
                &provider_type_str,
                &item.package_identifier,
                &item.name,
                &item.installed_version,
                now,
            )
            .await?;
        }
    }

    if !casks.is_empty() {
        let config_json = serde_json::json!({"package_type": "cask"});
        let pc_id = find_or_create_default_provider_config(
            db,
            tenant_id,
            &provider_type_str,
            &config_json,
            "Homebrew (Casks)",
        )
        .await?;
        for item in casks {
            process_one_discovery(
                db,
                tenant_id,
                host_id,
                pc_id,
                &provider_type_str,
                &item.package_identifier,
                &item.name,
                &item.installed_version,
                now,
            )
            .await?;
        }
    }

    let _ = unknown; // already warned above
    Ok(())
}

/// Process a single discovered software item: check ignore list, upsert or create.
#[allow(clippy::too_many_arguments)]
async fn process_one_discovery(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    provider_config_id: Uuid,
    provider_type_str: &str,
    package_identifier: &str,
    name: &str,
    installed_version: &str,
    now: OffsetDateTime,
) -> Result<(), AutodiscoveryError> {
    // 1. Check ignore list.
    let ignored = AutodiscoveryIgnore::find()
        .filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id))
        .filter(autodiscovery_ignore::Column::ProviderConfigId.eq(provider_config_id))
        .filter(autodiscovery_ignore::Column::PackageIdentifier.eq(package_identifier))
        .count(db)
        .await?;

    if ignored > 0 {
        tracing::debug!(
            %provider_config_id,
            %package_identifier,
            "skipping ignored autodiscovery item"
        );
        return Ok(());
    }

    // 2. Find existing active software item.
    let existing = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::ProviderConfigId.eq(provider_config_id))
        .filter(software_item::Column::PackageIdentifier.eq(package_identifier))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(db)
        .await?;

    let software_item_id = if let Some(existing_item) = existing {
        existing_item.id
    } else {
        // 3. Create new pending software item.
        let new_id = Uuid::now_v7();
        let new_item = software_item::ActiveModel {
            id: Set(new_id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            provider_config_id: Set(provider_config_id),
            package_identifier: Set(package_identifier.to_string()),
            config_override: Set(None),
            enabled: Set(false),
            discovery_state: Set(Some(SoftwareDiscoveryState::Pending)),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        SoftwareItem::insert(new_item).exec(db).await?;
        tracing::debug!(
            %package_identifier,
            %provider_type_str,
            "created pending software item from discovery"
        );
        new_id
    };

    // 4. Upsert host_software_item link.
    let existing_link = HostSoftwareItem::find_by_id((host_id, software_item_id))
        .one(db)
        .await?;

    match existing_link {
        Some(link) => {
            let mut active: host_software_item::ActiveModel = link.into();
            active.installed_version = Set(Some(installed_version.to_string()));
            active.installed_version_detected_at = Set(Some(now));
            active.update(db).await?;
        }
        None => {
            let link = host_software_item::ActiveModel {
                host_id: Set(host_id),
                software_item_id: Set(software_item_id),
                installed_version: Set(Some(installed_version.to_string())),
                installed_version_detected_at: Set(Some(now)),
                last_updated_at: Set(None),
                linked_at: Set(now),
            };
            HostSoftwareItem::insert(link).exec(db).await?;
        }
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[derive(Debug, sea_orm::FromQueryResult)]
struct IdRow {
    id: Uuid,
}

fn is_unique_violation(e: &sea_orm::DbErr) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("unique") || msg.contains("duplicate")
}
