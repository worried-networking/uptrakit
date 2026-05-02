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
mod m20260313_000001_drop_ssh_host_is_pve_node;
mod m20260322_000001_ssh_hosts_lower_name_index;

pub(crate) struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        // Core agent-ssh schema migrations.  Keep the legacy
        // `m20260307_000002_pending_proxmox_matches` entry — existing
        // databases already have it recorded in `seaql_migrations` and the
        // Proxmox plugin's own `CreateProxmoxPendingMatches` migration handles
        // the rename + data migration from the old table on first run.
        #[expect(
            clippy::allow_attributes,
            clippy::allow_attributes_without_reason,
            reason = "feature-conditional: `mut` is needed when plugin migrations are appended below; `#[expect]` would fail under feature variants where the binding is never mutated"
        )]
        #[allow(unused_mut)]
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
            Box::new(m20260313_000001_drop_ssh_host_is_pve_node::Migration),
            Box::new(m20260322_000001_ssh_hosts_lower_name_index::Migration),
        ];

        // Append plugin-owned service migrations so they run after the core
        // schema is in place.  Each plugin migration has a unique name tracked
        // in `seaql_migrations`, so already-applied migrations are skipped.
        //
        // `CreateProxmoxHostState`       — creates `proxmox_host_state`,
        //   migrates PVE state from legacy `ssh_hosts` columns if present.
        // `CreateProxmoxPendingMatches`  — creates `proxmox_pending_matches`,
        //   migrates data from the legacy `pending_proxmox_matches` table.
        //
        // TODO: Once the Proxmox plugin's InfraSlot is wired up in the
        // descriptor, collect service migrations from InfraBundle or via a
        // dedicated catalog method. Until then, existing databases already
        // have these migrations recorded in `seaql_migrations`.
        let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        if let Ok(catalog) = uptrakit_plugin_infrastructure_registry::build_catalog(&catalog_config)
        {
            let bundles = catalog.create_infra_bundles(&catalog_config);
            // InfraBundle service migrations will be available once the
            // Proxmox plugin registers its InfraSlot in the descriptor.
            let _ = &bundles;
        }

        migrations
    }
}

/// Run all pending migrations on the local SSH agent database.
pub(crate) async fn run_migrations(
    db: &DatabaseConnection,
) -> std::result::Result<(), sea_orm::DbErr> {
    Migrator::up(db, None).await
}
