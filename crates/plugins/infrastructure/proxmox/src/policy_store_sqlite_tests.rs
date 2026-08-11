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

use crate::entity::{proxmox_protection_default, proxmox_protection_item_override};
use crate::matching_isolation_tests::{insert_plugin_config, insert_tenant, setup_db};
use crate::policy_store::{
    ProtectionMode, ProtectionPolicy, upsert_global_default, upsert_item_override,
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
