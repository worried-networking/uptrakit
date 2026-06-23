//! Phase 3: Database connection, migrations, and default-tenant load.
//!
//! Opens the database, runs pending migrations, and resolves the default tenant.
//! The resulting [`Persistence`] value is threaded into all downstream boot
//! phases that require a live database connection.

use crate::boot::config::BootConfig;
use crate::boot::directories::AppLayout;

/// Output of Phase 3: an open database connection with its resolved URL and the
/// default tenant identifier.
pub(crate) struct Persistence {
    pub db: sea_orm::DatabaseConnection,
    pub url: String,
    pub default_tenant_id: uuid::Uuid,
}

/// Phase 3: open the database, run migrations, and load the default tenant.
///
/// Uses `cfg.booted.runtime.db.url` and `cfg.booted.runtime.db.pool_size` for
/// the connection parameters, and `layout.app_dirs.state_dir()` to derive the
/// on-disk SQLite path when the URL is empty.
pub(crate) async fn open(cfg: &BootConfig, layout: &AppLayout) -> crate::Result<Persistence> {
    let runtime = &cfg.booted.runtime;
    let db_init = crate::boot::init::init_database(
        &runtime.db.url,
        runtime.db.pool_size,
        layout.app_dirs.state_dir(),
    )
    .await?;
    let db = db_init.conn;
    let url = db_init.url;
    let default_tenant_id = db_init.default_tenant.id;
    tracing::info!(%default_tenant_id, "loaded default tenant");
    Ok(Persistence {
        db,
        url,
        default_tenant_id,
    })
}
