//! Database query helpers for the autodiscovery feature.
//!
//! Covers:
//! - Ignore rule management (create / list / delete)
//! - Pending-item bulk-discard
//! - Auto-creation of default provider configs from discovery `extra` metadata
//! - Processing incoming `DiscoveryResults` payloads (creating pending software
//!   items and upserting host-software-item links)

// ── PHS synthesis constants ───────────────────────────────────────────────────

/// Shell command used to detect the installed version of a GitHub-managed PHS app.
///
/// `{package_identifier}` is the PHS slug, replaced shell-escaped at runtime by
/// the GitHub provider's `detect_installed_version()` implementation.
const PHS_DETECT_VERSION_CMD: &str = r#"cat -- "${HOME}/.{package_identifier}""#;

/// Install command for PHS-managed apps.
///
/// Uses the unattended mode (`PHS_SILENT=1`) exactly as the official
/// `update-apps.sh` PVE tool does via `pct exec`, so the update runs without
/// interactive prompts and without requiring a network fetch of the script.
const PHS_INSTALL_CMD: &str = "env PHS_SILENT=1 /usr/bin/update";

use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::HashSet;
use time::OffsetDateTime;
use uptrakit_internal_wire::{DiscoveryProviderResult, DiscoveryResultsPayload, PluginType};
use uptrakit_shared_db::SoftwareDiscoveryState;
use uptrakit_shared_db::entity::{
    autodiscovery_ignore, host_software_item, prelude::*, provider_config, software_item,
};
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

    let mut query =
        AutodiscoveryIgnore::find().filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id));

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
/// All `host_software_items` links for the discarded items are hard-deleted so that
/// subsequent discovery runs treat those packages as new discoveries rather than
/// silently refreshing the version on an orphaned link.
pub async fn discard_pending_items(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id_filter: Option<Uuid>,
    provider_config_id_filter: Option<Uuid>,
) -> Result<DiscardDiscoveredResponse, AutodiscoveryError> {
    let now = OffsetDateTime::now_utc();

    // Gather candidate pending software item IDs for this tenant.
    let mut id_query = SoftwareItem::find()
        .select_only()
        .column(software_item::Column::Id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .filter(software_item::Column::DiscoveryState.eq("pending"));

    // If a provider config filter is requested, restrict to items that have at
    // least one host_software_items row with that provider config ID.
    let provider_filtered_item_ids: Option<Vec<Uuid>> =
        if let Some(pc_id) = provider_config_id_filter {
            let linked: Vec<Uuid> = HostSoftwareItem::find()
                .filter(host_software_item::Column::ProviderConfigId.eq(pc_id))
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

    if let Some(ref pfids) = provider_filtered_item_ids {
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
        let ignore_set = load_ignore_set(db, tenant_id, existing_pc_id).await?;
        for item in &result.discoveries {
            let args = ProcessDiscoveryArgs {
                package_identifier: &item.package_identifier,
                name: &item.name,
                installed_version: &item.installed_version,
                provider_type_str: &provider_type_str,
            };
            process_one_discovery(
                db,
                tenant_id,
                host_id,
                existing_pc_id,
                args,
                &ignore_set,
                now,
            )
            .await?;
        }
        return Ok(());
    }

    // Default/auto assignment (no pre-existing config) — group by config key
    // derived from the `extra` metadata on each discovery item.
    match result.provider_type {
        PluginType::Homebrew => {
            process_homebrew_default(db, tenant_id, host_id, now, result).await?;
        }
        PluginType::Docker => {
            let config_json = serde_json::json!({});
            let pc_id = find_or_create_default_provider_config(
                db,
                tenant_id,
                &provider_type_str,
                &config_json,
                "Docker",
            )
            .await?;
            let ignore_set = load_ignore_set(db, tenant_id, pc_id).await?;
            for item in &result.discoveries {
                let args = ProcessDiscoveryArgs {
                    package_identifier: &item.package_identifier,
                    name: &item.name,
                    installed_version: &item.installed_version,
                    provider_type_str: &provider_type_str,
                };
                process_one_discovery(db, tenant_id, host_id, pc_id, args, &ignore_set, now)
                    .await?;
            }
        }
        PluginType::ProxmoxHelperScripts => {
            process_phs_results(db, tenant_id, host_id, now, result).await?;
        }
        PluginType::Apt => {
            let config_json = serde_json::json!({});
            let pc_id = find_or_create_default_provider_config(
                db,
                tenant_id,
                &provider_type_str,
                &config_json,
                "APT",
            )
            .await?;
            let ignore_set = load_ignore_set(db, tenant_id, pc_id).await?;
            for item in &result.discoveries {
                let args = ProcessDiscoveryArgs {
                    package_identifier: &item.package_identifier,
                    name: &item.name,
                    installed_version: &item.installed_version,
                    provider_type_str: &provider_type_str,
                };
                process_one_discovery(db, tenant_id, host_id, pc_id, args, &ignore_set, now)
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

/// Process PHS discovery results by dispatching on the `extra` metadata set by
/// the PHS provider.
///
/// Each discovered item carries one of:
/// - `{ "github_owner": "…", "github_repo": "…" }` — GitHub-managed app.
///   A `github_releases` provider config is found-or-created per `(owner, repo)`
///   pair, pre-populated with the PHS detect-version command and install command.
/// - `{ "apt_package": "…" }` — APT-managed app (direct or install-script fallback).
///   A shared `apt` provider config (`{}`) is found-or-created for the tenant.
/// - Neither — logged as a warning and skipped.
async fn process_phs_results(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    now: OffsetDateTime,
    result: &DiscoveryProviderResult,
) -> Result<(), AutodiscoveryError> {
    for item in &result.discoveries {
        let github_owner = item
            .extra
            .as_ref()
            .and_then(|e| e.get("github_owner"))
            .and_then(|v| v.as_str());
        let github_repo = item
            .extra
            .as_ref()
            .and_then(|e| e.get("github_repo"))
            .and_then(|v| v.as_str());
        let apt_package = item
            .extra
            .as_ref()
            .and_then(|e| e.get("apt_package"))
            .and_then(|v| v.as_str());

        match (github_owner, github_repo, apt_package) {
            (Some(owner), Some(repo), _) => {
                // GitHub-managed: synthesize a GithubReleases provider config.
                let config_json = serde_json::json!({
                    "owner": owner,
                    "repo": repo,
                    "tag_strip_prefix": "v",
                    "include_prereleases": false,
                    "asset_patterns": [],
                    "detect_installed_version_command": PHS_DETECT_VERSION_CMD,
                    "install_command": PHS_INSTALL_CMD,
                });
                let display_name = format!("{owner}/{repo}");
                let pc_id = find_or_create_default_provider_config(
                    db,
                    tenant_id,
                    "github_releases",
                    &config_json,
                    &display_name,
                )
                .await?;
                let ignore_set = load_ignore_set(db, tenant_id, pc_id).await?;
                let args = ProcessDiscoveryArgs {
                    package_identifier: &item.package_identifier,
                    name: &item.name,
                    installed_version: &item.installed_version,
                    provider_type_str: "github_releases",
                };
                process_one_discovery(db, tenant_id, host_id, pc_id, args, &ignore_set, now)
                    .await?;
            }
            (_, _, Some(_apt_pkg)) => {
                // APT-managed: find-or-create the shared default APT provider config.
                let config_json = serde_json::json!({});
                let pc_id = find_or_create_default_provider_config(
                    db,
                    tenant_id,
                    "apt",
                    &config_json,
                    "APT (auto)",
                )
                .await?;
                let ignore_set = load_ignore_set(db, tenant_id, pc_id).await?;
                let args = ProcessDiscoveryArgs {
                    package_identifier: &item.package_identifier,
                    name: &item.name,
                    installed_version: &item.installed_version,
                    provider_type_str: "apt",
                };
                process_one_discovery(db, tenant_id, host_id, pc_id, args, &ignore_set, now)
                    .await?;
            }
            _ => {
                tracing::warn!(
                    package_identifier = %item.package_identifier,
                    "PHS item has no detectable upstream; skipping"
                );
            }
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
        let ignore_set = load_ignore_set(db, tenant_id, pc_id).await?;
        for item in formulae {
            let args = ProcessDiscoveryArgs {
                package_identifier: &item.package_identifier,
                name: &item.name,
                installed_version: &item.installed_version,
                provider_type_str: &provider_type_str,
            };
            process_one_discovery(db, tenant_id, host_id, pc_id, args, &ignore_set, now).await?;
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
        let ignore_set = load_ignore_set(db, tenant_id, pc_id).await?;
        for item in casks {
            let args = ProcessDiscoveryArgs {
                package_identifier: &item.package_identifier,
                name: &item.name,
                installed_version: &item.installed_version,
                provider_type_str: &provider_type_str,
            };
            process_one_discovery(db, tenant_id, host_id, pc_id, args, &ignore_set, now).await?;
        }
    }

    let _ = unknown; // already warned above
    Ok(())
}

/// Grouped arguments for a single discovered software item.
struct ProcessDiscoveryArgs<'a> {
    package_identifier: &'a str,
    name: &'a str,
    installed_version: &'a str,
    provider_type_str: &'a str,
}

/// Pre-load the ignore set for a specific `(tenant_id, provider_config_id)` pair.
///
/// Returns a `HashSet` of `package_identifier` strings that should be skipped.
/// Scoped per config to keep the set bounded.
async fn load_ignore_set(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    provider_config_id: Uuid,
) -> Result<HashSet<String>, AutodiscoveryError> {
    let rules = AutodiscoveryIgnore::find()
        .filter(autodiscovery_ignore::Column::TenantId.eq(tenant_id))
        .filter(autodiscovery_ignore::Column::ProviderConfigId.eq(provider_config_id))
        .all(db)
        .await?;
    Ok(rules.into_iter().map(|r| r.package_identifier).collect())
}

/// Process a single discovered software item: check ignore list, upsert or create.
///
/// Three-phase lookup:
/// 1. If this host already has a `host_software_item` row for
///    `(provider_config_id, package_identifier)`, update `installed_version` in place
///    **if** the linked `software_item` is still active.  If the item has been
///    discarded (soft-deleted), the orphaned link is removed and the function falls
///    through to phases 2/3 so a fresh pending item is created.
/// 2. If *any other* host in the tenant has the same assignment backed by an active
///    software item, reuse it and insert a new `host_software_item` link for this host.
/// 3. Otherwise create a new pending `software_item` (name only) and a new
///    `host_software_item` with the provider info.
async fn process_one_discovery(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    provider_config_id: Uuid,
    args: ProcessDiscoveryArgs<'_>,
    ignore_set: &HashSet<String>,
    now: OffsetDateTime,
) -> Result<(), AutodiscoveryError> {
    // 1. Check ignore list (O(1) lookup into pre-loaded set).
    if ignore_set.contains(args.package_identifier) {
        tracing::debug!(
            %provider_config_id,
            package_identifier = %args.package_identifier,
            "skipping ignored autodiscovery item"
        );
        return Ok(());
    }

    // Phase 1: Check if this specific host already tracks (provider_config_id, package_identifier).
    let existing_host_link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::ProviderConfigId.eq(provider_config_id))
        .filter(host_software_item::Column::PackageIdentifier.eq(args.package_identifier))
        .one(db)
        .await?;

    if let Some(link) = existing_host_link {
        // Verify the linked software item is still active.  `discard_pending_items`
        // removes host links together with the soft-delete, but this check guards
        // against any pre-existing orphaned rows so re-discovery always surfaces a
        // fresh pending item for previously discarded packages.
        let linked_item_active = SoftwareItem::find()
            .filter(software_item::Column::Id.eq(link.software_item_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(db)
            .await?
            .is_some();

        if linked_item_active {
            // Just refresh the installed version — no schema changes needed.
            let mut active: host_software_item::ActiveModel = link.into();
            active.installed_version = Set(Some(args.installed_version.to_string()));
            active.installed_version_detected_at = Set(Some(now));
            active.update(db).await?;
            return Ok(());
        }

        // The linked software item was discarded; remove the orphaned host link so
        // this package is treated as new and phases 2/3 create a fresh pending item.
        tracing::debug!(
            %provider_config_id,
            package_identifier = %args.package_identifier,
            "removing orphaned host link for discarded software item; will re-discover"
        );
        let link_model: host_software_item::ActiveModel = link.into();
        link_model.delete(db).await?;
        // Fall through to phases 2/3.
    }

    // Phase 2: Check if any other host in this tenant already has
    // (provider_config_id, package_identifier). If so, reuse the existing software item
    // so the global catalog stays unified.
    let candidate_links: Vec<Uuid> = HostSoftwareItem::find()
        .filter(host_software_item::Column::ProviderConfigId.eq(provider_config_id))
        .filter(host_software_item::Column::PackageIdentifier.eq(args.package_identifier))
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
        // Phase 3: Create a new pending software item — identity only, no provider fields.
        let new_id = Uuid::now_v7();
        let new_item = software_item::ActiveModel {
            id: Set(new_id),
            tenant_id: Set(tenant_id),
            name: Set(args.name.to_string()),
            enabled: Set(false),
            discovery_state: Set(Some(SoftwareDiscoveryState::Pending)),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        SoftwareItem::insert(new_item).exec(db).await?;
        tracing::debug!(
            package_identifier = %args.package_identifier,
            provider_type_str = %args.provider_type_str,
            "created pending software item from discovery"
        );
        new_id
    };

    // Insert a new host_software_item link carrying the provider info.
    // config_override is always NULL for auto-discovered items.
    let link = host_software_item::ActiveModel {
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        provider_config_id: Set(provider_config_id),
        package_identifier: Set(args.package_identifier.to_string()),
        config_override: Set(None),
        installed_version: Set(Some(args.installed_version.to_string())),
        installed_version_detected_at: Set(Some(now)),
        last_updated_at: Set(None),
        linked_at: Set(now),
    };
    HostSoftwareItem::insert(link).exec(db).await?;

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

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use uptrakit_shared_db::SoftwareDiscoveryState;
    use uptrakit_shared_db::entity::{host, provider_config, tenant};

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

    async fn insert_provider_config(db: &DatabaseConnection, id: Uuid, tenant_id: Uuid) {
        let now = time::OffsetDateTime::now_utc();
        provider_config::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(format!("Test Provider {id}")),
            provider_type: Set("homebrew".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert provider_config");
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
        SoftwareItem::insert(model).exec(db).await.expect("insert software_item");
    }

    async fn insert_host_link(
        db: &DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
        provider_config_id: Uuid,
        package_identifier: &str,
    ) {
        let now = time::OffsetDateTime::now_utc();
        let link = host_software_item::ActiveModel {
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            provider_config_id: Set(provider_config_id),
            package_identifier: Set(package_identifier.to_string()),
            config_override: Set(None),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(Some(now)),
            last_updated_at: Set(None),
            linked_at: Set(now),
        };
        HostSoftwareItem::insert(link).exec(db).await.expect("insert host_software_item");
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    /// `discard_pending_items` must remove the `host_software_items` rows for
    /// every discarded software item so that future discovery runs do not find
    /// an orphaned link and return early without creating a new pending item.
    #[tokio::test]
    async fn discard_pending_items_removes_host_links() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_provider_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, item_id, tenant_id, "git", None).await;
        insert_host_link(&db, host_id, item_id, pc_id, "git").await;

        // Sanity: one host link exists before discard.
        let links_before = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .count(&db)
            .await
            .expect("count before");
        assert_eq!(links_before, 1, "expected one host link before discard");

        let result =
            discard_pending_items(&db, tenant_id, None, None).await.expect("discard");
        assert_eq!(result.discarded_count, 1);

        // The host link must be gone after discard.
        let links_after = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .count(&db)
            .await
            .expect("count after");
        assert_eq!(links_after, 0, "host link must be deleted when software item is discarded");
    }

    /// When `process_one_discovery` encounters a `host_software_item` row that
    /// points to a deactivated (discarded) software item, it must delete the
    /// orphaned link and create a fresh pending item instead of returning early.
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
        insert_provider_config(&db, pc_id, tenant_id).await;

        // Insert a discarded (deactivated) software item and its orphaned link —
        // the state that existed before this fix.
        insert_software_item(&db, old_item_id, tenant_id, "curl", Some(now)).await;
        insert_host_link(&db, host_id, old_item_id, pc_id, "curl").await;

        let args = ProcessDiscoveryArgs {
            package_identifier: "curl",
            name: "curl",
            installed_version: "8.0.0",
            provider_type_str: "homebrew",
        };
        let ignore_set = HashSet::new();

        process_one_discovery(&db, tenant_id, host_id, pc_id, args, &ignore_set, now)
            .await
            .expect("process_one_discovery");

        // The orphaned link must be gone.
        let orphan_count = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(old_item_id))
            .count(&db)
            .await
            .expect("orphan count");
        assert_eq!(orphan_count, 0, "orphaned host link must be deleted");

        // A new active pending software item must have been created.
        let active_items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .all(&db)
            .await
            .expect("active items");
        assert_eq!(active_items.len(), 1, "expected exactly one new pending item");
        assert_eq!(
            active_items[0].discovery_state,
            Some(SoftwareDiscoveryState::Pending)
        );

        // A new host link pointing to the new pending item must exist.
        let new_link_count = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::ProviderConfigId.eq(pc_id))
            .filter(host_software_item::Column::PackageIdentifier.eq("curl"))
            .count(&db)
            .await
            .expect("new link count");
        assert_eq!(new_link_count, 1, "expected a new host link for the re-discovered item");
    }

    /// `process_one_discovery` must update `installed_version` in place when the
    /// existing host link points to an active (non-discarded) software item.
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
        insert_provider_config(&db, pc_id, tenant_id).await;

        // Active software item with an existing host link at version 1.0.0.
        insert_software_item(&db, item_id, tenant_id, "wget", None).await;
        insert_host_link(&db, host_id, item_id, pc_id, "wget").await;

        let args = ProcessDiscoveryArgs {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            provider_type_str: "homebrew",
        };
        let ignore_set = HashSet::new();

        process_one_discovery(&db, tenant_id, host_id, pc_id, args, &ignore_set, now)
            .await
            .expect("process_one_discovery");

        // The original item must still be active and the link updated.
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

    // ── PHS result processing ─────────────────────────────────────────────────

    use uptrakit_internal_wire::{DiscoveredSoftware as WireDiscoveredSoftware, DiscoveryProviderResult, PluginType};

    fn phs_result_with_github(pkg_id: &str, name: &str, version: &str, owner: &str, repo: &str) -> DiscoveryProviderResult {
        DiscoveryProviderResult {
            provider_type: PluginType::ProxmoxHelperScripts,
            provider_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: name.to_string(),
                installed_version: version.to_string(),
                extra: Some(serde_json::json!({
                    "github_owner": owner,
                    "github_repo": repo,
                })),
            }],
        }
    }

    fn phs_result_with_apt(pkg_id: &str, name: &str, version: &str, apt_pkg: &str) -> DiscoveryProviderResult {
        DiscoveryProviderResult {
            provider_type: PluginType::ProxmoxHelperScripts,
            provider_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: name.to_string(),
                installed_version: version.to_string(),
                extra: Some(serde_json::json!({ "apt_package": apt_pkg })),
            }],
        }
    }

    fn phs_result_no_extra(pkg_id: &str) -> DiscoveryProviderResult {
        DiscoveryProviderResult {
            provider_type: PluginType::ProxmoxHelperScripts,
            provider_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: pkg_id.to_string(),
                installed_version: "1.0.0".to_string(),
                extra: None,
            }],
        }
    }

    /// A GitHub PHS item must create a `github_releases` provider config and link the item.
    #[tokio::test]
    async fn process_phs_results_github_creates_provider_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result = phs_result_with_github("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");

        process_phs_results(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_phs_results");

        // A github_releases provider config must exist.
        let configs = ProviderConfig::find()
            .filter(provider_config::Column::TenantId.eq(tenant_id))
            .filter(provider_config::Column::ProviderType.eq("github_releases"))
            .all(&db)
            .await
            .expect("query configs");
        assert_eq!(configs.len(), 1, "expected one github_releases config");
        assert_eq!(configs[0].name, "BookLore/BookLore");

        // The config JSON must carry the PHS constants.
        let cfg_json = &configs[0].config;
        assert_eq!(cfg_json["owner"], "BookLore");
        assert_eq!(cfg_json["repo"], "BookLore");
        assert_eq!(cfg_json["detect_installed_version_command"], PHS_DETECT_VERSION_CMD);
        assert_eq!(cfg_json["install_command"], PHS_INSTALL_CMD);

        // A host link must exist.
        let links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::PackageIdentifier.eq("booklore"))
            .all(&db)
            .await
            .expect("query links");
        assert_eq!(links.len(), 1, "expected one host link");
        assert_eq!(links[0].installed_version.as_deref(), Some("1.18.5"));
    }

    /// An APT PHS item must create/reuse a shared `apt` provider config.
    #[tokio::test]
    async fn process_phs_results_apt_creates_apt_provider_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result = phs_result_with_apt("grafana", "Grafana", "10.2.3", "grafana");

        process_phs_results(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_phs_results");

        let configs = ProviderConfig::find()
            .filter(provider_config::Column::TenantId.eq(tenant_id))
            .filter(provider_config::Column::ProviderType.eq("apt"))
            .all(&db)
            .await
            .expect("query configs");
        assert_eq!(configs.len(), 1, "expected one apt config");
        assert_eq!(configs[0].name, "APT (auto)");

        let links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::PackageIdentifier.eq("grafana"))
            .all(&db)
            .await
            .expect("query links");
        assert_eq!(links.len(), 1, "expected one host link");
        assert_eq!(links[0].installed_version.as_deref(), Some("10.2.3"));
    }

    /// An install-script fallback APT item (e.g. `influxdb2`) must use the same
    /// shared `apt` config path as a direct APT item.
    #[tokio::test]
    async fn process_phs_results_apt_install_fallback_uses_apt_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        // The PHS provider emits `apt_package: "influxdb2"` for install-script items too.
        let result = phs_result_with_apt("influxdb2", "InfluxDB", "2.7.6", "influxdb2");

        process_phs_results(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_phs_results");

        let configs = ProviderConfig::find()
            .filter(provider_config::Column::TenantId.eq(tenant_id))
            .filter(provider_config::Column::ProviderType.eq("apt"))
            .all(&db)
            .await
            .expect("query configs");
        assert_eq!(configs.len(), 1, "expected shared apt config");

        let links = HostSoftwareItem::find()
            .filter(host_software_item::Column::PackageIdentifier.eq("influxdb2"))
            .all(&db)
            .await
            .expect("query links");
        assert_eq!(links.len(), 1);
    }

    /// A PHS item with no detectable upstream (no `extra` fields) must be skipped.
    #[tokio::test]
    async fn process_phs_results_no_extra_skips_item() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        let result = phs_result_no_extra("n8n");

        process_phs_results(&db, tenant_id, host_id, now, &result)
            .await
            .expect("process_phs_results");

        let configs = ProviderConfig::find()
            .filter(provider_config::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query configs");
        assert!(configs.is_empty(), "no config should be created for skipped item");

        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query items");
        assert!(items.is_empty(), "no software item should be created");
    }

    /// Two hosts discovering the same GitHub PHS app must share one provider config.
    #[tokio::test]
    async fn process_phs_results_two_hosts_share_github_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host1 = Uuid::now_v7();
        let host2 = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host1, tenant_id).await;
        insert_host(&db, host2, tenant_id).await;

        let result1 = phs_result_with_github("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");
        let result2 = phs_result_with_github("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");

        process_phs_results(&db, tenant_id, host1, now, &result1)
            .await
            .expect("host1");
        process_phs_results(&db, tenant_id, host2, now, &result2)
            .await
            .expect("host2");

        // Still only one provider config.
        let config_count = ProviderConfig::find()
            .filter(provider_config::Column::TenantId.eq(tenant_id))
            .filter(provider_config::Column::ProviderType.eq("github_releases"))
            .count(&db)
            .await
            .expect("count configs");
        assert_eq!(config_count, 1, "both hosts must share a single provider config");

        // Each host has its own link.
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
}
