//! Database query helpers for the autodiscovery feature.
//!
//! Covers:
//! - Ignore rule management (create / list / delete)
//! - Pending-item bulk-discard
//! - Auto-creation of default plugin configs from discovery targets
//! - Processing incoming `DiscoveryResults` payloads (creating pending software
//!   items and upserting host-software-item links)
//!
//! The controller is completely generic: plugins return structured
//! [`DiscoveryTarget`](uptrakit_shared_types::DiscoveryTarget) values that
//! specify exactly which plugin configs and roles to create — no plugin-specific
//! synthesis logic lives here.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::HashSet;
use time::OffsetDateTime;
use uptrakit_internal_wire::DiscoveryTarget;
use uptrakit_internal_wire::{DiscoveryPluginResult, DiscoveryResultsPayload};
use uptrakit_shared_db::entity::{
    autodiscovery_ignore, host_software_item, host_software_item_plugin, plugin_config, prelude::*,
    software_item,
};
use uptrakit_shared_db::is_unique_constraint_violation;
use uptrakit_shared_types::SoftwareDiscoveryState;
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
    plugin_config_id: Uuid,
    package_identifier: &str,
) -> Result<(), AutodiscoveryError> {
    // Verify the rule does not already exist to avoid the conflict entirely.
    let exists = AutodiscoveryIgnore::find()
        .filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id))
        .filter(autodiscovery_ignore::Column::PluginConfigId.eq(plugin_config_id))
        .filter(autodiscovery_ignore::Column::PackageIdentifier.eq(package_identifier))
        .count(db)
        .await?;

    if exists > 0 {
        return Ok(());
    }

    let record = autodiscovery_ignore::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        plugin_config_id: Set(plugin_config_id),
        package_identifier: Set(package_identifier.to_string()),
        created_at: Set(OffsetDateTime::now_utc()),
    };

    AutodiscoveryIgnore::insert(record)
        .exec(db)
        .await
        .map_err(|e| {
            // Suppress unique-constraint violations (race condition between the
            // read above and the insert).
            if is_unique_constraint_violation(&e) {
                return AutodiscoveryError::Db(e);
            }
            AutodiscoveryError::Db(e)
        })?;

    Ok(())
}

