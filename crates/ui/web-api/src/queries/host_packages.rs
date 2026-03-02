//! Database query helpers for host packages.
//!
//! Covers CRUD operations, version result storage, update count aggregation,
//! and host package ignore rules.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::HashSet;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host_package, host_package_ignore, host_package_update_history, plugin_config, prelude::*,
};
use uptrakit_shared_db::is_unique_constraint_violation;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::host_packages::{
    HostPackageIgnoreResponse, HostPackageResponse, HostPackageUpdateHistoryResponse,
    HostUpdateSummary, ListHostPackagesParams,
};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::update_history::UpdateStatus;
use uuid::Uuid;

use crate::tenant_db::TenantDb;

/// Error returned by host package query helpers.
#[derive(Debug, thiserror::Error)]
pub enum HostPackageError {
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<HostPackageError>>;
impl_report_conversion!(sea_orm::DbErr => HostPackageError::Db);

// ── Host package CRUD ───────────────────────────────────────────────────────

fn model_to_response(m: host_package::Model) -> HostPackageResponse {
    let has_update = match (&m.installed_version, &m.latest_version) {
        (Some(installed), Some(latest)) => installed != latest,
        _ => false,
    };
    HostPackageResponse {
        id: m.id,
        host_id: m.host_id,
        plugin_config_id: m.plugin_config_id,
        package_identifier: m.package_identifier,
        name: m.name,
        installed_version: m.installed_version,
        installed_version_detected_at: m.installed_version_detected_at,
        latest_version: m.latest_version,
        latest_version_fetched_at: m.latest_version_fetched_at,
        update_category: m.update_category,
        enabled: m.enabled,
        last_checked_at: m.last_checked_at,
        last_updated_at: m.last_updated_at,
        created_at: m.created_at,
        has_update,
    }
}

/// List host packages for a given host with filtering and pagination.
pub async fn list_host_packages(
    tenant_db: &TenantDb,
    host_id: Uuid,
    params: &ListHostPackagesParams,
) -> Result<PaginatedResponse<HostPackageResponse>> {
    let pagination = params.pagination().resolve();

    let mut condition = Condition::all()
        .add(host_package::Column::HostId.eq(host_id))
        .add(host_package::Column::DeactivatedAt.is_null());

    if let Some(enabled) = params.enabled {
        condition = condition.add(host_package::Column::Enabled.eq(enabled));
    }

    if let Some(ref category) = params.category {
        condition = condition.add(host_package::Column::UpdateCategory.eq(category.as_str()));
    }

    if let Some(ref search) = params.search {
        let pattern = format!("%{search}%");
        condition = condition.add(
            Condition::any()
                .add(host_package::Column::Name.like(&pattern))
                .add(host_package::Column::PackageIdentifier.like(&pattern)),
        );
    }

    let base_query = tenant_db
        .find::<host_package::Entity>()
        .filter(condition.clone())
        .order_by_asc(host_package::Column::Name);

    let total = tenant_db
        .find::<host_package::Entity>()
        .filter(condition)
        .count(tenant_db.db())
        .await
        .context_to()?;

    let items = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;

    // Apply has_update filter in-memory (can't do SQL != on two nullable cols easily).
    let mut responses: Vec<HostPackageResponse> = items.into_iter().map(model_to_response).collect();

    if let Some(has_update) = params.has_update {
        responses.retain(|r| r.has_update == has_update);
    }

    Ok(PaginatedResponse::new(responses, total, pagination))
}

/// Get a single host package by ID.
pub async fn get_host_package(
    tenant_db: &TenantDb,
    host_id: Uuid,
    package_id: Uuid,
) -> Result<Option<HostPackageResponse>> {
    let result = tenant_db
        .find_by_id::<host_package::Entity, _>(package_id)
        .filter(host_package::Column::HostId.eq(host_id))
        .filter(host_package::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()?;

    Ok(result.map(model_to_response))
}

/// Update a host package (enable/disable).
pub async fn update_host_package(
    tenant_db: &TenantDb,
    host_id: Uuid,
    package_id: Uuid,
    enabled: bool,
) -> Result<Option<HostPackageResponse>> {
    let Some(pkg) = tenant_db
        .find_by_id::<host_package::Entity, _>(package_id)
        .filter(host_package::Column::HostId.eq(host_id))
        .filter(host_package::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()?
    else {
        return Ok(None);
    };

    let mut active: host_package::ActiveModel = pkg.into();
    active.enabled = Set(enabled);
    active.updated_at = Set(OffsetDateTime::now_utc());
    let updated = active.update(tenant_db.db()).await.context_to()?;
    Ok(Some(model_to_response(updated)))
}

/// Soft-delete a host package. Optionally creates an ignore rule.
pub async fn deactivate_host_package(
    tenant_db: &TenantDb,
    host_id: Uuid,
    package_id: Uuid,
    create_ignore: bool,
) -> Result<bool> {
    let Some(pkg) = tenant_db
        .find_by_id::<host_package::Entity, _>(package_id)
        .filter(host_package::Column::HostId.eq(host_id))
        .filter(host_package::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()?
    else {
        return Ok(false);
    };

    let now = OffsetDateTime::now_utc();

    if create_ignore {
        let ignore = host_package_ignore::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_db.tenant_id),
            host_id: Set(host_id),
            plugin_config_id: Set(pkg.plugin_config_id),
            package_identifier: Set(pkg.package_identifier.clone()),
            created_at: Set(now),
        };
        if let Err(e) = HostPackageIgnore::insert(ignore)
            .exec(tenant_db.db())
            .await
            && !is_unique_constraint_violation(&e)
        {
            return Err(report!(HostPackageError::Db(e)));
        }
    }

    let mut active: host_package::ActiveModel = pkg.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(tenant_db.db()).await.context_to()?;
    Ok(true)
}

// ── Update count aggregation ────────────────────────────────────────────────

/// Compute aggregate update counts for a host from its host_packages.
pub async fn compute_update_summary(
    tenant_db: &TenantDb,
    host_id: Uuid,
) -> Result<HostUpdateSummary> {
    let packages = tenant_db
        .find::<host_package::Entity>()
        .filter(host_package::Column::HostId.eq(host_id))
        .filter(host_package::Column::DeactivatedAt.is_null())
        .filter(host_package::Column::Enabled.eq(true))
        .all(tenant_db.db())
        .await
        .context_to()?;

    let mut available = 0u32;
    let mut security = 0u32;

    for pkg in &packages {
        if let (Some(installed), Some(latest)) = (&pkg.installed_version, &pkg.latest_version)
            && installed != latest
        {
            available += 1;
            if pkg.update_category == "security" {
                security += 1;
            }
        }
    }

    Ok(HostUpdateSummary {
        available_updates_count: available,
        security_updates_count: security,
    })
}

/// Compute update summaries for multiple hosts in a single query.
pub async fn compute_update_summaries_batch(
    tenant_db: &TenantDb,
    host_ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, HostUpdateSummary>> {
    if host_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let packages = tenant_db
        .find::<host_package::Entity>()
        .filter(host_package::Column::HostId.is_in(host_ids.to_vec()))
        .filter(host_package::Column::DeactivatedAt.is_null())
        .filter(host_package::Column::Enabled.eq(true))
        .all(tenant_db.db())
        .await
        .context_to()?;

    let mut summaries: std::collections::HashMap<Uuid, HostUpdateSummary> =
        std::collections::HashMap::new();

    for pkg in &packages {
        if let (Some(installed), Some(latest)) = (&pkg.installed_version, &pkg.latest_version)
            && installed != latest
        {
            let entry = summaries.entry(pkg.host_id).or_default();
            entry.available_updates_count += 1;
            if pkg.update_category == "security" {
                entry.security_updates_count += 1;
            }
        }
    }

    Ok(summaries)
}

// ── Host package ignore rules ───────────────────────────────────────────────

/// List host package ignore rules for a host.
pub async fn list_host_package_ignores(
    tenant_db: &TenantDb,
    host_id: Uuid,
) -> Result<Vec<HostPackageIgnoreResponse>> {
    let rules = tenant_db
        .find::<host_package_ignore::Entity>()
        .filter(host_package_ignore::Column::HostId.eq(host_id))
        .order_by_asc(host_package_ignore::Column::PackageIdentifier)
        .all(tenant_db.db())
        .await
        .context_to()?;

    Ok(rules
        .into_iter()
        .map(|r| HostPackageIgnoreResponse {
            id: r.id,
            host_id: r.host_id,
            plugin_config_id: r.plugin_config_id,
            package_identifier: r.package_identifier,
            created_at: r.created_at,
        })
        .collect())
}

/// Create a host package ignore rule. Returns `true` if created, `false` if already exists.
pub async fn create_host_package_ignore(
    tenant_db: &TenantDb,
    host_id: Uuid,
    plugin_config_id: Uuid,
    package_identifier: &str,
) -> Result<bool> {
    let record = host_package_ignore::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_db.tenant_id),
        host_id: Set(host_id),
        plugin_config_id: Set(plugin_config_id),
        package_identifier: Set(package_identifier.to_string()),
        created_at: Set(OffsetDateTime::now_utc()),
    };

    match HostPackageIgnore::insert(record)
        .exec(tenant_db.db())
        .await
    {
        Ok(_) => Ok(true),
        Err(e) if is_unique_constraint_violation(&e) => Ok(false),
        Err(e) => Err(report!(HostPackageError::Db(e))),
    }
}

/// Delete a host package ignore rule. Returns `true` if deleted.
pub async fn delete_host_package_ignore(
    tenant_db: &TenantDb,
    host_id: Uuid,
    ignore_id: Uuid,
) -> Result<bool> {
    let result = tenant_db
        .find_by_id::<host_package_ignore::Entity, _>(ignore_id)
        .filter(host_package_ignore::Column::HostId.eq(host_id))
        .one(tenant_db.db())
        .await
        .context_to()?;

    if let Some(rule) = result {
        let active: host_package_ignore::ActiveModel = rule.into();
        active.delete(tenant_db.db()).await.context_to()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Load the ignore set for `(host_id, plugin_config_id)`.
pub async fn load_host_package_ignore_set(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<HashSet<String>> {
    let rules = HostPackageIgnore::find()
        .filter(host_package_ignore::Column::TenantId.eq(tenant_id))
        .filter(host_package_ignore::Column::HostId.eq(host_id))
        .filter(host_package_ignore::Column::PluginConfigId.eq(plugin_config_id))
        .all(db)
        .await
        .context_to()?;
    Ok(rules.into_iter().map(|r| r.package_identifier).collect())
}

// ── Discovery routing: find-or-create host package ──────────────────────────

/// Parameters for [`find_or_create_host_package`].
pub struct FindOrCreateHostPackageParams<'a> {
    pub db: &'a sea_orm::DatabaseConnection,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub plugin_config_id: Uuid,
    pub package_identifier: &'a str,
    pub name: &'a str,
    pub installed_version: &'a str,
    pub ignore_set: &'a HashSet<String>,
}

/// Find or create a host package record.
///
/// - Checks the ignore list first
/// - If active record exists for `(host_id, plugin_config_id, package_identifier)`,
///   updates installed_version in place
/// - Otherwise creates a new enabled host_package
pub async fn find_or_create_host_package(params: FindOrCreateHostPackageParams<'_>) -> Result<()> {
    let FindOrCreateHostPackageParams {
        db,
        tenant_id,
        host_id,
        plugin_config_id,
        package_identifier,
        name,
        installed_version,
        ignore_set,
    } = params;

    if ignore_set.contains(package_identifier) {
        tracing::debug!(
            %plugin_config_id,
            package_identifier = %package_identifier,
            "skipping ignored host package"
        );
        return Ok(());
    }

    let now = OffsetDateTime::now_utc();

    // Check if active record exists for this triple.
    let existing = HostPackage::find()
        .filter(host_package::Column::HostId.eq(host_id))
        .filter(host_package::Column::PluginConfigId.eq(plugin_config_id))
        .filter(host_package::Column::PackageIdentifier.eq(package_identifier))
        .filter(host_package::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?;

    if let Some(existing) = existing {
        // Update installed version in place.
        let mut active: host_package::ActiveModel = existing.into();
        active.installed_version = Set(Some(installed_version.to_string()));
        active.installed_version_detected_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        return Ok(());
    }

    // Create new host package (enabled by default, no approval step).
    let new_pkg = host_package::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        plugin_config_id: Set(plugin_config_id),
        package_identifier: Set(package_identifier.to_string()),
        name: Set(name.to_string()),
        installed_version: Set(Some(installed_version.to_string())),
        installed_version_detected_at: Set(Some(now)),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        update_category: Set("unknown".to_string()),
        enabled: Set(true),
        last_checked_at: Set(None),
        last_updated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    match HostPackage::insert(new_pkg).exec(db).await {
        Ok(_) => {
            tracing::debug!(
                package_identifier = %package_identifier,
                "created host package from discovery"
            );
        }
        Err(e) if is_unique_constraint_violation(&e) => {
            // Concurrent discovery race — update in place instead.
            let existing = HostPackage::find()
                .filter(host_package::Column::HostId.eq(host_id))
                .filter(host_package::Column::PluginConfigId.eq(plugin_config_id))
                .filter(host_package::Column::PackageIdentifier.eq(package_identifier))
                .filter(host_package::Column::DeactivatedAt.is_null())
                .one(db)
                .await
                .context_to()?;
            if let Some(existing) = existing {
                let mut active: host_package::ActiveModel = existing.into();
                active.installed_version = Set(Some(installed_version.to_string()));
                active.installed_version_detected_at = Set(Some(now));
                active.updated_at = Set(now);
                active.update(db).await.context_to()?;
            }
        }
        Err(e) => return Err(report!(HostPackageError::Db(e))),
    }

    Ok(())
}

// ── Update history helpers ──────────────────────────────────────────────────

fn history_to_response(m: host_package_update_history::Model) -> HostPackageUpdateHistoryResponse {
    let status = m.status.parse::<UpdateStatus>().unwrap_or(UpdateStatus::Pending);
    HostPackageUpdateHistoryResponse {
        id: m.id,
        host_package_id: m.host_package_id,
        from_version: m.from_version,
        to_version: m.to_version,
        status,
        output: m.output.unwrap_or_default(),
        actor_type: m.actor_type,
        actor_id: m.actor_id,
        update_category: m.update_category,
        started_at: m.started_at,
        completed_at: m.completed_at,
        created_at: m.created_at,
        batch_id: m.batch_id,
    }
}

/// Get recent update history for a host package.
pub async fn get_host_package_update_history(
    tenant_db: &TenantDb,
    host_package_id: Uuid,
    limit: u64,
) -> Result<Vec<HostPackageUpdateHistoryResponse>> {
    let items = tenant_db
        .find::<host_package_update_history::Entity>()
        .filter(host_package_update_history::Column::HostPackageId.eq(host_package_id))
        .order_by_desc(host_package_update_history::Column::CreatedAt)
        .limit(Some(limit))
        .all(tenant_db.db())
        .await
        .context_to()?;

    Ok(items.into_iter().map(history_to_response).collect())
}

/// Get the plugin config associated with a host package.
pub async fn get_host_package_plugin_config(
    tenant_db: &TenantDb,
    plugin_config_id: Uuid,
) -> Result<Option<plugin_config::Model>> {
    let result = tenant_db
        .find_by_id::<plugin_config::Entity, _>(plugin_config_id)
        .one(tenant_db.db())
        .await
        .context_to()?;
    Ok(result)
}
