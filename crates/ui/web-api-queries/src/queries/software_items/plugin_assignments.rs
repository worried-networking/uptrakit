//! Plugin assignment management for software items.

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uptrakit_shared_db::entity::{host_software_item_plugin, prelude::*};
use uptrakit_web_api_types::PluginRole;
use uptrakit_web_api_types::software_items::SoftwareItemDetailResponse;
use uuid::Uuid;

use crate::tenant_db::TenantDb;

use super::{
    SoftwareItemQueryError, build_detail_response, find_active_item, load_item_hosts,
    load_latest_version_for_item, load_plugins,
};

// ---------------------------------------------------------------------------
// Transaction-aware _in_tx variant (for emit_stateful callers)
// ---------------------------------------------------------------------------

/// Delete a plugin assignment inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Returns `true` if the assignment was found and deleted, `false` if not found.
///
/// # Errors
///
/// Returns `SoftwareItemQueryError::Db` on DB failures.
pub async fn delete_plugin_assignment_in_tx(
    txn: &sea_orm::DatabaseTransaction,
    item_id: Uuid,
    host_id: Uuid,
    role: &PluginRole,
    ordinal: i32,
) -> super::Result<bool> {
    let deleted = HostSoftwareItemPlugin::delete_many()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role.as_str()))
        .filter(host_software_item_plugin::Column::Ordinal.eq(ordinal))
        .exec(txn)
        .await
        .context_to()?;

    Ok(deleted.rows_affected > 0)
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
) -> super::Result<SoftwareItemDetailResponse> {
    find_active_item(tenant_db.db(), tenant_db.tenant_id(), item_id)
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

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id(), item_id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let hosts = load_item_hosts(tenant_db.db(), tenant_db.tenant_id(), item_id).await;
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
