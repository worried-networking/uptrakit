//! Real-key acceptance test: aliased-read decryption (task-7-brief Step 2).
//!
//! `crate::executors::queries::query_agent_assignment_rows` selects
//! `plugin_configs.config` under an *aliased* column name
//! (`column_as(plugin_config::Column::Config, "profile_config")`). Under the
//! old (pre-`encrypted_column!`) registry-based decryption mechanism, an
//! aliased read like this decrypted with the wrong AAD, because AAD lookup
//! was keyed off the result column name rather than the source table/column.
//! `encrypted_column!`-generated newtypes decode via `TryGetable` with a
//! compile-time AAD baked into the type itself, so the alias is irrelevant.
//! This test proves that end to end through the *real* query (not a
//! reimplementation of it).
//!
//! This lives in its own `tests/` integration target -- its own compiled
//! binary / process -- rather than inline in `crates/core/scheduler-runtime`,
//! because crypto mode (master key, DEK ring) is process-global via
//! `OnceLock`, and in-crate scheduler tests elsewhere call
//! `enable_plaintext_mode()` (see `executors/service_cert_check.rs`). Mixing
//! a real-key test with plaintext-mode tests in one process would be
//! order-dependent and unsound. Gated behind `test-support` (see
//! `Cargo.toml`'s `[[test]]` stanza) since it needs the `test_support`
//! module to reach otherwise `pub(crate)` query internals; `--all-features`
//! runs it, a bare `cargo test -p uptrakit-scheduler-runtime` skips it
//! cleanly.

#![expect(
    clippy::expect_used,
    reason = "integration test file: seed/fixture helpers outside #[tokio::test] fns panic on failure, matching other tests/*.rs integration files in this workspace"
)]

use sea_orm::{ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, service, service_host,
    software_item, tenant,
};
use uuid::Uuid;
use zeroize::Zeroizing;

async fn setup_test_db() -> DatabaseConnection {
    let opt = ConnectOptions::new("sqlite::memory:");
    let db = Database::connect(opt)
        .await
        .expect("connect sqlite::memory:");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("run migrations");
    db
}

async fn insert_tenant(db: &DatabaseConnection, tenant_id: Uuid) {
    let now = OffsetDateTime::now_utc();
    tenant::ActiveModel {
        id: Set(tenant_id),
        name: Set("test".to_string()),
        slug: Set(tenant_id.to_string()),
        is_default: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert tenant");
}

async fn insert_service(db: &DatabaseConnection, tenant_id: Uuid, service_id: Uuid) {
    let now = OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(service_id),
        tenant_id: Set(tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set("agent-host".to_string()),
        friendly_name: Set("Agent".to_string()),
        ip_address: Set(None),
        status: Set(service::ServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("secret-{service_id}")),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        enrollment_token_id: Set(None),
        cert_lifetime_hours: Set(None),
        service_app_name: Set(None),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .expect("insert service");
}

async fn insert_host(db: &DatabaseConnection, tenant_id: Uuid, host_id: Uuid) {
    let now = OffsetDateTime::now_utc();
    host::ActiveModel {
        id: Set(host_id),
        tenant_id: Set(tenant_id),
        machine_id: Set(format!("machine-{host_id}")),
        hostname: Set(format!("host-{host_id}")),
        friendly_name: Set(format!("Host {host_id}")),
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
}

async fn insert_software_item(db: &DatabaseConnection, tenant_id: Uuid, software_item_id: Uuid) {
    let now = OffsetDateTime::now_utc();
    software_item::ActiveModel {
        id: Set(software_item_id),
        tenant_id: Set(tenant_id),
        name: Set("Acceptance Software".to_string()),
        featured: Set(false),
        icon_url: Set(None),
        last_checked_at: Set(None),
        awaiting_restart_timeout: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert software_item");
}

/// Insert a `plugin_configs` row whose `config` is real-key-encrypted, plus
/// a `host_software_item`/`host_software_item_plugin` pair that links a host
/// to that config -- the exact join path
/// `query_agent_assignment_rows` walks (`host_software_item_plugin` ->
/// `plugin_config` LEFT JOIN, aliased as `profile_config`).
async fn insert_plugin_assignment_with_config(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
    seeded_json: &str,
) {
    let now = OffsetDateTime::now_utc();
    plugin_config::ActiveModel {
        id: Set(plugin_config_id),
        tenant_id: Set(tenant_id),
        name: Set("acceptance-plugin-config".to_string()),
        plugin_type: Set("acceptance-plugin-type".to_string()),
        config: Set(
            EncryptedPluginConfig::new(seeded_json.to_string()).expect("encrypt seed config")
        ),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        credential_updated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert plugin_config");

    let host_software_item_id = host_software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        qualifier: Set(None),
        plugin_config_id: Set(Some(plugin_config_id)),
        package_identifier: Set(None),
        installed_version: Set(None),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("unknown".to_string()),
        deactivated_at: Set(None),
        last_discovered_at: Set(None),
        discovery_source: Set(None),
        missing_since: Set(None),
    }
    .insert(db)
    .await
    .expect("insert host_software_item")
    .id;

    host_software_item_plugin::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(host_software_item_id),
        plugin_config_id: Set(Some(plugin_config_id)),
        plugin_type: Set("acceptance-plugin-type".to_string()),
        role: Set("detect_version".to_string()),
        ordinal: Set(0),
        package_identifier: Set("acceptance-package".to_string()),
        config: Set(None),
        execution_site: Set("agent".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert host_software_item_plugin");
}

#[tokio::test]
async fn aliased_select_decrypts_plugin_config_with_correct_aad() {
    uptrakit_crypto::init_master_key(Zeroizing::new([0x42u8; 32])).expect("init master key");

    let db = setup_test_db().await;
    let tenant_id = Uuid::now_v7();
    let service_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let software_item_id = Uuid::now_v7();
    let plugin_config_id = Uuid::now_v7();
    let seeded_json = r#"{"auth_token":"aliased-decrypt-secret"}"#;

    insert_tenant(&db, tenant_id).await;
    insert_service(&db, tenant_id, service_id).await;
    insert_host(&db, tenant_id, host_id).await;
    insert_software_item(&db, tenant_id, software_item_id).await;
    insert_plugin_assignment_with_config(
        &db,
        tenant_id,
        host_id,
        software_item_id,
        plugin_config_id,
        seeded_json,
    )
    .await;

    let now = OffsetDateTime::now_utc();
    service_host::ActiveModel {
        service_id: Set(service_id),
        host_id: Set(host_id),
        linked_at: Set(now),
    }
    .insert(&db)
    .await
    .expect("insert service_host");

    let rows = uptrakit_scheduler_runtime::test_support::query_agent_assignment_rows(
        &db,
        tenant_id,
        &["detect_version"],
    )
    .await
    .expect("query agent assignment rows");

    assert_eq!(rows.len(), 1, "expected exactly one assignment row");
    let row = &rows[0];

    let profile_config = row
        .profile_config
        .as_ref()
        .expect("profile_config must be present for a linked plugin_config");
    assert_eq!(
        profile_config.as_json(),
        &serde_json::json!({"auth_token": "aliased-decrypt-secret"}),
        "aliased select must decrypt under the correct compile-time AAD"
    );
}
