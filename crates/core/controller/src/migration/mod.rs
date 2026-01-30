use crate::db::{DbError, Result};
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

mod m20260129_000001_initial;
mod m20260129_000002_create_users;
mod m20260129_000003_create_rbac;
mod m20260129_000004_create_sessions;
mod m20260129_000005_create_settings;
mod m20260129_000006_create_agents;
mod m20260130_000001_create_agent_certificates;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260129_000001_initial::Migration),
            Box::new(m20260129_000002_create_users::Migration),
            Box::new(m20260129_000003_create_rbac::Migration),
            Box::new(m20260129_000004_create_sessions::Migration),
            Box::new(m20260129_000005_create_settings::Migration),
            Box::new(m20260129_000006_create_agents::Migration),
            Box::new(m20260130_000001_create_agent_certificates::Migration),
        ]
    }
}

/// Run all pending migrations
pub async fn run_migrations(db: &DatabaseConnection) -> Result<()> {
    Migrator::up(db, None)
        .await
        .context_to::<DbError>()
        .context(DbError::Migration(
            "running database migrations".to_string(),
        ))
}
