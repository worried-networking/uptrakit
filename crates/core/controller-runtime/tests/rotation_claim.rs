//! Real-key acceptance test: rotation claim (task-7-brief Step 3, spec §9
//! test 4). After a full `reencrypt_to_v3` pass with the DEK ring
//! initialized, every row of the three plugin-config tables must report
//! `needs_v3_upgrade() == false` -- `ENC:v3:` envelope encryption IS the
//! rotation story here (a master-key rotation re-wraps DEKs, not individual
//! column values), so "fully migrated to v3" is the meaningful claim to
//! prove.
//!
//! **Deviation from the literal brief wording**: the brief's Step 3 says
//! "in the reencrypt test module ... with the ring initialized". That is not
//! safely satisfiable in `crates/core/controller-runtime/src/reencrypt.rs`'s
//! existing `#[cfg(test)] mod tests`: several pre-existing tests there (e.g.
//! `plugin_config_upgrade_is_idempotent_for_data_integrity`,
//! `setting_v2_gets_upgraded`) hard-depend on `encrypt_str` producing
//! `ENC:v2:` (no ring) at the exact moment their own seed-time
//! `Encrypted*::new`/`EncryptedString::new` calls execute -- `cargo test`
//! compiles all of a crate's `#[cfg(test)]` unit tests into one shared
//! `--lib` binary with process-global `OnceLock` crypto state, so
//! initializing the ring anywhere in that module would flip those
//! assertions from `1` to `0` (their rows would already be `ENC:v3:` by
//! construction, and the upgrade helpers skip already-v3 rows). This mirrors
//! the brief's own stated rationale for isolating Step 2
//! (`crates/core/scheduler-runtime/tests/aliased_decrypt.rs`) -- "crypto
//! mode is process-global ... a tests/ target is its own process" -- applied
//! with equal force to Step 3's ring requirement. See task-7-report.md for
//! the full writeup.

#![expect(
    clippy::expect_used,
    reason = "integration test file: seed/fixture helpers outside #[tokio::test] fns panic on failure, matching other tests/*.rs integration files in this workspace"
)]

use sea_orm::sea_query::{Expr as SqExpr, Query};
use sea_orm::{
    ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    Set,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    instance_plugin_setting, plugin_config, plugin_type_setting, tenant,
};
use uuid::Uuid;
use zeroize::Zeroizing;

