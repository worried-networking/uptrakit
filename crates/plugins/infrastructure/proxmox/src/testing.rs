//! Test-only insertion + assertion helpers. See `lib.rs` doc comment.

#![expect(
    clippy::expect_used,
    reason = "test-only helpers: panics are how callers learn about DB setup bugs"
)]

use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::entity::{
    proxmox_backup_target_cache, proxmox_host_mapping, proxmox_protection_audit,
    proxmox_protection_default, proxmox_resource_scaling_record, proxmox_scaling_default,
};
use crate::scaling_mode::ScalingMode;

pub async fn insert_host_mapping(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    host_id: Uuid,
    node: &str,
    vmid: i32,
    vm_type: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    proxmox_host_mapping::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        plugin_config_id: Set(plugin_config_id),
        host_id: Set(Some(host_id)),
        proxmox_node: Set(node.to_string()),
        proxmox_vmid: Set(vmid),
        proxmox_type: Set(vm_type.to_string()),
        proxmox_name: Set(None),
        proxmox_status: Set("running".to_string()),
        hostname: Set(None),
        ip_addresses: Set(None),
        machine_id: Set(None),
        match_method: Set(Some("manual".to_string())),
        discovered_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert proxmox_host_mapping");
    id
}

/// Insert an **unmatched** `proxmox_host_mapping` row (`host_id = NULL`),
/// i.e. a discovered guest awaiting matching to an Uptrakit host.
///
/// Distinct from [`insert_host_mapping`], which always assigns `host_id`.
pub async fn insert_unmatched_host_mapping(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    node: &str,
    vmid: i32,
    vm_type: &str,
    name: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    proxmox_host_mapping::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        plugin_config_id: Set(plugin_config_id),
        host_id: Set(None),
        proxmox_node: Set(node.to_string()),
        proxmox_vmid: Set(vmid),
        proxmox_type: Set(vm_type.to_string()),
        proxmox_name: Set(Some(name.to_string())),
        proxmox_status: Set("running".to_string()),
        hostname: Set(None),
        ip_addresses: Set(None),
        machine_id: Set(None),
        match_method: Set(None),
        discovered_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert unmatched proxmox_host_mapping");
    id
}

/// Read back the `host_id` column of a `proxmox_host_mapping` row by id.
///
/// Used to confirm a match handler actually ran to completion (gate-pass is
/// not the same as handler-runs).
pub async fn host_mapping_host_id(db: &DatabaseConnection, mapping_id: Uuid) -> Option<Uuid> {
    proxmox_host_mapping::Entity::find_by_id(mapping_id)
        .one(db)
        .await
        .expect("query proxmox_host_mapping")
        .expect("mapping present")
        .host_id
}

pub async fn insert_protection_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    mode: &str,
    backup_target_key: Option<&str>,
) {
    let now = OffsetDateTime::now_utc();
    proxmox_protection_default::ActiveModel {
        tenant_id: Set(tenant_id),
        plugin_config_id: Set(plugin_config_id),
        mode: Set(mode.to_string()),
        backup_target_key: Set(backup_target_key.map(str::to_string)),
        snapshot_timeout_seconds: Set(None),
        backup_timeout_seconds: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert proxmox_protection_default");
}

pub async fn insert_scaling_default_delta(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    delta_cores: i32,
    delta_memory_mb: i32,
) {
    insert_scaling_default(
        db,
        tenant_id,
        plugin_config_id,
        ScalingMode::Delta,
        None,
        None,
        Some(delta_cores),
        Some(delta_memory_mb),
    )
    .await;
}

pub async fn insert_scaling_default_absolute(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    absolute_cores: i32,
    absolute_memory_mb: i32,
) {
    insert_scaling_default(
        db,
        tenant_id,
        plugin_config_id,
        ScalingMode::Absolute,
        Some(absolute_cores),
        Some(absolute_memory_mb),
        None,
        None,
    )
    .await;
}

#[expect(
    clippy::too_many_arguments,
    reason = "test fixture explicitly enumerates every column to avoid masking schema drift"
)]
async fn insert_scaling_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    scaling_mode: ScalingMode,
    absolute_cores: Option<i32>,
    absolute_memory_mb: Option<i32>,
    delta_cores: Option<i32>,
    delta_memory_mb: Option<i32>,
) {
    let now = OffsetDateTime::now_utc();
    proxmox_scaling_default::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        plugin_config_id: Set(plugin_config_id),
        scaling_mode: Set(scaling_mode),
        absolute_cores: Set(absolute_cores),
        absolute_memory_mb: Set(absolute_memory_mb),
        delta_cores: Set(delta_cores),
        delta_memory_mb: Set(delta_memory_mb),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert proxmox_scaling_default");
}

pub async fn insert_backup_target_cache(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    node: &str,
    storage_id: &str,
    storage_type: &str,
    target_key: &str,
) {
    let now = OffsetDateTime::now_utc();
    proxmox_backup_target_cache::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        plugin_config_id: Set(plugin_config_id),
        proxmox_node: Set(node.to_string()),
        storage_id: Set(storage_id.to_string()),
        storage_type: Set(storage_type.to_string()),
        target_key: Set(target_key.to_string()),
        discovered_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert proxmox_backup_target_cache");
}

#[expect(
    clippy::too_many_arguments,
    reason = "test fixture explicitly enumerates every column to avoid masking schema drift"
)]
pub async fn insert_resource_scaling_record(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    update_history_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
    mapping_id: Uuid,
    vm_type: &str,
    original_cores: i32,
    original_memory_mb: i64,
    scaled_cores: i32,
    scaled_memory_mb: i64,
) {
    let now = OffsetDateTime::now_utc();
    proxmox_resource_scaling_record::ActiveModel {
        update_history_id: Set(update_history_id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        plugin_config_id: Set(plugin_config_id),
        mapping_id: Set(mapping_id),
        vm_type: Set(vm_type.to_string()),
        original_cores: Set(original_cores),
        original_memory_mb: Set(original_memory_mb),
        scaled_cores: Set(scaled_cores),
        scaled_memory_mb: Set(scaled_memory_mb),
        scale_status: Set("scaled".to_string()),
        restore_status: Set("pending".to_string()),
        error_message: Set(None),
        scaling_mode_used: Set("absolute".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert proxmox_resource_scaling_record");
}

pub async fn count_protection_audits(db: &DatabaseConnection) -> usize {
    proxmox_protection_audit::Entity::find()
        .all(db)
        .await
        .expect("query proxmox_protection_audit")
        .len()
}

pub async fn first_scaling_record(
    db: &DatabaseConnection,
) -> proxmox_resource_scaling_record::Model {
    proxmox_resource_scaling_record::Entity::find()
        .one(db)
        .await
        .expect("query proxmox_resource_scaling_record")
        .expect("scaling record present")
}

pub async fn count_scaling_records(db: &DatabaseConnection) -> usize {
    proxmox_resource_scaling_record::Entity::find()
        .all(db)
        .await
        .expect("query proxmox_resource_scaling_record")
        .len()
}
