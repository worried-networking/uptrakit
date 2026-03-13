use crate::db::{DbError, Result};
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;

/// Run all pending migrations, including plugin-contributed controller migrations.
pub async fn run_migrations(db: &DatabaseConnection) -> Result<()> {
    let plugin_migrations = uptrakit_plugin_infrastructure_registry::all_controller_migrations();
    uptrakit_shared_db::migration::run_migrations_with_plugins(db, plugin_migrations)
        .await
        .context_to::<DbError>()
        .context(DbError::Migration(
            "running database migrations".to_string(),
        ))
}
