use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

mod m20260215_000001_initial;
mod m20260222_000002_add_machine_id;
mod m20260224_000003_add_sudo_columns;
mod m20260302_000001_convert_ssh_host_timestamps;
mod m20260302_000002_ensure_machine_id_nullable;
mod m20260306_000001_add_pve_columns;
mod m20260307_000001_add_pve_node_name;
mod m20260307_000002_pending_proxmox_matches;
mod m20260308_000003_ssh_host_uuid_columns;
mod m20260310_000001_data_encryption_keys;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        // Core agent-ssh schema migrations.  Keep the legacy
        // `m20260307_000002_pending_proxmox_matches` entry — existing
        // databases already have it recorded in `seaql_migrations` and the
        // Proxmox plugin's own `CreateProxmoxPendingMatches` migration handles
        // the rename + data migration from the old table on first run.
        let mut migrations: Vec<Box<dyn MigrationTrait>> = vec![
            Box::new(m20260215_000001_initial::Migration),
            Box::new(m20260222_000002_add_machine_id::Migration),
            Box::new(m20260224_000003_add_sudo_columns::Migration),
            Box::new(m20260302_000001_convert_ssh_host_timestamps::Migration),
            Box::new(m20260302_000002_ensure_machine_id_nullable::Migration),
            Box::new(m20260310_000001_data_encryption_keys::Migration),
            Box::new(m20260306_000001_add_pve_columns::Migration),
            Box::new(m20260307_000001_add_pve_node_name::Migration),
            Box::new(m20260307_000002_pending_proxmox_matches::Migration),
            Box::new(m20260308_000003_ssh_host_uuid_columns::Migration),
        ];

        // Append plugin-owned migrations so they run after the core schema is
        // in place.  Each plugin migration has a unique name tracked in
        // `seaql_migrations`, so already-applied migrations are skipped.
        //
        // `CreateProxmoxHostState`       — creates `proxmox_host_state`,
        //   migrates PVE state from legacy `ssh_hosts` columns if present.
        // `CreateProxmoxPendingMatches`  — creates `proxmox_pending_matches`,
        //   migrates data from the legacy `pending_proxmox_matches` table.
        for plugin in uptrakit_plugin_infrastructure_registry::create_agent_infra_plugins() {
            migrations.extend(plugin.service_migrations());
        }

        migrations
    }
}

/// Run all pending migrations on the local SSH agent database.
pub async fn run_migrations(db: &DatabaseConnection) -> std::result::Result<(), sea_orm::DbErr> {
    Migrator::up(db, None).await
}