/// List autodiscovery ignore rules for a tenant, with optional plugin-config filter.
pub async fn list_ignore_rules(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id_filter: Option<Uuid>,
    params: &PaginationParams,
) -> Result<PaginatedResponse<AutodiscoveryIgnoreResponse>, AutodiscoveryError> {
    use sea_orm::PaginatorTrait;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 1000);

    let mut query =
        AutodiscoveryIgnore::find().filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id));

    if let Some(pc_id) = plugin_config_id_filter {
        query = query.filter(autodiscovery_ignore::Column::PluginConfigId.eq(pc_id));
    }

    let paginator = query
        .order_by_desc(autodiscovery_ignore::Column::CreatedAt)
        .paginate(db, per_page);

    let total = paginator.num_items().await?;
    let items_raw = paginator.fetch_page(page - 1).await?;

    // Collect all plugin_config IDs we need to join.
    let pc_ids: Vec<Uuid> = items_raw
        .iter()
        .map(|r| r.plugin_config_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let configs = PluginConfig::find()
        .filter(plugin_config::Column::Id.is_in(pc_ids))
        .all(db)
        .await?;

    let config_map: std::collections::HashMap<Uuid, plugin_config::Model> =
        configs.into_iter().map(|c| (c.id, c)).collect();

    let items = items_raw
        .into_iter()
        .filter_map(|r| {
            let cfg = config_map.get(&r.plugin_config_id)?;
            Some(AutodiscoveryIgnoreResponse {
                id: r.id,
                plugin_config_id: r.plugin_config_id,
                plugin_config_name: cfg.name.clone(),
                plugin_type: cfg.plugin_type.clone(),
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
/// host or plugin config.
///
/// No ignore rules are created — deleted items can be re-discovered later.
/// All `host_software_items` links for the discarded items are hard-deleted so that
/// subsequent discovery runs treat those packages as new discoveries rather than
/// silently refreshing the version on an orphaned link.
pub async fn discard_pending_items(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id_filter: Option<Uuid>,
    plugin_config_id_filter: Option<Uuid>,
) -> Result<DiscardDiscoveredResponse, AutodiscoveryError> {
    let now = OffsetDateTime::now_utc();

    // Gather candidate pending software item IDs for this tenant.
    let mut id_query = SoftwareItem::find()
        .select_only()
        .column(software_item::Column::Id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .filter(software_item::Column::DiscoveryState.eq("pending"));

    // If a plugin config filter is requested, restrict to items that have at
    // least one host_software_item_plugin row with that plugin config ID.
    let plugin_filtered_item_ids: Option<Vec<Uuid>> = if let Some(pc_id) = plugin_config_id_filter {
        let linked: Vec<Uuid> = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::PluginConfigId.eq(pc_id))
            .all(db)
            .await?
            .into_iter()
            .map(|l| l.software_item_id)
            .collect();

        if linked.is_empty() {
            return Ok(DiscardDiscoveredResponse { discarded_count: 0 });
        }
        Some(linked)
    } else {
        None
    };

    if let Some(ref pfids) = plugin_filtered_item_ids {
        id_query = id_query.filter(software_item::Column::Id.is_in(pfids.clone()));
    }

    // If filtering by host, find items linked to that host.
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

    // Remove host-software-item links for all discarded items.  Without this,
    // the orphaned rows would cause subsequent discovery runs to find the link
    // in Phase 1 and return early — silently refreshing the version on a
    // deactivated item instead of surfacing a new pending entry.
    HostSoftwareItem::delete_many()
        .filter(host_software_item::Column::SoftwareItemId.is_in(ids.clone()))
        .exec(db)
        .await?;

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

// ── Auto-create default plugin configs ───────────────────────────────────────

/// Find or create a plugin config matched by `(plugin_type, name)`.
///
/// Lookup order:
///
/// 1. **Name match, JSON match** — returns the existing ID unchanged.
/// 2. **Name match, JSON differs** — updates the config JSON in-place and
///    returns the same ID. This is the self-healing path for plugin updates
///    that change default command templates (e.g. adding `sudo` to a PHS
///    update command after commit `8695cbc`): existing role assignments that
///    reference the config by ID automatically pick up the new command on the
///    next discovery run without requiring manual re-linking.
/// 3. **No match** — creates a new config row.
///
/// Idempotent and safe under concurrent calls: the `uq_plugin_configs_active_name`
/// partial unique index (`WHERE deactivated_at IS NULL`) guarantees that at most one
/// active config with a given `(tenant_id, name)` pair exists at any time. On a
/// unique-constraint violation (two concurrent auto-creates racing), the function
/// re-queries by name and returns the winner's ID.
pub async fn find_or_create_default_plugin_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_type: &str,
    config_json: &serde_json::Value,
    display_name: &str,
) -> Result<Uuid, AutodiscoveryError> {
    // Search by the natural identity key: (tenant_id, plugin_type, name).
    // Matching on name — rather than JSON content — means that when a plugin
    // update rewrites a default command template, the existing row is updated
    // in-place so all current role assignments automatically pick up the change.
    let existing = PluginConfig::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::PluginType.eq(plugin_type))
        .filter(plugin_config::Column::Name.eq(display_name))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await?;

    if let Some(cfg) = existing {
        let id = cfg.id;
        if &cfg.config == config_json {
            // Config is already up-to-date.
            return Ok(id);
        }
        // Config JSON has changed. Update in-place so existing role assignments
        // referencing this ID continue to work with the new configuration.
        let now = OffsetDateTime::now_utc();
        let mut active: plugin_config::ActiveModel = cfg.into();
        active.config = Set(config_json.clone());
        active.updated_at = Set(now);
        active.update(db).await?;
        tracing::debug!(
            %id,
            plugin_type = %plugin_type,
            name = %display_name,
            "updated auto-generated plugin config to reflect new defaults"
        );
        return Ok(id);
    }

    // None found — try to create one.
    let now = OffsetDateTime::now_utc();
    let new_id = Uuid::now_v7();
    let record = plugin_config::ActiveModel {
        id: Set(new_id),
        tenant_id: Set(tenant_id),
        name: Set(display_name.to_string()),
        plugin_type: Set(plugin_type.to_string()),
        config: Set(config_json.clone()),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    match PluginConfig::insert(record).exec(db).await {
        Ok(_) => Ok(new_id),
        Err(e) if is_unique_constraint_violation(&e) => {
            // A concurrent task created this config at the same time.
            // Re-query by name to get the winner's ID.
            PluginConfig::find()
                .filter(plugin_config::Column::TenantId.eq(tenant_id))
                .filter(plugin_config::Column::PluginType.eq(plugin_type))
                .filter(plugin_config::Column::Name.eq(display_name))
                .filter(plugin_config::Column::DeactivatedAt.is_null())
                .one(db)
                .await?
                .map(|c| c.id)
                .ok_or(AutodiscoveryError::Db(e))
        }
        Err(e) => Err(AutodiscoveryError::Db(e)),
    }
}

// ── Process discovery results ─────────────────────────────────────────────────

/// Process a `DiscoveryResultsPayload` received from an agent.
///
/// For each plugin result, delegates to one of two generic processing paths:
///
/// 1. **Target-based**: Items with non-empty `targets` are processed via
///    [`process_targets_discovery`] — each target drives plugin-config
///    find-or-create and role-assignment creation.
/// 2. **Config-ID-based**: Items with empty `targets` use the pre-existing
///    `plugin_config_id` from the result for all three standard roles.
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
                plugin_type = %result.plugin_type,
                error = %err,
                "discovery plugin reported an error, skipping"
            );
            continue;
        }

        if result.discoveries.is_empty() {
            tracing::debug!(
                %agent_id,
                plugin_type = %result.plugin_type,
                "discovery plugin returned no items"
            );
            continue;
        }

        process_plugin_result(db, tenant_id, host_id, now, &result).await?;
    }

    Ok(())
}

/// Process a single plugin's discovery results.
///
/// Routes each item to the correct processing path based on its `targets` field
/// and the result's `plugin_config_id`. This function is fully generic — no
/// plugin-type-specific branching.
async fn process_plugin_result(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    now: OffsetDateTime,
    result: &DiscoveryPluginResult,
) -> Result<(), AutodiscoveryError> {
    for item in &result.discoveries {
        let item_info = DiscoveredItemInfo {
            package_identifier: &item.package_identifier,
            name: &item.name,
            installed_version: &item.installed_version,
        };
        if !item.targets.is_empty() {
            // Target-based: each target specifies its own plugin config and roles.
            process_targets_discovery(db, tenant_id, host_id, &item_info, &item.targets, now)
                .await?;
        } else if let Some(existing_pc_id) = result.plugin_config_id {
            // Config-ID-based: use the pre-existing plugin config for all roles.
            let ignore_set = load_ignore_set(db, tenant_id, existing_pc_id).await?;
            process_one_discovery(
                db,
                tenant_id,
                host_id,
                existing_pc_id,
                item_info,
                &ignore_set,
                now,
            )
            .await?;
        } else {
            tracing::warn!(
                plugin_type = %result.plugin_type,
                package_identifier = %item.package_identifier,
                "discovery item has no targets and no plugin_config_id; skipping"
            );
        }
    }

    Ok(())
}

/// Grouped arguments for a discovered item's identity fields.
struct DiscoveredItemInfo<'a> {
    package_identifier: &'a str,
    name: &'a str,
    installed_version: &'a str,
}

