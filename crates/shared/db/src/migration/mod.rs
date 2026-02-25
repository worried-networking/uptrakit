use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

mod m20260209_000001_initial;
mod m20260224_000001_mqtt_ha_discovery;
mod m20260225_000001_rename_docker_provider;
mod m20260225_000002_phs_discovery_only;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260209_000001_initial::Migration),
            Box::new(m20260224_000001_mqtt_ha_discovery::Migration),
            Box::new(m20260225_000001_rename_docker_provider::Migration),
            Box::new(m20260225_000002_phs_discovery_only::Migration),
        ]
    }
}

/// Run all pending migrations.
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    Migrator::up(db, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};

    #[tokio::test]
    async fn migrations_run_on_empty_sqlite() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();
        db.execute_unprepared("SELECT count(*) FROM software_items")
            .await
            .unwrap();
    }
}
