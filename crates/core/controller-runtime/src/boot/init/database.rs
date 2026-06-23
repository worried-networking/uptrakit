//! Phase 3: Database initialization.

use rootcause::prelude::*;

use crate::AppError;

/// Database initialization result.
pub(crate) struct DatabaseInit {
    pub conn: sea_orm::DatabaseConnection,
    pub default_tenant: uptrakit_shared_db::entity::tenant::Model,
    /// The resolved database URL (for credential delivery to external services).
    pub url: String,
}

/// Connect to the database, run migrations, and load the default tenant.
///
/// `db_url` is taken directly from `runtime.db.url` (TOML config).
/// `pool_size` is taken from `runtime.db.pool_size`.
pub(crate) async fn init_database(
    db_url: &str,
    pool_size: u32,
    state_dir: &std::path::Path,
) -> crate::Result<DatabaseInit> {
    let db_config = crate::db::DbConfig::from_args(
        if db_url.is_empty() {
            None
        } else {
            Some(db_url.to_owned())
        },
        state_dir,
        pool_size,
    )
    .context(AppError::Database)?;
    tracing::info!(
        max_connections = db_config.max_connections,
        "connecting to database: {}",
        crate::db::sanitize_url(&db_config.url)
    );
    let db_conn = crate::db::connect(&db_config)
        .await
        .context(AppError::Database)?;

    tracing::info!("running database migrations");
    crate::migration::run_migrations(&db_conn)
        .await
        .context(AppError::Database)?;
    tracing::info!("database initialized successfully");

    let default_tenant = {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        use uptrakit_shared_db::entity::{prelude::Tenant, tenant};

        Tenant::find()
            .filter(tenant::Column::IsDefault.eq(true))
            .filter(tenant::Column::DeactivatedAt.is_null())
            .one(&db_conn)
            .await
            .context(AppError::Database)?
            .ok_or_else(|| report!(AppError::Database))?
    };

    Ok(DatabaseInit {
        conn: db_conn,
        default_tenant,
        url: db_config.url,
    })
}
