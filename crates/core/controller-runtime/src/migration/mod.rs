use crate::db::{DbError, Result};
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;

/// Run all pending migrations, including plugin-contributed controller migrations.
///
/// Collects migrations directly from compiled-in plugin descriptors rather than
/// requiring a full `PluginCatalog` instance (which needs HTTP clients and
/// cancellation tokens that are unavailable at migration time).
pub(crate) async fn run_migrations(db: &DatabaseConnection) -> Result<()> {
    let plugin_migrations = uptrakit_plugin_infrastructure_registry::all_descriptors()
        .into_iter()
        .filter_map(|d| d.migrations)
        .flat_map(|f| f())
        .collect();
    uptrakit_shared_db::migration::run_migrations_with_plugins(db, plugin_migrations)
        .await
        .context_to::<DbError>()
        .context(DbError::Migration(
            "running database migrations".to_string(),
        ))
}
