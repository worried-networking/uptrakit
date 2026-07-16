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
mod m20260507_000001_add_routeros_host_config;

pub(crate) struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        // Keep legacy m20260307_000002_pending_proxmox_matches — existing
        // databases already have it recorded in seaql_migrations.
        let migrations: Vec<Box<dyn MigrationTrait>> = vec![
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
            Box::new(m20260507_000001_add_routeros_host_config::Migration),
        ];
        let mut migrations = migrations;
        // Plugin agent migrations run AFTER the runtime list: the legacy
        // pending_proxmox_matches table (m20260307_000002 above) must exist
        // before CreateProxmoxPendingMatches copies its rows. Do not reorder.
        migrations.extend(
            uptrakit_plugin_infrastructure_registry::all_descriptors()
                .into_iter()
                .filter_map(|d| d.agent_migrations)
                .flat_map(|f| f()),
        );
        migrations
    }
}

pub(crate) async fn run_migrations(
    db: &DatabaseConnection,
) -> std::result::Result<(), sea_orm::DbErr> {
    Migrator::up(db, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn table_names(db: &DatabaseConnection) -> Vec<String> {
        use sea_orm::{ConnectionTrait, Statement};
        let rows = db
            .query_all_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                // Raw introspection query: sqlite_master has no SeaORM entity —
                // approved read-only exception, mirrors db-migrate's coverage test.
                "SELECT name FROM sqlite_master WHERE type = 'table'".to_string(),
            ))
            .await
            .expect("query sqlite_master");
        rows.iter()
            .map(|r| r.try_get_by_index::<String>(0).expect("table name"))
            .collect()
    }

    #[tokio::test]
    async fn run_migrations_creates_plugin_agent_tables() {
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        run_migrations(&db).await.expect("migrations run");
        let tables = table_names(&db).await;
        assert!(tables.iter().any(|t| t == "proxmox_pending_matches"));
        assert!(tables.iter().any(|t| t == "proxmox_host_state"));
        // Idempotency: second run is a no-op.
        run_migrations(&db).await.expect("second run is a no-op");
    }
}