/// Process a discovered item that carries explicit `DiscoveryTarget` values.
///
/// For each target:
/// 1. Find-or-create a plugin config matching the target's type and JSON config.
/// 2. Check the ignore list.
/// 3. Upsert or create the software item and host link.
/// 4. Create role assignments per the target's `roles` list.
async fn process_targets_discovery(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    item: &DiscoveredItemInfo<'_>,
    targets: &[DiscoveryTarget],
    now: OffsetDateTime,
) -> Result<(), AutodiscoveryError> {
    for target in targets {
        let target_plugin_type_str = target.plugin_type.to_string();

        // Use the target's package_identifier override, or fall back to the item's.
        let pkg_id = target
            .package_identifier
            .as_deref()
            .unwrap_or(item.package_identifier);

        let execution_site = target.execution_site.as_deref().unwrap_or("auto");

        // Find-or-create plugin config for this target.
        let pc_id = find_or_create_default_plugin_config(
            db,
            tenant_id,
            &target_plugin_type_str,
            &target.plugin_config,
            &target.plugin_config_name,
        )
        .await?;

        let ignore_set = load_ignore_set(db, tenant_id, pc_id).await?;

        // Check ignore list.
        if ignore_set.contains(pkg_id) {
            tracing::debug!(
                %pc_id,
                package_identifier = %pkg_id,
                "skipping ignored autodiscovery item"
            );
            continue;
        }

        // Build target-specific item info (may override package_identifier).
        let target_item = DiscoveredItemInfo {
            package_identifier: pkg_id,
            name: item.name,
            installed_version: item.installed_version,
        };

        // Find-or-create the software item and host link.
        let software_item_id =
            find_or_create_software_item(db, tenant_id, host_id, pc_id, &target_item, now).await?;

        // If None, the item already existed and was updated in-place.
        let Some(software_item_id) = software_item_id else {
            continue;
        };

        // Create role assignments from the target's role list.
        for role in &target.roles {
            let plugin_link = host_software_item_plugin::ActiveModel {
                id: Set(Uuid::now_v7()),
                host_id: Set(host_id),
                software_item_id: Set(software_item_id),
                plugin_config_id: Set(pc_id),
                role: Set(role.as_str().to_string()),
                ordinal: Set(0),
                package_identifier: Set(pkg_id.to_string()),
                config_override: Set(target.config_override.clone()),
                execution_site: Set(execution_site.to_owned()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            if let Err(e) = HostSoftwareItemPlugin::insert(plugin_link).exec(db).await
                && !is_unique_constraint_violation(&e)
            {
                return Err(e.into());
            }
        }
    }

    Ok(())
}

/// Pre-load the ignore set for a specific `(tenant_id, plugin_config_id)` pair.
///
/// Returns a `HashSet` of `package_identifier` strings that should be skipped.
/// Scoped per config to keep the set bounded.
async fn load_ignore_set(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<HashSet<String>, AutodiscoveryError> {
    let rules = AutodiscoveryIgnore::find()
        .filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id))
        .filter(autodiscovery_ignore::Column::PluginConfigId.eq(plugin_config_id))
        .all(db)
        .await?;
    Ok(rules.into_iter().map(|r| r.package_identifier).collect())
}

/// Find-or-create a software item + host link. Returns the software_item_id if a
/// new link was created (caller must then create role assignments), or `None` if
/// the existing link was updated in-place.
///
/// Three-phase lookup:
/// 1. If this host already has a `host_software_item_plugin` row for
///    `(plugin_config_id, package_identifier)`, update `installed_version` in place
///    **if** the linked `software_item` is still active. If the item has been
///    discarded (soft-deleted), the orphaned link is removed and the function falls
///    through to phases 2/3 so a fresh pending item is created.
/// 2. If *any other* host in the tenant has the same assignment backed by an active
///    software item, reuse it and insert a new `host_software_item` link for this host.
/// 3. Otherwise create a new pending `software_item` and `host_software_item`.
///    If the insert hits a `(tenant_id, name)` unique-constraint violation (e.g.
///    a second `DiscoveryTarget` for the same `DiscoveredSoftware` item races
///    through Phase 3 first), fall back to a `(tenant_id, name)` lookup so both
///    targets end up sharing the same `software_item` row.
async fn find_or_create_software_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    plugin_config_id: Uuid,
    item: &DiscoveredItemInfo<'_>,
    now: OffsetDateTime,
) -> Result<Option<Uuid>, AutodiscoveryError> {
    let package_identifier = item.package_identifier;
    let name = item.name;
    let installed_version = item.installed_version;
    // Phase 1: Check if this specific host already tracks (plugin_config_id, package_identifier)
    let existing_plugin_link = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::PluginConfigId.eq(plugin_config_id))
        .filter(host_software_item_plugin::Column::PackageIdentifier.eq(package_identifier))
        .one(db)
        .await?;

    if let Some(plugin_link) = existing_plugin_link {
        let linked_item_active = SoftwareItem::find()
            .filter(software_item::Column::Id.eq(plugin_link.software_item_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(db)
            .await?
            .is_some();

        if linked_item_active {
            // Just refresh the installed version on the parent host_software_item row.
            let hsi = HostSoftwareItem::find_by_id((host_id, plugin_link.software_item_id))
                .one(db)
                .await?;
            if let Some(hsi) = hsi {
                let mut active: host_software_item::ActiveModel = hsi.into();
                active.installed_version = Set(Some(installed_version.to_string()));
                active.installed_version_detected_at = Set(Some(now));
                active.update(db).await?;
            }
            return Ok(None);
        }

        // The linked software item was discarded; remove the orphaned link.
        tracing::debug!(
            %plugin_config_id,
            package_identifier = %package_identifier,
            "removing orphaned host link for discarded software item; will re-discover"
        );
        if let Some(hsi) = HostSoftwareItem::find_by_id((host_id, plugin_link.software_item_id))
            .one(db)
            .await?
        {
            let hsi_active: host_software_item::ActiveModel = hsi.into();
            hsi_active.delete(db).await?;
        }
        // Fall through to phases 2/3.
    }

    // Phase 2: Check if any other host in this tenant already has
    // (plugin_config_id, package_identifier).
    let candidate_links: Vec<Uuid> = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::PluginConfigId.eq(plugin_config_id))
        .filter(host_software_item_plugin::Column::PackageIdentifier.eq(package_identifier))
        .all(db)
        .await?
        .into_iter()
        .map(|l| l.software_item_id)
        .collect();

    let existing_item = if candidate_links.is_empty() {
        None
    } else {
        SoftwareItem::find()
            .filter(software_item::Column::Id.is_in(candidate_links))
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(db)
            .await?
    };

    let software_item_id = if let Some(item) = existing_item {
        item.id
    } else {
        // Phase 3: Create a new pending software item.
        let new_id = Uuid::now_v7();
        let new_item = software_item::ActiveModel {
            id: Set(new_id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            enabled: Set(false),
            discovery_state: Set(Some(SoftwareDiscoveryState::Pending)),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        match SoftwareItem::insert(new_item).exec(db).await {
            Ok(_) => {
                tracing::debug!(
                    package_identifier = %package_identifier,
                    "created pending software item from discovery"
                );
                new_id
            }
            Err(e) if is_unique_constraint_violation(&e) => {
                // Another target for the same DiscoveredSoftware (or a concurrent
                // request) already created a software_item with this name.
                tracing::debug!(
                    package_identifier = %package_identifier,
                    name = %name,
                    "software_item name collision on insert; reusing existing item"
                );
                SoftwareItem::find()
                    .filter(software_item::Column::TenantId.eq(tenant_id))
                    .filter(software_item::Column::Name.eq(name))
                    .filter(software_item::Column::DeactivatedAt.is_null())
                    .one(db)
                    .await?
                    .ok_or_else(|| {
                        AutodiscoveryError::Db(sea_orm::DbErr::RecordNotFound(format!(
                            "software_item with name '{name}' not found after collision"
                        )))
                    })?
                    .id
            }
            Err(e) => return Err(e.into()),
        }
    };

    // Insert host_software_item link.
    let link = host_software_item::ActiveModel {
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        installed_version: Set(Some(installed_version.to_string())),
        installed_version_detected_at: Set(Some(now)),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
    };
    if let Err(e) = HostSoftwareItem::insert(link).exec(db).await
        && !is_unique_constraint_violation(&e)
    {
        return Err(e.into());
    }

    Ok(Some(software_item_id))
}

/// Process a single discovered software item using the config-ID path.
///
/// Used when items have no targets and the enclosing result has a pre-existing
/// `plugin_config_id`. Creates all three standard role assignments.
async fn process_one_discovery(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    plugin_config_id: Uuid,
    args: DiscoveredItemInfo<'_>,
    ignore_set: &HashSet<String>,
    now: OffsetDateTime,
) -> Result<(), AutodiscoveryError> {
    // Check ignore list (O(1) lookup into pre-loaded set).
    if ignore_set.contains(args.package_identifier) {
        tracing::debug!(
            %plugin_config_id,
            package_identifier = %args.package_identifier,
            "skipping ignored autodiscovery item"
        );
        return Ok(());
    }

    let software_item_id =
        find_or_create_software_item(db, tenant_id, host_id, plugin_config_id, &args, now).await?;

    // If None, the item already existed and was updated in-place.
    let Some(software_item_id) = software_item_id else {
        return Ok(());
    };

    // Create role plugin assignments for all three standard roles.
    for role in ["detect_version", "fetch_releases", "execute_update"] {
        let plugin_link = host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            plugin_config_id: Set(plugin_config_id),
            role: Set(role.to_string()),
            ordinal: Set(0),
            package_identifier: Set(args.package_identifier.to_string()),
            config_override: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        if let Err(e) = HostSoftwareItemPlugin::insert(plugin_link).exec(db).await
            && !is_unique_constraint_violation(&e)
        {
            return Err(e.into());
        }
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[derive(Debug, sea_orm::FromQueryResult)]
struct IdRow {
    id: Uuid,
}


#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use uptrakit_internal_wire::{
        DiscoveredSoftware as WireDiscoveredSoftware, DiscoveryPluginResult, DiscoveryTarget,
        PluginRole, PluginType,
    };
    use uptrakit_shared_db::entity::{host, plugin_config, tenant};
    use uptrakit_shared_types::SoftwareDiscoveryState;

    async fn setup_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    // ── FK-setup helpers ──────────────────────────────────────────────────────

    async fn insert_tenant(db: &DatabaseConnection, id: Uuid) {
        let now = time::OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set("Test Tenant".to_string()),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
    }

    async fn insert_host(db: &DatabaseConnection, id: Uuid, tenant_id: Uuid) {
        let now = time::OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            machine_id: Set(id.to_string()),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("Test Host".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host");
    }

    async fn insert_plugin_config(db: &DatabaseConnection, id: Uuid, tenant_id: Uuid) {
        let now = time::OffsetDateTime::now_utc();
        plugin_config::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(format!("Test Plugin Config {id}")),
            plugin_type: Set("package_manager_homebrew".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert plugin_config");
    }

    // ── query helpers ─────────────────────────────────────────────────────────

    async fn insert_software_item(
        db: &DatabaseConnection,
        id: Uuid,
        tenant_id: Uuid,
        name: &str,
        deactivated_at: Option<time::OffsetDateTime>,
    ) {
        let now = time::OffsetDateTime::now_utc();
        let model = software_item::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            enabled: Set(false),
            discovery_state: Set(Some(SoftwareDiscoveryState::Pending)),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(deactivated_at),
        };
        SoftwareItem::insert(model)
            .exec(db)
            .await
            .expect("insert software_item");
    }

    async fn insert_host_link(
        db: &DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
        package_identifier: &str,
    ) {
        let now = time::OffsetDateTime::now_utc();
        let link = host_software_item::ActiveModel {
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(Some(now)),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
        };
        HostSoftwareItem::insert(link)
            .exec(db)
            .await
            .expect("insert host_software_item");

        // Also create plugin link rows for all three roles to match the new schema.
        for role in ["detect_version", "fetch_releases", "execute_update"] {
            let plugin_link = host_software_item_plugin::ActiveModel {
                id: Set(Uuid::now_v7()),
                host_id: Set(host_id),
                software_item_id: Set(software_item_id),
                plugin_config_id: Set(plugin_config_id),
                role: Set(role.to_string()),
                ordinal: Set(0),
                package_identifier: Set(package_identifier.to_string()),
                config_override: Set(None),
                execution_site: Set("auto".to_string()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            HostSoftwareItemPlugin::insert(plugin_link)
                .exec(db)
                .await
                .expect("insert host_software_item_plugin");
        }
    }

    // ── Helper: make DiscoveryPluginResult with targets ───────────────────────

    fn all_roles() -> Vec<PluginRole> {
        vec![
            PluginRole::DetectVersion,
            PluginRole::FetchReleases,
            PluginRole::ExecuteUpdate,
        ]
    }

    fn phs_result_with_github_target(
        pkg_id: &str,
        name: &str,
        version: &str,
        owner: &str,
        repo: &str,
    ) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: name.to_string(),
                installed_version: version.to_string(),
                targets: vec![DiscoveryTarget {
                    plugin_type: PluginType::ReleasesGithub,
                    plugin_config: serde_json::json!({
                        "owner": owner,
                        "repo": repo,
                        "tag_strip_prefix": "v",
                        "include_prereleases": false,
                        "asset_patterns": [],
                        "detect_installed_version_command":
                            r#"cat -- "${HOME}/.{package_identifier}""#,
                        "install_command": "env PHS_SILENT=1 /usr/bin/update",
                    }),
                    plugin_config_name: format!("{owner}/{repo}"),
                    roles: all_roles(),
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }],
                extra: None,
            }],
        }
    }

    fn phs_result_with_apt_target(
        pkg_id: &str,
        name: &str,
        version: &str,
    ) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: name.to_string(),
                installed_version: version.to_string(),
                targets: vec![DiscoveryTarget {
                    plugin_type: PluginType::PackageManagerApt,
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "APT (auto)".to_string(),
                    roles: all_roles(),
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }],
                extra: None,
            }],
        }
    }

    fn phs_result_no_targets(pkg_id: &str) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: pkg_id.to_string(),
                installed_version: "1.0.0".to_string(),
                targets: vec![],
                extra: None,
            }],
        }
    }

    /// Mirrors the *actual* PHS plugin output for a GitHub-managed LXC container:
    /// - Target 1: `ReleasesGithub`, `FetchReleases` only,
    ///   `package_identifier = Some("owner/repo")`
    /// - Target 2: `GenericShell`, `[DetectVersion, ExecuteUpdate]`,
    ///   `package_identifier = None` (falls back to `pkg_id`)
    fn phs_result_with_two_targets(
        pkg_id: &str,
        name: &str,
        version: &str,
        owner: &str,
        repo: &str,
    ) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: name.to_string(),
                installed_version: version.to_string(),
                targets: vec![
                    DiscoveryTarget {
                        plugin_type: PluginType::ReleasesGithub,
                        plugin_config: serde_json::json!({
                            "tag_strip_prefix": "v",
                            "include_prereleases": false,
                            "asset_patterns": [],
                        }),
                        plugin_config_name: "GitHub Releases".to_string(),
                        roles: vec![PluginRole::FetchReleases],
                        package_identifier: Some(format!("{owner}/{repo}")),
                        config_override: None,
                        execution_site: None,
                    },
                    DiscoveryTarget {
                        plugin_type: PluginType::GenericShell,
                        plugin_config: serde_json::json!({
                            "version_command": "phs-app --version",
                            "update_command": "phs-update",
                        }),
                        plugin_config_name: "PHS Shell".to_string(),
                        roles: vec![PluginRole::DetectVersion, PluginRole::ExecuteUpdate],
                        package_identifier: None,
                        config_override: None,
                        execution_site: None,
                    },
                ],
                extra: None,
            }],
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    /// `discard_pending_items` must remove the `host_software_items` rows for
    /// every discarded software item.
    #[tokio::test]
    async fn discard_pending_items_removes_host_links() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, item_id, tenant_id, "git", None).await;
        insert_host_link(&db, host_id, item_id, pc_id, "git").await;

        let links_before = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .count(&db)
            .await
            .expect("count before");
        assert_eq!(links_before, 1, "expected one host link before discard");

        let result = discard_pending_items(&db, tenant_id, None, None)
            .await
            .expect("discard");
        assert_eq!(result.discarded_count, 1);

        let links_after = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .count(&db)
            .await
            .expect("count after");
        assert_eq!(
            links_after, 0,
            "host link must be deleted when software item is discarded"
        );
    }

    /// When `find_or_create_software_item` encounters a host link pointing to a
    /// deactivated software item, it must delete the orphaned link and create a
    /// fresh pending item.
    #[tokio::test]
    async fn process_one_discovery_orphaned_link_creates_new_pending() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let old_item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, old_item_id, tenant_id, "curl", Some(now)).await;
        insert_host_link(&db, host_id, old_item_id, pc_id, "curl").await;

        let args = DiscoveredItemInfo {
            package_identifier: "curl",
            name: "curl",
            installed_version: "8.0.0",
        };
        let ignore_set = HashSet::new();

        process_one_discovery(&db, tenant_id, host_id, pc_id, args, &ignore_set, now)
            .await
            .expect("process_one_discovery");

        let orphan_count = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(old_item_id))
            .count(&db)
            .await
            .expect("orphan count");
        assert_eq!(orphan_count, 0, "orphaned host link must be deleted");

        let active_items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .all(&db)
            .await
            .expect("active items");
        assert_eq!(
            active_items.len(),
            1,
            "expected exactly one new pending item"
        );
        assert_eq!(
            active_items[0].discovery_state,
            Some(SoftwareDiscoveryState::Pending)
        );

        let new_link_count = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PluginConfigId.eq(pc_id))
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("curl"))
            .count(&db)
            .await
            .expect("new link count");
        assert_eq!(
            new_link_count, 3,
            "expected plugin link rows for all three roles"
        );
    }

    /// `process_one_discovery` must update `installed_version` in place when the
    /// existing host link points to an active software item.
    #[tokio::test]
    async fn process_one_discovery_active_link_updates_version() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, item_id, tenant_id, "wget", None).await;
        insert_host_link(&db, host_id, item_id, pc_id, "wget").await;

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
        };
        let ignore_set = HashSet::new();

        process_one_discovery(&db, tenant_id, host_id, pc_id, args, &ignore_set, now)
            .await
            .expect("process_one_discovery");

        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .all(&db)
            .await
            .expect("items");
        assert_eq!(items.len(), 1, "no new item should be created");
        assert_eq!(items[0].id, item_id, "the original item must be retained");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("link query")
            .expect("link must exist");
        assert_eq!(
            link.installed_version.as_deref(),
            Some("2.0.0"),
            "installed_version must be updated to the new value"
        );
    }

    // ── Target-based processing tests ─────────────────────────────────────────

    /// A GitHub PHS item (with target) must create a `github_releases` plugin config.
    #[tokio::test]
    async fn target_based_github_creates_plugin_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result =
            phs_result_with_github_target("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");

        process_plugin_result(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_plugin_result");

        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("releases_github"))
            .all(&db)
            .await
            .expect("query configs");
        assert_eq!(configs.len(), 1, "expected one github_releases config");
        assert_eq!(configs[0].name, "BookLore/BookLore");

        let cfg_json = &configs[0].config;
        assert_eq!(cfg_json["owner"], "BookLore");
        assert_eq!(cfg_json["repo"], "BookLore");

        let hsi_links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query hsi links");
        assert_eq!(hsi_links.len(), 1, "expected one host_software_item");
        assert_eq!(hsi_links[0].installed_version.as_deref(), Some("1.18.5"));

        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("booklore"))
            .all(&db)
            .await
            .expect("query plugin links");
        assert_eq!(
            plugin_links.len(),
            3,
            "expected three role-based plugin links"
        );
    }

    /// An APT PHS item (with target) must create/reuse a shared `apt` plugin config.
    #[tokio::test]
    async fn target_based_apt_creates_apt_plugin_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result = phs_result_with_apt_target("grafana", "Grafana", "10.2.3");

        process_plugin_result(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_plugin_result");

        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package_manager_apt"))
            .all(&db)
            .await
            .expect("query configs");
        assert_eq!(configs.len(), 1, "expected one apt config");
        assert_eq!(configs[0].name, "APT (auto)");

        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("grafana"))
            .all(&db)
            .await
            .expect("query plugin links");
        assert_eq!(
            plugin_links.len(),
            3,
            "expected three role-based plugin links"
        );

        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query hsi");
        assert_eq!(hsi.len(), 1, "expected one host_software_item");
        assert_eq!(hsi[0].installed_version.as_deref(), Some("10.2.3"));
    }

    /// An item with no targets and no plugin_config_id must be skipped.
    #[tokio::test]
    async fn no_targets_no_config_id_skips_item() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        let result = phs_result_no_targets("n8n");

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        process_plugin_result(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_plugin_result");

        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query configs");
        assert!(
            configs.is_empty(),
            "no config should be created for skipped item"
        );

        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query items");
        assert!(items.is_empty(), "no software item should be created");
    }

    /// Two hosts discovering the same GitHub PHS app must share one plugin config.
    #[tokio::test]
    async fn target_based_two_hosts_share_github_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host1 = Uuid::now_v7();
        let host2 = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host1, tenant_id).await;
        insert_host(&db, host2, tenant_id).await;

        let result1 =
            phs_result_with_github_target("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");
        let result2 =
            phs_result_with_github_target("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");

        process_plugin_result(&db, tenant_id, host1, now, &result1)
            .await
            .expect("host1");
        process_plugin_result(&db, tenant_id, host2, now, &result2)
            .await
            .expect("host2");

        let config_count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("releases_github"))
            .count(&db)
            .await
            .expect("count configs");
        assert_eq!(
            config_count, 1,
            "both hosts must share a single plugin config"
        );

        let host1_links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host1))
            .count(&db)
            .await
            .expect("host1 links");
        let host2_links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host2))
            .count(&db)
            .await
            .expect("host2 links");
        assert_eq!(host1_links, 1);
        assert_eq!(host2_links, 1);
    }

    /// Config-ID-based path: items with empty targets and a pre-existing
    /// plugin_config_id must use that config for all roles.
    #[tokio::test]
    async fn config_id_path_uses_existing_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        let result = DiscoveryPluginResult {
            plugin_type: PluginType::PackageManagerHomebrew,
            plugin_config_id: Some(pc_id),
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: "wget".to_string(),
                name: "Wget".to_string(),
                installed_version: "1.21.4".to_string(),
                targets: vec![],
                extra: None,
            }],
        };

        process_plugin_result(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_plugin_result");

        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PluginConfigId.eq(pc_id))
            .all(&db)
            .await
            .expect("query plugin links");
        assert_eq!(
            plugin_links.len(),
            3,
            "expected three role-based plugin links"
        );

        for link in &plugin_links {
            assert_eq!(link.package_identifier, "wget");
        }
    }

    /// Target-based ignore rules work correctly: items on the ignore list
    /// for a target's plugin config are skipped.
    #[tokio::test]
    async fn target_based_ignore_rules_work() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        // First, create the apt config by processing a normal item.
        let result1 = phs_result_with_apt_target("curl", "cURL", "8.0.0");
        process_plugin_result(&db, tenant_id, host_id, now, &result1)
            .await
            .expect("first process");

        // Find the auto-created apt config.
        let apt_config = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package_manager_apt"))
            .one(&db)
            .await
            .expect("query")
            .expect("apt config must exist");

        // Add "wget" to the ignore list for that config.
        create_or_ignore_ignore_rule(&db, tenant_id, apt_config.id, "wget")
            .await
            .expect("create ignore rule");

        // Now try to discover "wget" via the same apt target path.
        let result2 = phs_result_with_apt_target("wget", "Wget", "1.21.4");
        process_plugin_result(&db, tenant_id, host_id, now, &result2)
            .await
            .expect("second process");

        // wget must NOT have been created.
        let wget_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("wget"))
            .count(&db)
            .await
            .expect("count");
        assert_eq!(wget_links, 0, "ignored item must not be created");
    }

    /// Regression test: a PHS item with both a GitHub `FetchReleases` target and
    /// a Shell `[DetectVersion, ExecuteUpdate]` target must produce exactly one
    /// `software_items` row, one `host_software_items` row, two `plugin_configs`,
    /// and three `host_software_item_plugins` rows (one per role).
    ///
    /// This exercises the Phase 3 name-collision fallback introduced to fix the
    /// `(tenant_id, name)` unique-index violation that previously aborted the
    /// entire processing loop when the Shell target tried to insert a
    /// `software_items` row whose name already existed from the GitHub target.
    #[tokio::test]
    async fn target_based_phs_two_targets_share_one_software_item() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result =
            phs_result_with_two_targets("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");

        process_plugin_result(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_plugin_result must not fail on name collision");

        // Exactly one software_items row.
        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .all(&db)
            .await
            .expect("query software_items");
        assert_eq!(items.len(), 1, "expected exactly one software_items row");
        assert_eq!(items[0].name, "BookLore");

        // Exactly one host_software_items link.
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query host_software_items");
        assert_eq!(
            hsi.len(),
            1,
            "expected exactly one host_software_items link"
        );

        // Exactly two plugin_configs: releases_github + generic_shell.
        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query plugin_configs");
        assert_eq!(configs.len(), 2, "expected two plugin_configs");
        let config_types: std::collections::HashSet<String> =
            configs.iter().map(|c| c.plugin_type.clone()).collect();
        assert!(
            config_types.contains("releases_github"),
            "expected a releases_github config"
        );
        assert!(
            config_types.contains("generic_shell"),
            "expected a generic_shell config"
        );

        // Exactly three host_software_item_plugins rows.
        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query host_software_item_plugins");
        assert_eq!(
            plugin_links.len(),
            3,
            "expected three host_software_item_plugins rows"
        );

        // Validate individual role assignments.
        let github_config_id = configs
            .iter()
            .find(|c| c.plugin_type == "releases_github")
            .unwrap()
            .id;
        let shell_config_id = configs
            .iter()
            .find(|c| c.plugin_type == "generic_shell")
            .unwrap()
            .id;

        let fetch = plugin_links
            .iter()
            .find(|l| l.role == "fetch_releases")
            .expect("fetch_releases role must exist");
        assert_eq!(fetch.plugin_config_id, github_config_id);
        assert_eq!(fetch.package_identifier, "BookLore/BookLore");

        let detect = plugin_links
            .iter()
            .find(|l| l.role == "detect_version")
            .expect("detect_version role must exist");
        assert_eq!(detect.plugin_config_id, shell_config_id);
        assert_eq!(detect.package_identifier, "booklore");

        let update = plugin_links
            .iter()
            .find(|l| l.role == "execute_update")
            .expect("execute_update role must exist");
        assert_eq!(update.plugin_config_id, shell_config_id);
        assert_eq!(update.package_identifier, "booklore");
    }

    /// When a plugin config with the same `(plugin_type, name)` already exists
    /// but with different JSON, `find_or_create_default_plugin_config` must
    /// update the config in-place and return the original ID.
    ///
    /// This is the self-healing mechanism for the case where a plugin update
    /// changes default command templates — e.g. `8695cbc` rewrote the PHS
    /// update command from `"env PHS_SILENT=1 /usr/bin/update"` (runs without
    /// root and fails) to `"sudo /usr/local/bin/uptrakit-phs-update"`. Without
    /// this in-place update, existing role assignments would keep pointing to
    /// the old config ID and continue executing the broken command.
    #[tokio::test]
    async fn find_or_create_updates_config_json_on_name_match() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let old_config = serde_json::json!({
            "update_command": "env PHS_SILENT=1 /usr/bin/update",
        });
        let new_config = serde_json::json!({
            "update_command": "sudo /usr/local/bin/uptrakit-phs-update",
        });

        // Create the config with the old JSON.
        let first_id = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "generic_shell",
            &old_config,
            "PHS Shell",
        )
        .await
        .expect("create first");

        // Call again with the same name but updated JSON.
        let second_id = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "generic_shell",
            &new_config,
            "PHS Shell",
        )
        .await
        .expect("update in-place");

        // Must return the same ID — no new row.
        assert_eq!(
            first_id, second_id,
            "must return the existing config ID, not create a new one"
        );

        // The stored config must reflect the new JSON.
        let stored = PluginConfig::find()
            .filter(plugin_config::Column::Id.eq(first_id))
            .one(&db)
            .await
            .expect("query config")
            .expect("config must still exist");
        assert_eq!(
            stored.config, new_config,
            "config JSON must be updated in-place"
        );

        // Exactly one active config with this name must exist.
        let count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::Name.eq("PHS Shell"))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .count(&db)
            .await
            .expect("count");
        assert_eq!(count, 1, "must not create a duplicate config");
    }

    /// Calling `find_or_create_default_plugin_config` twice with identical
    /// `(name, JSON)` must return the same ID and leave exactly one row.
    #[tokio::test]
    async fn find_or_create_is_idempotent_when_json_unchanged() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let config = serde_json::json!({"tag_strip_prefix": "v"});

        let id1 = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases_github",
            &config,
            "GitHub Releases",
        )
        .await
        .expect("first call");

        let id2 = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases_github",
            &config,
            "GitHub Releases",
        )
        .await
        .expect("second call");

        assert_eq!(id1, id2, "must return the same ID on repeated calls");

        let count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::Name.eq("GitHub Releases"))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .count(&db)
            .await
            .expect("count");
        assert_eq!(count, 1, "must not create duplicate rows");
    }
}
