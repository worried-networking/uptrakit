//! Real-SQLite double-upsert tests for `policy_store` — `MockDatabase`
//! (the idiom of `policy_store`'s own unit tests) cannot exercise the
//! BEGIN IMMEDIATE transaction wrapping these upserts.

#![expect(
    clippy::expect_used,
    reason = "test-only module: panics are how tests learn about DB setup bugs"
)]

use std::time::Duration;

use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set, SqlxSqliteConnector};
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
        timeout_seconds: Set(None),
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

/// Opens a real on-disk sqlite connection pool (WAL + busy_timeout), the same
/// idiom as `uptrakit-db-tx`'s `busy_snapshot_tests`: a `MockDatabase` cannot
/// model real SQLite locking, and `sqlite::memory:` gives each connection its
/// own private database rather than one shared file two connections can race
/// against.
async fn connect(path: &std::path::Path) -> DatabaseConnection {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_millis(2000));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect sqlite");
    SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)
}

/// The double-upsert tests above run both calls sequentially on one
/// connection, so they stay green even if `upsert_global_default`'s
/// `begin_immediate()`/`commit()` wrap is deleted — they never exercise two
/// writers actually racing. This test does: two real connections over one
/// shared on-disk database both call the real `upsert_global_default` for
/// the same `(tenant_id, plugin_config_id)` key concurrently.
///
/// Without `BEGIN IMMEDIATE`, each connection's read-then-write runs as two
/// separate autocommitted statements, so both connections can read "no row"
/// before either has written — the second writer's INSERT then hits the
/// composite-PK conflict on `(tenant_id, plugin_config_id)` and returns an
/// error instead of taking the update arm. With `BEGIN IMMEDIATE`, the write
/// lock is taken at `BEGIN`, so the loser blocks until the winner commits,
/// then re-reads under the lock and always takes the update arm: both calls
/// succeed and exactly one row (matching one writer's policy, never a torn
/// mix of both) survives.
#[tokio::test]
async fn concurrent_global_default_upserts_do_not_conflict() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("policy.db");

    let a = connect(&path).await;
    uptrakit_shared_db::migration::run_migrations_with_plugins(
        &a,
        crate::ProxmoxPlugin::controller_migrations,
    )
    .await
    .expect("shared + proxmox migrations should run");
    let b = connect(&path).await;

    let tenant_id = insert_tenant(&a).await;
    let plugin_config_id = insert_plugin_config(&a, tenant_id).await;
    let snapshot = snapshot_policy();
    let backup = backup_policy();

    let (r1, r2) = tokio::join!(
        upsert_global_default(&a, tenant_id, plugin_config_id, &snapshot),
        upsert_global_default(&b, tenant_id, plugin_config_id, &backup),
    );

    assert!(
        r1.is_ok(),
        "connection a's upsert must not fail on a concurrent writer: {r1:?}"
    );
    assert!(
        r2.is_ok(),
        "connection b's upsert must not fail on a concurrent writer: {r2:?}"
    );

    let rows = proxmox_protection_default::Entity::find()
        .all(&a)
        .await
        .expect("load rows");
    assert_eq!(
        rows.len(),
        1,
        "exactly one row must survive concurrent upserts"
    );

    let matches_snapshot = rows[0].mode == ProtectionMode::Snapshot.as_str()
        && rows[0].backup_target_key.is_none()
        && rows[0].snapshot_timeout_seconds == Some(60)
        && rows[0].backup_timeout_seconds.is_none();
    let matches_backup = rows[0].mode == ProtectionMode::Backup.as_str()
        && rows[0].backup_target_key.as_deref() == Some("pve1:local:dir")
        && rows[0].snapshot_timeout_seconds.is_none()
        && rows[0].backup_timeout_seconds == Some(1200);
    assert!(
        matches_snapshot || matches_backup,
        "surviving row must coherently match one writer's policy, never a torn mix: {:?}",
        rows[0]
    );
}
