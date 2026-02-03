// Compile-time check: at least one database backend must be enabled
#[cfg(not(any(feature = "db-sqlite", feature = "db-postgres", feature = "db-mysql")))]
compile_error!(
    "At least one database backend feature must be enabled: db-sqlite, db-postgres, or db-mysql"
);

pub mod error;

pub use error::{DbError, Result};

use rootcause::prelude::*;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::time::Duration;

/// Initialize database connection with the given URL
pub async fn connect(db_url: &str) -> Result<DatabaseConnection> {
    let mut opt = ConnectOptions::new(db_url.to_owned());
    opt.max_connections(10)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(8))
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(8))
        .sqlx_logging(false);

    Database::connect(opt)
        .await
        .context_to::<DbError>()
        .context(DbError::Connection("connecting to database".to_string()))
}

/// Sanitize database URL for logging (removes credentials)
pub fn sanitize_url(url: &str) -> String {
    if let Some(at_pos) = url.find('@')
        && let Some(proto_end) = url.find("://")
    {
        let protocol = &url[..proto_end + 3];
        let rest = &url[at_pos..];
        return format!("{protocol}***{rest}");
    }
    url.to_string()
}
