//! Tenant-isolation tests for `matching.rs` on a real in-memory SQLite DB.
//!
//! `MockDatabase` cannot prove tenant filtering (it ignores WHERE clauses), so
//! these tests run the shared-db + proxmox migrations and assert foreign rows'
//! post-state directly.
//!
//! Also hosts the shared real-SQLite helpers used by sibling SQLite test modules.

#![expect(
    clippy::expect_used,
    reason = "test-only module: panics are how tests learn about DB setup bugs"
)]

use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use time::OffsetDateTime;
use uptrakit_tenant_db::TenantDb;
use uuid::Uuid;

use crate::entity::proxmox_host_mapping;
use crate::matching::{manual_match, unmatch};

pub(crate) async fn setup_db() -> DatabaseConnection {
    // Tests never initialize a real master key; plaintext mode lets
    // `EncryptedPluginConfig::from_json` work without one. Safe to call
    // repeatedly.
    uptrakit_crypto::enable_plaintext_mode();
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    uptrakit_shared_db::migration::run_migrations_with_plugins(
        &db,
        crate::ProxmoxPlugin::controller_migrations,
    )
    .await
    .expect("shared + proxmox migrations should run");
    db
}

pub(crate) async fn insert_tenant(db: &DatabaseConnection) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    uptrakit_shared_db::entity::tenant::ActiveModel {
        id: Set(id),
        name: Set(format!("tenant-{id}")),
        slug: Set(id.to_string()),
        is_default: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert tenant");
    id
}

pub(crate) async fn insert_host(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    uptrakit_shared_db::entity::host::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        machine_id: Set(format!("machine-{id}")),
        hostname: Set(format!("host-{id}")),
        friendly_name: Set(format!("host-{id}")),
        os_type: Set(None),
        os_version: Set(None),
        architecture: Set(None),
        ip_address: Set(None),
        host_features: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert host");
    id
}

pub(crate) async fn insert_plugin_config(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    uptrakit_shared_db::entity::plugin_config::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        name: Set(format!("pve-{id}")),
        plugin_type: Set("infrastructure.proxmox".to_string()),
        config: Set(
            uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig::from_json(
                &serde_json::json!({}),
            )
            .expect("encrypt test config"),
        ),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: sea_orm::ActiveValue::NotSet,
        credential_updated_at: sea_orm::ActiveValue::NotSet,
    }
    .insert(db)
    .await
    .expect("insert plugin_config");
    id
}

async fn insert_mapping(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    host_id: Option<Uuid>,
    vmid: i32,
) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    proxmox_host_mapping::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        plugin_config_id: Set(plugin_config_id),
        host_id: Set(host_id),
        proxmox_node: Set("pve1".to_string()),
        proxmox_vmid: Set(vmid),
        proxmox_type: Set("qemu".to_string()),
        proxmox_name: Set(Some(format!("vm-{vmid}"))),
        proxmox_status: Set("running".to_string()),
        hostname: Set(None),
        ip_addresses: Set(None),
        machine_id: Set(None),
        match_method: Set(host_id.map(|_| "manual".to_string())),
        discovered_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert proxmox_host_mapping");
    id
}

async fn load_mapping(db: &DatabaseConnection, id: Uuid) -> proxmox_host_mapping::Model {
    proxmox_host_mapping::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("query mapping")
        .expect("mapping row exists")
}

/// Two-tenant world: tenant A (caller) and tenant B (victim).
/// Returns (db, tenant_a, tenant_b).
async fn two_tenant_db() -> (DatabaseConnection, Uuid, Uuid) {
    let db = setup_db().await;
    let tenant_a = insert_tenant(&db).await;
    let tenant_b = insert_tenant(&db).await;
    (db, tenant_a, tenant_b)
}

#[tokio::test]
async fn manual_match_rejects_foreign_tenant_mapping() {
    let (db, tenant_a, tenant_b) = two_tenant_db().await;
    let config_b = insert_plugin_config(&db, tenant_b).await;
    let host_b = insert_host(&db, tenant_b).await;
    let foreign_mapping = insert_mapping(&db, tenant_b, config_b, Some(host_b), 100).await;
    let host_a = insert_host(&db, tenant_a).await;

    let tenant_db = TenantDb::new(db.clone(), tenant_a);
    let err = manual_match(&tenant_db, foreign_mapping, host_a)
        .await
        .expect_err("foreign mapping must not be matchable");
    assert!(err.to_string().contains("not found"), "got: {err}");

    // Primary check: the foreign row is unchanged.
    let row = load_mapping(&db, foreign_mapping).await;
    assert_eq!(
        row.host_id,
        Some(host_b),
        "foreign mapping must keep its host"
    );
    assert_eq!(row.match_method.as_deref(), Some("manual"));
}

