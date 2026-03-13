//! Bulk deletion of all tenant-scoped data for the reset-data endpoint.

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use thiserror::Error;
use uptrakit_shared_db::entity::{
    host, host_discovery_allowlist, host_software_item, host_software_item_plugin, host_tag,
    host_tag_assignment, notification_channel, notification_log, notification_rule, plugin_config,
    plugin_type_setting, proxmox_host_mapping, service_host, software_ignore, software_item,
    tenant_discovery_allowlist, update_batch, update_history, update_output_line,
};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::settings_reset::ResetDeletedCounts;

use crate::TenantDb;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors returned by the reset-data query.
#[derive(Debug, Error)]
pub enum ResetDataQueryError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<ResetDataQueryError>>;
impl_report_conversion!(sea_orm::DbErr => ResetDataQueryError::Database);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Delete all tenant-scoped data in FK-safe order within a single transaction.
///
/// Returns per-category counts of deleted rows for the primary entities.
#[tracing::instrument(skip_all, fields(tenant_id = %tenant_db.tenant_id))]
pub async fn reset_tenant_data(tenant_db: &TenantDb) -> Result<ResetDeletedCounts> {
    let tenant_id = tenant_db.tenant_id;
    let txn = tenant_db.db().begin().await.context_to()?;

    // -- 1. update_output_lines: FK to update_history (no tenant_id) --
    let uh_sub = sea_orm::sea_query::Query::select()
        .column(update_history::Column::Id)
        .from(update_history::Entity)
        .and_where(update_history::Column::TenantId.eq(tenant_id))
        .to_owned();
    update_output_line::Entity::delete_many()
        .filter(update_output_line::Column::UpdateHistoryId.in_subquery(uh_sub))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 2. update_history --
    let update_history_result = update_history::Entity::delete_many()
        .filter(update_history::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 3. update_batches --
    let update_batches_result = update_batch::Entity::delete_many()
        .filter(update_batch::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 4. host_software_item_plugins: no tenant_id, has host_id FK --
    let host_sub = sea_orm::sea_query::Query::select()
        .column(host::Column::Id)
        .from(host::Entity)
        .and_where(host::Column::TenantId.eq(tenant_id))
        .to_owned();
    host_software_item_plugin::Entity::delete_many()
        .filter(host_software_item_plugin::Column::HostId.in_subquery(host_sub.clone()))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 5. host_software_items: no tenant_id, has host_id FK --
    host_software_item::Entity::delete_many()
        .filter(host_software_item::Column::HostId.in_subquery(host_sub.clone()))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 6. proxmox_host_mappings: has tenant_id --
    proxmox_host_mapping::Entity::delete_many()
        .filter(proxmox_host_mapping::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 7. software_ignores: has tenant_id --
    software_ignore::Entity::delete_many()
        .filter(software_ignore::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 8. software_items --
    let software_items_result = software_item::Entity::delete_many()
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 9. plugin_type_settings: has tenant_id --
    plugin_type_setting::Entity::delete_many()
        .filter(plugin_type_setting::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 10. plugin_configs --
    let plugin_configs_result = plugin_config::Entity::delete_many()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 11. notification_log: has tenant_id, FK to notification_rule & channel --
    notification_log::Entity::delete_many()
        .filter(notification_log::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 12. notification_rules: has tenant_id, FK to notification_channel --
    notification_rule::Entity::delete_many()
        .filter(notification_rule::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 13. notification_channels --
    notification_channel::Entity::delete_many()
        .filter(notification_channel::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 14. host_tag_assignments: no tenant_id, FK to host_tag --
    let ht_sub = sea_orm::sea_query::Query::select()
        .column(host_tag::Column::Id)
        .from(host_tag::Entity)
        .and_where(host_tag::Column::TenantId.eq(tenant_id))
        .to_owned();
    host_tag_assignment::Entity::delete_many()
        .filter(host_tag_assignment::Column::HostTagId.in_subquery(ht_sub))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 15. host_discovery_allowlist: has tenant_id --
    host_discovery_allowlist::Entity::delete_many()
        .filter(host_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 16. host_tags --
    let host_tags_result = host_tag::Entity::delete_many()
        .filter(host_tag::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 17. tenant_discovery_allowlist --
    tenant_discovery_allowlist::Entity::delete_many()
        .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 18. service_hosts: no tenant_id, FK to host --
    service_host::Entity::delete_many()
        .filter(service_host::Column::HostId.in_subquery(host_sub))
        .exec(&txn)
        .await
        .context_to()?;

    // -- 19. hosts --
    let hosts_result = host::Entity::delete_many()
        .filter(host::Column::TenantId.eq(tenant_id))
        .exec(&txn)
        .await
        .context_to()?;

    txn.commit().await.context_to()?;

    Ok(ResetDeletedCounts {
        hosts: hosts_result.rows_affected,
        software_items: software_items_result.rows_affected,
        plugin_configs: plugin_configs_result.rows_affected,
        host_tags: host_tags_result.rows_affected,
        update_history: update_history_result.rows_affected,
        update_batches: update_batches_result.rows_affected,
    })
}