async fn insert_tenant(db: &DatabaseConnection, tenant_id: Uuid) {
    let now = OffsetDateTime::now_utc();
    tenant::ActiveModel {
        id: Set(tenant_id),
        name: Set("test".to_string()),
        slug: Set(tenant_id.to_string()),
        is_default: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert tenant");
}

/// Insert a legacy plaintext `plugin_configs` row via a raw `sea_query`
/// insert, bypassing the `EncryptedPluginConfig` newtype (which always
/// encrypts on construction).
async fn insert_plaintext_plugin_config(
    db: &DatabaseConnection,
    id: Uuid,
    plaintext_json: &str,
    now: OffsetDateTime,
) {
    let insert = Query::insert()
        .into_table(plugin_config::Entity)
        .columns([
            plugin_config::Column::Id,
            plugin_config::Column::TenantId,
            plugin_config::Column::Name,
            plugin_config::Column::PluginType,
            plugin_config::Column::Config,
            plugin_config::Column::Enabled,
            plugin_config::Column::CreatedAt,
            plugin_config::Column::UpdatedAt,
            plugin_config::Column::DeactivatedAt,
            plugin_config::Column::CredentialUpdatedAt,
        ])
        .values_panic([
            SqExpr::value(id),
            SqExpr::value(Uuid::nil()),
            SqExpr::value(format!("rotation-plugin-config-{id}")),
            SqExpr::value("rotation-plugin-type"),
            SqExpr::value(plaintext_json),
            SqExpr::value(true),
            SqExpr::value(now),
            SqExpr::value(now),
            SqExpr::value(sea_orm::Value::TimeDateTimeWithTimeZone(None)),
            SqExpr::value(sea_orm::Value::TimeDateTimeWithTimeZone(None)),
        ])
        .to_owned();

    db.execute(&insert)
        .await
        .expect("insert legacy plaintext plugin_config row");
}

/// Insert a legacy plaintext `plugin_type_settings` row via a raw
/// `sea_query` insert.
async fn insert_plaintext_plugin_type_setting(
    db: &DatabaseConnection,
    id: Uuid,
    plaintext_json: &str,
    now: OffsetDateTime,
) {
    let insert = Query::insert()
        .into_table(plugin_type_setting::Entity)
        .columns([
            plugin_type_setting::Column::Id,
            plugin_type_setting::Column::TenantId,
            plugin_type_setting::Column::PluginType,
            plugin_type_setting::Column::Config,
            plugin_type_setting::Column::CreatedAt,
            plugin_type_setting::Column::UpdatedAt,
        ])
        .values_panic([
            SqExpr::value(id),
            SqExpr::value(Uuid::nil()),
            SqExpr::value("rotation-plugin-type"),
            SqExpr::value(plaintext_json),
            SqExpr::value(now),
            SqExpr::value(now),
        ])
        .to_owned();

    db.execute(&insert)
        .await
        .expect("insert legacy plaintext plugin_type_setting row");
}

/// Insert a legacy plaintext `instance_plugin_setting` row via a raw
/// `sea_query` insert.
async fn insert_plaintext_instance_plugin_setting(
    db: &DatabaseConnection,
    plugin_type_id: &str,
    plaintext_json: &str,
    now: OffsetDateTime,
) {
    let insert = Query::insert()
        .into_table(instance_plugin_setting::Entity)
        .columns([
            instance_plugin_setting::Column::PluginTypeId,
            instance_plugin_setting::Column::Enabled,
            instance_plugin_setting::Column::Config,
            instance_plugin_setting::Column::UpdatedAt,
        ])
        .values_panic([
            SqExpr::value(plugin_type_id),
            SqExpr::value(true),
            SqExpr::value(plaintext_json),
            SqExpr::value(now),
        ])
        .to_owned();

    db.execute(&insert)
        .await
        .expect("insert legacy plaintext instance_plugin_setting row");
}

#[tokio::test]
async fn full_reencrypt_pass_leaves_no_row_needing_v3_upgrade() {
    uptrakit_crypto::init_master_key(Zeroizing::new([0x42u8; 32])).expect("init master key");
    let data_key = uptrakit_crypto::generate_data_key().expect("generate data key");
    let key_id = data_key.key_id.clone();
    let mut keys = std::collections::HashMap::new();
    keys.insert(key_id.clone(), data_key.key);
    let ring = uptrakit_crypto::DataKeyRing::new(keys, key_id).expect("build data key ring");
    uptrakit_crypto::init_data_key_ring(ring).expect("init data key ring");
    assert!(uptrakit_crypto::data_key_ring_available());

    uptrakit_controller_runtime::test_support::register_column_aad_mappings();

    let opt = ConnectOptions::new("sqlite::memory:");
    let db = Database::connect(opt)
        .await
        .expect("connect sqlite::memory:");
    uptrakit_controller_runtime::test_support::run_migrations(&db)
        .await
        .expect("run migrations");

    let tenant_id = Uuid::nil();
    insert_tenant(&db, tenant_id).await;

    let now = OffsetDateTime::now_utc();
    let plugin_config_id = Uuid::now_v7();
    insert_plaintext_plugin_config(&db, plugin_config_id, r#"{"token":"rotation-secret"}"#, now)
        .await;

    let plugin_type_setting_id = Uuid::now_v7();
    insert_plaintext_plugin_type_setting(
        &db,
        plugin_type_setting_id,
        r#"{"default_timeout":30}"#,
        now,
    )
    .await;

    insert_plaintext_instance_plugin_setting(
        &db,
        "rotation-instance-plugin",
        r#"{"api_key":"rotation-instance"}"#,
        now,
    )
    .await;

    uptrakit_controller_runtime::test_support::reencrypt_to_v3(&db).await;

    let plugin_config_row =
        uptrakit_shared_db::entity::prelude::PluginConfig::find_by_id(plugin_config_id)
            .one(&db)
            .await
            .expect("query plugin_config")
            .expect("plugin_config row exists");
    assert!(
        !plugin_config_row.config.needs_v3_upgrade(),
        "plugin_configs.config must be fully migrated to ENC:v3 after a full pass with the ring initialized"
    );

    let plugin_type_setting_row =
        uptrakit_shared_db::entity::prelude::PluginTypeSetting::find_by_id(plugin_type_setting_id)
            .one(&db)
            .await
            .expect("query plugin_type_setting")
            .expect("plugin_type_setting row exists");
    assert!(
        !plugin_type_setting_row.config.needs_v3_upgrade(),
        "plugin_type_settings.config must be fully migrated to ENC:v3 after a full pass with the ring initialized"
    );

    let instance_plugin_setting_row =
        uptrakit_shared_db::entity::prelude::InstancePluginSetting::find_by_id(
            "rotation-instance-plugin".to_string(),
        )
        .one(&db)
        .await
        .expect("query instance_plugin_setting")
        .expect("instance_plugin_setting row exists");
    assert!(
        !instance_plugin_setting_row.config.needs_v3_upgrade(),
        "instance_plugin_setting.config must be fully migrated to ENC:v3 after a full pass with the ring initialized"
    );
}
