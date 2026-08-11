//! Real-SQLite double-upsert tests for `policy_store` — `MockDatabase`
//! (the idiom of `policy_store`'s own unit tests) cannot exercise the
//! BEGIN IMMEDIATE transaction wrapping these upserts.

#![expect(
    clippy::expect_used,
    reason = "test-only module: panics are how tests learn about DB setup bugs"
)]

use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::entity::{
    proxmox_protection_audit, proxmox_protection_default, proxmox_protection_item_override,
};
use crate::matching_isolation_tests::{insert_host, insert_plugin_config, insert_tenant, setup_db};
use crate::policy_store::{
    ProtectionAudit, ProtectionMode, ProtectionPolicy, upsert_global_default, upsert_item_override,
    upsert_protection_audit,
};

async fn insert_software_item(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    uptrakit_shared_db::entity::software_item::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        name: Set(format!("item-{id}")),
        featured: Set(false),
        icon_url: Set(None),
        last_checked_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        awaiting_restart_timeout: Set(None),
    }
    .insert(db)
    .await
    .expect("insert software item");
    id
}

async fn insert_update_history(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
) {
    let now = OffsetDateTime::now_utc();
    uptrakit_shared_db::entity::update_history::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(None),
        from_version: Set(Some("1.0.0".to_string())),
        to_version: Set(Some("1.1.0".to_string())),
        status: Set(uptrakit_shared_db::entity::update_history::UpdateStatus::Pending),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set("user".to_string()),
        actor_id: Set(String::new()),
        execution_owner_service_id: Set(None),
        execution_owner_instance_id: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        awaiting_restart_since: Set(None),
        created_at: Set(now),
        update_category: Set("security".to_string()),
        batch_id: Set(None),
        interactive: Set(false),
        output_truncated: Set(false),
        pre_update_protection_status: Set(None),
        pre_update_protection_summary: Set(None),
        recovery_hint: Set(None),
    }
    .insert(db)
    .await
    .expect("insert update_history");
}

fn snapshot_policy() -> ProtectionPolicy {
    ProtectionPolicy {
        mode: ProtectionMode::Snapshot,
        backup_target_key: None,
        snapshot_timeout_seconds: Some(60),
        backup_timeout_seconds: None,
    }
}

fn backup_policy() -> ProtectionPolicy {
    ProtectionPolicy {
        mode: ProtectionMode::Backup,
        backup_target_key: Some("pve1:local:dir".to_string()),
        snapshot_timeout_seconds: None,
        backup_timeout_seconds: Some(1200),
    }
}

#[tokio::test]
async fn double_upsert_global_default_updates_the_single_row() {
    let db = setup_db().await;
    let tenant_id = insert_tenant(&db).await;
    let plugin_config_id = insert_plugin_config(&db, tenant_id).await;

    upsert_global_default(&db, tenant_id, plugin_config_id, &snapshot_policy())
        .await
        .expect("insert path");
    upsert_global_default(&db, tenant_id, plugin_config_id, &backup_policy())
        .await
        .expect("update path");

    let rows = proxmox_protection_default::Entity::find()
        .all(&db)
        .await
        .expect("load rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].mode, ProtectionMode::Backup.as_str());
    assert_eq!(rows[0].backup_target_key.as_deref(), Some("pve1:local:dir"));
    assert_eq!(rows[0].backup_timeout_seconds, Some(1200));
}

#[tokio::test]
async fn double_upsert_item_override_updates_the_single_row() {
    let db = setup_db().await;
    let tenant_id = insert_tenant(&db).await;
    let plugin_config_id = insert_plugin_config(&db, tenant_id).await;
    let software_item_id = insert_software_item(&db, tenant_id).await;

    upsert_item_override(&db, software_item_id, plugin_config_id, &snapshot_policy())
        .await
        .expect("insert path");
    upsert_item_override(&db, software_item_id, plugin_config_id, &backup_policy())
        .await
        .expect("update path");

    let rows = proxmox_protection_item_override::Entity::find()
        .all(&db)
        .await
        .expect("load rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].mode, ProtectionMode::Backup.as_str());
    assert_eq!(rows[0].backup_target_key.as_deref(), Some("pve1:local:dir"));
    assert_eq!(rows[0].backup_timeout_seconds, Some(1200));
}

#[tokio::test]
async fn double_upsert_protection_audit_updates_the_single_row() {
    let db = setup_db().await;
    let tenant_id = insert_tenant(&db).await;
    let plugin_config_id = insert_plugin_config(&db, tenant_id).await;
    let host_id = insert_host(&db, tenant_id).await;
    let software_item_id = insert_software_item(&db, tenant_id).await;
    let update_history_id = Uuid::now_v7();
    insert_update_history(&db, update_history_id, tenant_id, host_id, software_item_id).await;

    let audit = ProtectionAudit {
        update_history_id,
        tenant_id,
        host_id,
        software_item_id,
        plugin_config_id,
        mapping_id: None,
        mode: ProtectionMode::Snapshot,
        status: "pending".to_string(),
        artifact_kind: None,
        artifact_ref: None,
        backup_target_key: None,
        detail: None,
        error_message: None,
    };
    upsert_protection_audit(&db, &audit)
        .await
        .expect("insert path");

    let updated_audit = ProtectionAudit {
        status: "completed".to_string(),
        artifact_kind: Some("snapshot".to_string()),
        artifact_ref: Some("snap-1".to_string()),
        detail: Some("snapshot taken".to_string()),
        ..audit
    };
    upsert_protection_audit(&db, &updated_audit)
        .await
        .expect("update path");

    let rows = proxmox_protection_audit::Entity::find()
        .all(&db)
        .await
        .expect("load rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].update_history_id, update_history_id);
    assert_eq!(rows[0].status, "completed");
    assert_eq!(rows[0].artifact_kind.as_deref(), Some("snapshot"));
    assert_eq!(rows[0].artifact_ref.as_deref(), Some("snap-1"));
    assert_eq!(rows[0].detail.as_deref(), Some("snapshot taken"));
}
