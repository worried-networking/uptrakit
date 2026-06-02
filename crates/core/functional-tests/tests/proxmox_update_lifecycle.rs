#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "functional test infrastructure: panics acceptable in test helpers and assertions"
)]

mod support;

use sea_orm::ConnectionTrait;

#[tokio::test]
async fn setup_test_db_runs_core_and_proxmox_migrations() {
    let db = support::db::setup_test_db().await;
    // proxmox_host_mapping is a proxmox-side table; existence proves the
    // plugin migration ran. tenant is core-side; existence proves the core
    // migration ran. Both in one pass.
    db.execute_unprepared("SELECT id FROM proxmox_host_mappings LIMIT 1")
        .await
        .expect("proxmox_host_mappings table must exist");
    db.execute_unprepared("SELECT id FROM tenants LIMIT 1")
        .await
        .expect("tenants table must exist");
}
