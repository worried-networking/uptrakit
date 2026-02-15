use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

mod m20260215_000001_initial;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260215_000001_initial::Migration)]
    }
}

/// Run all pending migrations on the local SSH agent database.
pub async fn run_migrations(db: &DatabaseConnection) -> std::result::Result<(), sea_orm::DbErr> {
    Migrator::up(db, None).await
}
