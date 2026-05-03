//! Tenant-data reset callback for the Proxmox plugin.
//!
//! Deletes all Proxmox-specific plugin rows for a tenant in FK-safe order.
//! Registered via `PluginDescriptor::reset_tenant_data`; called by the registry
//! during tenant-data reset inside an existing transaction.
//!
//! `proxmox_protection_audit` is intentionally omitted — audit records are
//! append-only history entries that must outlive plugin configuration.

#[cfg(feature = "migrations")]
pub(crate) fn proxmox_reset_tenant_data<'a>(
    tenant_id: uuid::Uuid,
    txn: &'a sea_orm::DatabaseTransaction,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), sea_orm::DbErr>> + Send + 'a>> {
    Box::pin(async move {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
        use uptrakit_shared_db::entity::plugin_config;

        use crate::entity::{
            proxmox_backup_target_cache, proxmox_protection_default,
            proxmox_protection_item_override,
        };
        use uptrakit_shared_db::entity::proxmox_host_mapping;

        // proxmox_protection_item_override has no tenant_id column.
        // Delete via plugin_config_id subquery scoped to the tenant.
        let config_ids: Vec<uuid::Uuid> = plugin_config::Entity::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .select_only()
            .column(plugin_config::Column::Id)
            .into_tuple::<uuid::Uuid>()
            .all(txn)
            .await?;

        proxmox_protection_item_override::Entity::delete_many()
            .filter(proxmox_protection_item_override::Column::PluginConfigId.is_in(config_ids))
            .exec(txn)
            .await?;

        proxmox_protection_default::Entity::delete_many()
            .filter(proxmox_protection_default::Column::TenantId.eq(tenant_id))
            .exec(txn)
            .await?;

        proxmox_backup_target_cache::Entity::delete_many()
            .filter(proxmox_backup_target_cache::Column::TenantId.eq(tenant_id))
            .exec(txn)
            .await?;

        proxmox_host_mapping::Entity::delete_many()
            .filter(proxmox_host_mapping::Column::TenantId.eq(tenant_id))
            .exec(txn)
            .await?;

        // proxmox_protection_audit: audit table — intentionally never deleted.
        Ok(())
    })
}

/// No-op stub used when `migrations` feature is inactive.
#[cfg(not(feature = "migrations"))]
#[allow(dead_code)]
pub(crate) fn proxmox_reset_tenant_data() {}
