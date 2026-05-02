// Compile-time check: at least one database backend must be enabled
#[cfg(not(any(feature = "db-sqlite", feature = "db-postgres")))]
compile_error!("At least one database backend feature must be enabled: db-sqlite or db-postgres");

pub(crate) mod config;
mod error;

pub(crate) use config::{DEFAULT_MAX_CONNECTIONS, DbConfig};
pub(crate) use error::{DbError, Result};

use rootcause::prelude::*;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::time::Duration;

/// Initialize a database connection pool using the provided configuration.
pub(crate) async fn connect(config: &DbConfig) -> Result<DatabaseConnection> {
    #[cfg(feature = "db-sqlite")]
    if config.url.starts_with("sqlite") {
        return connect_sqlite(config).await;
    }

    let mut opt = ConnectOptions::new(config.url.clone());
    opt.max_connections(config.max_connections)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(8))
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(300))
        .sqlx_logging(false);

    Database::connect(opt)
        .await
        .context_to::<DbError>()
        .context(DbError::Connection("connecting to database".to_string()))
}

/// Build a SQLite connection pool with WAL journal mode, busy-timeout retry,
/// and NORMAL fsync policy. WAL allows concurrent readers during writes and
/// avoids SQLITE_BUSY errors under pool contention.
#[cfg(feature = "db-sqlite")]
async fn connect_sqlite(config: &DbConfig) -> Result<DatabaseConnection> {
    use sea_orm::SqlxSqliteConnector;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use std::str::FromStr;

    let connect_opts = SqliteConnectOptions::from_str(&config.url)
        .map_err(|e| Report::new(DbError::Connection(e.to_string())))?
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_millis(5000))
        .synchronous(SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(300))
        .connect_with(connect_opts)
        .await
        .map_err(|e| Report::new(DbError::Connection(e.to_string())))?;

    Ok(SqlxSqliteConnector::from_sqlx_sqlite_pool(pool))
}

/// Sanitize database URL for logging (removes credentials)
pub(crate) fn sanitize_url(url: &str) -> String {
    if let Some(at_pos) = url.find('@')
        && let Some(proto_end) = url.find("://")
    {
        #[expect(
            clippy::string_slice,
            reason = "char-boundary safe: `proto_end` and `at_pos` are byte indices of ASCII `:`/`@` returned by `str::find`, so the slices land on UTF-8 boundaries"
        )]
        let protocol = &url[..proto_end + 3];
        #[expect(
            clippy::string_slice,
            reason = "char-boundary safe: `at_pos` is the byte index of ASCII `@` returned by `str::find`"
        )]
        let rest = &url[at_pos..];
        return format!("{protocol}***{rest}");
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test code: `assert!(r.is_ok())` is idiomatic in tests where the success value is not inspected"
    )]

    use super::*;

    #[tokio::test]
    async fn test_sqlite_memory_connection() {
        let config = DbConfig {
            url: "sqlite::memory:".to_string(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
        };
        let db = connect(&config).await.unwrap();
        assert!(db.ping().await.is_ok());
    }
}
