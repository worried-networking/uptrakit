//! DB connection pool reloadable subsystem.
//!
//! [`DbConnHandle`] wraps a live [`DatabaseConnection`] and is distributed to
//! consumers via a [`tokio::sync::watch`] channel so that in-flight requests
//! finish against the old pool while new requests pick up the replacement pool
//! atomically.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use tokio::sync::watch;
use uptrakit_config_reload::config::DbConfig;
use uptrakit_config_reload::defaults::WATCHDOG_DB_POOL;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

/// A thin wrapper around a [`DatabaseConnection`] that is distributed to
/// consumers via a `watch` channel.
///
/// Marked `#[non_exhaustive]` so that additional diagnostics fields can be
/// added without a semver break.
#[non_exhaustive]
pub(crate) struct DbConnHandle {
    inner: DatabaseConnection,
}

impl DbConnHandle {
    /// Wrap a raw connection.
    pub(crate) fn new(inner: DatabaseConnection) -> Self {
        Self { inner }
    }

    /// Borrow the underlying connection.
    pub(crate) fn conn(&self) -> &DatabaseConnection {
        &self.inner
    }
}

/// A [`Reloadable`] subsystem that manages the database connection pool.
///
/// Pool-size and acquire-timeout can be adjusted at runtime.  The database
/// URL cannot change without a full process restart (reexec).
///
/// On `apply`, a new pool is opened, published via the `watch` channel, and
/// the previous handle is stashed for `revert`.  On `revert` the previous
/// handle is re-published.
pub(crate) struct DbPoolReloadable {
    /// The URL the current pool was opened against.  Used to reject URL changes.
    current_url: String,
    /// Sender half of the handle broadcast channel.
    tx: watch::Sender<Arc<DbConnHandle>>,
    /// The previous handle, saved by `apply` so that `revert` can restore it.
    snapshot: Mutex<Option<Arc<DbConnHandle>>>,
}

impl DbPoolReloadable {
    /// Create a new `DbPoolReloadable` wrapping an already-open pool.
    pub(crate) fn new(initial: DatabaseConnection, url: String) -> Self {
        let handle = Arc::new(DbConnHandle::new(initial));
        let (tx, _rx) = watch::channel(handle);
        Self {
            current_url: url,
            tx,
            snapshot: Mutex::new(None),
        }
    }

    /// Subscribe to handle updates.  Each receiver always holds the latest
    /// live pool; consumers should `borrow()` it before issuing queries.
    pub(crate) fn receiver(&self) -> watch::Receiver<Arc<DbConnHandle>> {
        self.tx.subscribe()
    }
}

impl Reloadable for DbPoolReloadable {
    type Config = DbConfig;

    fn name(&self) -> &'static str {
        "db_pool"
    }

    /// Validate that the incoming config does not attempt to change the DB URL,
    /// and that `pool_size` is at least 1.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the URL differs from the one
    /// this pool was opened against, or if `pool_size` is zero.
    fn validate(&self, new: &DbConfig) -> Result<(), Report> {
        if new.url != self.current_url {
            bail!(ConfigReloadError::Validate(format!(
                "db.url change requires reexec (current = {}, new = {})",
                self.current_url, new.url
            )));
        }
        if new.pool_size == 0 {
            bail!(ConfigReloadError::Validate(
                "db.pool_size must be >= 1".into()
            ));
        }
        Ok(())
    }

    /// Open a fresh pool with the new settings and publish it via the watch
    /// channel.  The previous handle is stashed for a potential `revert`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::ApplyFailed`] if the new pool cannot be
    /// opened (e.g. the database file disappeared between validate and apply).
    async fn apply(&self, new: Arc<DbConfig>) -> Result<(), Report> {
        let mut opt = ConnectOptions::new(new.url.clone());
        opt.max_connections(new.pool_size);
        opt.acquire_timeout(Duration::from_millis(new.acquire_timeout_ms));

        let pool = Database::connect(opt).await.map_err(|e| {
            report!(ConfigReloadError::ApplyFailed {
                subsystem: "db_pool".into(),
                message: e.to_string(),
            })
        })?;

        let new_handle = Arc::new(DbConnHandle::new(pool));

        // Stash the current handle before replacing it, so revert can restore it.
        let current = self.tx.borrow().clone();
        {
            let mut guard = self.snapshot.lock();
            *guard = Some(current);
        } // guard dropped before the send/.await boundary

        #[expect(
            clippy::let_underscore_must_use,
            reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
        )]
        let _ = self.tx.send(new_handle);
        Ok(())
    }

    /// Restore the previous pool handle if one was stashed by `apply`.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())` — if no snapshot exists there is nothing to
    /// revert and the subsystem remains in its current state.
    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.snapshot.lock().clone() {
            #[expect(
                clippy::let_underscore_must_use,
                reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
            )]
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    /// Confirm the current pool can reach the database with a trivial query.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::HealthFailed`] if the query fails.
    async fn health_check(&self) -> Result<(), Report> {
        let handle = self.tx.borrow().clone();
        handle
            .conn()
            .execute_unprepared("SELECT 1")
            .await
            .map_err(|e| {
                report!(ConfigReloadError::HealthFailed {
                    subsystem: "db_pool".into(),
                    message: e.to_string(),
                })
            })?;
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_DB_POOL
    }
}

uptrakit_config_reload::reloadable_erased_impl!(DbPoolReloadable, RuntimeConfigDelta::Db);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_URL: &str = "sqlite::memory:";

    #[tokio::test(flavor = "current_thread")]
    async fn db_reloadable_validates_url_unchanged() {
        let pool = build_test_pool().await;
        let reloadable = DbPoolReloadable::new(pool.clone(), TEST_URL.to_string());
        let mut new = DbConfig::default();
        new.url = TEST_URL.to_string();
        new.pool_size = 32;
        new.acquire_timeout_ms = 6_000;
        reloadable.validate(&new).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn db_reloadable_rejects_url_change() {
        let pool = build_test_pool().await;
        let reloadable = DbPoolReloadable::new(pool.clone(), TEST_URL.to_string());
        let mut new = DbConfig::default();
        new.url = "sqlite::memory:foo".to_string();
        let err = reloadable.validate(&new).unwrap_err();
        assert!(err.to_string().contains("db.url"));
    }

    async fn build_test_pool() -> DatabaseConnection {
        sea_orm::Database::connect(TEST_URL)
            .await
            .expect("test pool")
    }
}
