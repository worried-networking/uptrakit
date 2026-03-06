use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

mod m20260215_000001_initial;
mod m20260222_000002_add_machine_id;
mod m20260224_000003_add_sudo_columns;
mod m20260302_000001_convert_ssh_host_timestamps;
mod m20260302_000002_ensure_machine_id_nullable;
mod m20260306_000001_add_pve_columns;
mod m20260310_000001_data_encryption_keys;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260215_000001_initial::Migration),
            Box::new(m20260222_000002_add_machine_id::Migration),
            Box::new(m20260224_000003_add_sudo_columns::Migration),
            Box::new(m20260302_000001_convert_ssh_host_timestamps::Migration),
            Box::new(m20260302_000002_ensure_machine_id_nullable::Migration),
            Box::new(m20260310_000001_data_encryption_keys::Migration),
            Box::new(m20260306_000001_add_pve_columns::Migration),
        ]
    }
}

/// Run all pending migrations on the local SSH agent database.
pub async fn run_migrations(db: &DatabaseConnection) -> std::result::Result<(), sea_orm::DbErr> {
    Migrator::up(db, None).await
}
