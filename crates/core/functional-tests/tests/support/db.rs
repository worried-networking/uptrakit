use sea_orm::{Database, DatabaseConnection};

/// In-memory SQLite + combined core+proxmox migrations in one pass.
///
/// `CombinedMigrator` uses a thread-local to merge plugin migrations with
/// core migrations; calling core and plugin migrations in separate steps
/// would double-run the schema, so this helper performs both in one
/// `run_migrations_with_plugins` call.
pub(crate) async fn setup_test_db() -> DatabaseConnection {
    // EncryptedString columns appear indirectly in plugin_config payloads;
    // plaintext mode keeps them inspectable.
    uptrakit_crypto::enable_plaintext_mode();

    let db = Database::connect("sqlite::memory:")
        .await
        .expect("open in-memory SQLite");

    uptrakit_shared_db::migration::run_migrations_with_plugins(
        &db,
        uptrakit_plugin_infrastructure_proxmox::ProxmoxPlugin::controller_migrations(),
    )
    .await
    .expect("combined migrations");

    db
}
