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
pub(crate) async fn init_database(
    args: &crate::cli::Args,
    state_dir: &std::path::Path,
) -> crate::Result<DatabaseInit> {
    let db_config =
        crate::db::DbConfig::from_args(args.db_url.clone(), state_dir, args.db_max_connections)
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

/// Initialize a separate database connection for audit log storage.
///
/// Connects to the provided URL, runs the standard migrations (extra
/// empty application tables in the audit database are harmless), and
/// returns the connection for use by the audit log backend.
pub(crate) async fn init_audit_database(
    url: &str,
    max_connections: u32,
) -> crate::Result<sea_orm::DatabaseConnection> {
    let db_config = crate::db::DbConfig {
        url: url.to_string(),
        max_connections,
    };
    tracing::info!(
        max_connections,
        "connecting to audit log database: {}",
        crate::db::sanitize_url(url)
    );
    let conn = crate::db::connect(&db_config)
        .await
        .context(AppError::Database)?;

    tracing::info!("running audit log database migrations");
    crate::migration::run_migrations(&conn)
        .await
        .context(AppError::Database)?;
    tracing::info!("audit log database initialized");

    Ok(conn)
}