#[tokio::test]
async fn manual_match_rejects_foreign_tenant_host() {
    let (db, tenant_a, tenant_b) = two_tenant_db().await;
    let config_a = insert_plugin_config(&db, tenant_a).await;
    let own_mapping = insert_mapping(&db, tenant_a, config_a, None, 100).await;
    let host_b = insert_host(&db, tenant_b).await;

    let tenant_db = TenantDb::new(db.clone(), tenant_a);
    let err = manual_match(&tenant_db, own_mapping, host_b)
        .await
        .expect_err("foreign host must not be assignable");
    assert!(err.to_string().contains("not found"), "got: {err}");

    // Primary check: our mapping's host_id stays NULL.
    let row = load_mapping(&db, own_mapping).await;
    assert_eq!(row.host_id, None, "mapping must stay unmatched");
}

#[tokio::test]
async fn unmatch_rejects_foreign_tenant_mapping() {
    let (db, tenant_a, tenant_b) = two_tenant_db().await;
    let config_b = insert_plugin_config(&db, tenant_b).await;
    let host_b = insert_host(&db, tenant_b).await;
    let foreign_mapping = insert_mapping(&db, tenant_b, config_b, Some(host_b), 100).await;

    let tenant_db = TenantDb::new(db.clone(), tenant_a);
    let err = unmatch(&tenant_db, foreign_mapping)
        .await
        .expect_err("foreign mapping must not be unmatchable");
    assert!(err.to_string().contains("not found"), "got: {err}");

    // Primary check: the foreign mapping keeps its match.
    let row = load_mapping(&db, foreign_mapping).await;
    assert_eq!(
        row.host_id,
        Some(host_b),
        "foreign mapping must keep its host"
    );
}

#[tokio::test]
async fn unmatch_twice_is_ok_rows_affected_counts_matched_rows() {
    // Pins `rows_affected` counting matched rows, not changed rows: a
    // matched-but-unchanged row must not surface as "not found". Both
    // supported backends (SQLite, Postgres) report rows matched by WHERE.
    let db = setup_db().await;
    let tenant = insert_tenant(&db).await;
    let config = insert_plugin_config(&db, tenant).await;
    let host = insert_host(&db, tenant).await;
    let mapping = insert_mapping(&db, tenant, config, Some(host), 100).await;

    let tenant_db = TenantDb::new(db.clone(), tenant);
    unmatch(&tenant_db, mapping)
        .await
        .expect("first unmatch should succeed");
    unmatch(&tenant_db, mapping)
        .await
        .expect("second unmatch on already-unmatched mapping should also succeed");

    let row = load_mapping(&db, mapping).await;
    assert_eq!(row.host_id, None);
    assert_eq!(row.match_method, None);
}

#[tokio::test]
async fn manual_match_clears_conflicts_across_same_tenant_configs() {
    // Real-WHERE complement to the MockDatabase conflict test: an in-tenant
    // mapping holding the host is cleared regardless of plugin_config_id;
    // a foreign-tenant row is untouched.
    //
    // Deviation from the brief: `proxmox_host_mappings.host_id` carries a
    // global unique index (`uix_proxmox_hm_host_unique`, migration
    // `m20260417_000004_proxmox_hm_unique_host_id` in controller_migration.rs)
    // enforced by real SQLite (MockDatabase ignores it). Two rows can never
    // hold the same non-null host_id at rest, so the brief's fixture — two
    // simultaneous same-tenant conflict rows both set to `Some(host_a)` —
    // cannot be inserted; it fails with `UNIQUE constraint failed:
    // proxmox_host_mappings.host_id` at fixture-insert time, before
    // `manual_match` even runs. Reduced to a single conflict row (in a
    // different plugin_config from the target, preserving the
    // cross-plugin_config-id assertion) plus the foreign-tenant row, which
    // holds a *different* host_id (`host_b`) and was never a real conflict
    // candidate for `host_a`.
    let (db, tenant_a, tenant_b) = two_tenant_db().await;
    let config_a1 = insert_plugin_config(&db, tenant_a).await;
    let config_a2 = insert_plugin_config(&db, tenant_a).await;
    let config_b = insert_plugin_config(&db, tenant_b).await;
    let host_a = insert_host(&db, tenant_a).await;
    let host_b = insert_host(&db, tenant_b).await;

    let target = insert_mapping(&db, tenant_a, config_a1, None, 100).await;
    let conflict_other_config = insert_mapping(&db, tenant_a, config_a2, Some(host_a), 102).await;
    // Foreign row holding an unrelated host_id column must not be touched.
    let foreign_row = insert_mapping(&db, tenant_b, config_b, Some(host_b), 103).await;

    let tenant_db = TenantDb::new(db.clone(), tenant_a);
    manual_match(&tenant_db, target, host_a)
        .await
        .expect("match should succeed");

    assert_eq!(load_mapping(&db, target).await.host_id, Some(host_a));
    assert_eq!(
        load_mapping(&db, conflict_other_config).await.host_id,
        None,
        "cross-config same-tenant conflict must be cleared"
    );
    assert_eq!(
        load_mapping(&db, foreign_row).await.host_id,
        Some(host_b),
        "foreign tenant's mapping must be untouched"
    );
}
