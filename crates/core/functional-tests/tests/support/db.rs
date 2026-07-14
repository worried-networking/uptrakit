use sea_orm::{Database, DatabaseConnection};

/// In-memory SQLite + combined core+proxmox migrations in one pass.
///
/// Core and plugin migrations must run through one
/// `run_migrations_with_plugins` call so SeaORM sees the complete migration
/// list; running them in separate steps would double-run the schema.
pub(crate) async fn setup_test_db() -> DatabaseConnection {
    // EncryptedString columns appear indirectly in plugin_config payloads;
    // plaintext mode keeps them inspectable.
    uptrakit_crypto::enable_plaintext_mode();

    let db = Database::connect("sqlite::memory:")
        .await
        .expect("open in-memory SQLite");

    uptrakit_shared_db::migration::run_migrations_with_plugins(
        &db,
        uptrakit_plugin_infrastructure_proxmox::ProxmoxPlugin::controller_migrations,
    )
    .await
    .expect("combined migrations");

    db
}
