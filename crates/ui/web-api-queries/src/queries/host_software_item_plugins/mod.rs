//! Tenant-scoped lookups against `host_software_item_plugin`.
//!
//! `host_software_item_plugin::Entity` is NOT `TenantScoped`; this module
//! enforces tenant isolation via `TenantDb::find_via_tenant_join` through
//! `software_item::Entity` (which IS `TenantScoped`).

use std::collections::HashMap;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DbErr, FromQueryResult, QueryFilter, QuerySelect, RelationTrait};
use thiserror::Error;
use uptrakit_shared_db::entity::{host_software_item_plugin, software_item};
use uuid::Uuid;

use crate::tenant_db::TenantDb;

#[cfg(all(test, feature = "db-sqlite"))]
mod tests;

/// Errors returned by `host_software_item_plugin` queries.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HostSoftwareItemPluginQueryError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, rootcause::Report<HostSoftwareItemPluginQueryError>>;

#[derive(Debug, FromQueryResult)]
struct PluginAssignmentRow {
    host_software_item_id: Uuid,
    plugin_type: String,
    package_identifier: String,
}

/// Per-`host_software_item` record returned by [`plugin_types_for_role`].
///
/// Includes `package_identifier` so the caller can pass it into the per-plugin
/// enrichment batch without a second DB round-trip — `VersionCheckResult` from
/// the wire does not carry it (it lives on `host_software_item_plugin`).
#[non_exhaustive]
pub struct PluginAssignment {
    /// The plugin type discriminator (e.g. `"package-manager.skills"`).
    pub plugin_type: String,
    /// The package identifier recorded on the assignment row.
    pub package_identifier: String,
}

/// Return `host_software_item_id → PluginAssignment` for the given hsi ids
/// restricted to `role`. Tenant-scoped via `host_software_item_plugin →
/// software_item`. Rows belonging to other tenants are silently excluded by the
/// join — they never appear in the result map.
#[tracing::instrument(skip_all, fields(role, hsi_count = host_software_item_ids.len()))]
pub async fn plugin_types_for_role(
    tenant_db: &TenantDb,
    host_software_item_ids: &[Uuid],
    role: &str,
) -> Result<HashMap<Uuid, PluginAssignment>> {
    if host_software_item_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = tenant_db
        .find_via_tenant_join::<host_software_item_plugin::Entity, software_item::Entity>(
            host_software_item_plugin::Relation::SoftwareItem.def(),
        )
        .filter(
            host_software_item_plugin::Column::HostSoftwareItemId
                .is_in(host_software_item_ids.iter().copied()),
        )
        .filter(host_software_item_plugin::Column::Role.eq(role))
        .select_only()
        .column(host_software_item_plugin::Column::HostSoftwareItemId)
        .column(host_software_item_plugin::Column::PluginType)
        .column(host_software_item_plugin::Column::PackageIdentifier)
        .into_model::<PluginAssignmentRow>()
        .all(tenant_db.db())
        .await
        .map_err(|e| report!(HostSoftwareItemPluginQueryError::Database(e)))?;

    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.host_software_item_id,
                PluginAssignment {
                    plugin_type: r.plugin_type,
                    package_identifier: r.package_identifier,
                },
            )
        })
        .collect())
}
