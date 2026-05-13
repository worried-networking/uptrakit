//! NATS client reloadable subsystem.
//!
//! [`NatsReloadable`] wraps an [`async_nats::Client`] and distributes updated
//! client handles to consumers via a [`tokio::sync::watch`] channel so that
//! subscribers can atomically pick up a new connection without a full process
//! restart.
//!
//! On `apply`, a fresh client is connected, published via the watch channel,
//! and the prior client is stashed for a potential `revert`.  A `flush` probe
//! with a 5-second timeout confirms liveness after apply.

#![cfg(feature = "nats")]

use std::sync::Arc;
use std::time::Duration;

use async_nats::Client;
use parking_lot::Mutex;
use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_config_reload::config::NatsConfig;
use uptrakit_config_reload::defaults::WATCHDOG_NATS;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

/// A [`Reloadable`] subsystem that manages the NATS client connection.
///
/// The watch channel carries `Arc<Client>` so that subscribers can cheaply
/// clone the current client without copying internal state.
///
/// On `apply`, a new client is connected to the new URL, published via the
/// watch channel, and the prior client is stashed for a potential `revert`.
/// On `revert` the prior client is re-published.
pub(crate) struct NatsReloadable {
    /// Sender half of the client broadcast channel.
    tx: watch::Sender<Arc<Client>>,
    /// The previous client, stashed by `apply` so that `revert` can restore it.
    snapshot: Mutex<Option<Arc<Client>>>,
    /// URL the snapshot client was connected to, saved for restoration.
    snapshot_url: Mutex<Option<String>>,
    /// URL the current client is connected to.
    current_url: Mutex<String>,
}

impl NatsReloadable {
    /// Create a new `NatsReloadable` wrapping an already-connected client.
    pub(crate) fn new(initial_client: Client, url: String) -> Self {
        let (tx, _rx) = watch::channel(Arc::new(initial_client));
        Self {
            tx,
            snapshot: Mutex::new(None),
            snapshot_url: Mutex::new(None),
            current_url: Mutex::new(url),
        }
    }

    /// Subscribe to client updates.  Each receiver always holds the latest
    /// live client; consumers should `borrow()` it before issuing requests.
    pub(crate) fn receiver(&self) -> watch::Receiver<Arc<Client>> {
        self.tx.subscribe()
    }
}

impl Reloadable for NatsReloadable {
    type Config = NatsConfig;

    fn name(&self) -> &'static str {
        "nats"
    }

    /// Validate the incoming config.
    ///
    /// Delegates to [`NatsConfig::validate`] which rejects an empty URL.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the config is invalid.
    fn validate(&self, new: &NatsConfig) -> Result<(), Report> {
        new.validate()?;
        Ok(())
    }

    /// Connect a fresh NATS client and publish it via the watch channel.
    ///
    /// The current client is stashed so that `revert` can restore it.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::ApplyFailed`] if the new client cannot
    /// connect (e.g. NATS server unreachable).
    async fn apply(&self, new: Arc<NatsConfig>) -> Result<(), Report> {
        // Read current URL before any await so no lock is held across .await.
        let old_url = self.current_url.lock().clone();

        let client = async_nats::connect(&new.url).await.map_err(|e| {
            report!(ConfigReloadError::ApplyFailed {
                subsystem: "nats".into(),
                message: e.to_string(),
            })
        })?;
        let new_arc = Arc::new(client);

        // Stash the current client and URL before replacing them.
        // Each lock is acquired, used, and released before the next — no
        // two locks are ever held simultaneously.
        let prior = self.tx.borrow().clone();
        {
            let mut guard = self.snapshot.lock();
            *guard = Some(prior);
        } // guard dropped
        {
            let mut guard = self.snapshot_url.lock();
            *guard = Some(old_url);
        } // guard dropped
        {
            let mut guard = self.current_url.lock();
            *guard = new.url.clone();
        } // guard dropped

        tracing::info!(url = %new.url, "nats client reloaded");

        #[expect(
            clippy::let_underscore_must_use,
            reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
        )]
        let _ = self.tx.send(new_arc);
        Ok(())
    }

    /// Restore the previously stashed client if one was saved by `apply`.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())` — if no snapshot exists there is nothing to
    /// revert and the subsystem remains in its current state.
    async fn revert(&self) -> Result<(), Report> {
        let prior = self.snapshot.lock().clone();
        let prior_url = self.snapshot_url.lock().clone();
        // Both guards are dropped before the send (no .await while holding them).
        if let Some(prior) = prior {
            if let Some(url) = prior_url {
                let mut guard = self.current_url.lock();
                *guard = url;
            } // guard dropped
            tracing::info!("nats client reverted to prior");

            #[expect(
                clippy::let_underscore_must_use,
                reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
            )]
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    /// Confirm the current NATS client is live by flushing pending output.
    ///
    /// A 5-second timeout is applied; exceeding it is treated as a failure.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::HealthFailed`] if the flush times out or
    /// returns an error.
    async fn health_check(&self) -> Result<(), Report> {
        let client = self.tx.borrow().clone();
        tokio::time::timeout(Duration::from_secs(5), client.flush())
            .await
            .map_err(|_elapsed| {
                report!(ConfigReloadError::HealthFailed {
                    subsystem: "nats".into(),
                    message: "flush timed out after 5s".into(),
                })
            })?
            .map_err(|e| {
                report!(ConfigReloadError::HealthFailed {
                    subsystem: "nats".into(),
                    message: e.to_string(),
                })
            })?;
        tracing::debug!("nats health check ok");
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_NATS
    }
}

uptrakit_config_reload::reloadable_erased_impl!(NatsReloadable, RuntimeConfigDelta::Nats);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // These tests validate NatsConfig directly without requiring a live NATS
    // server.  Integration tests for apply/health_check require Docker
    // (testcontainers) and live under tests/ with #[ignore].

    #[test]
    fn nats_validate_rejects_empty_url() {
        let cfg = NatsConfig::default(); // url = ""
        cfg.validate().unwrap_err();
    }

    #[test]
    fn nats_validate_accepts_valid_url() {
        let cfg = NatsConfig::new("nats://localhost:4222");
        cfg.validate().unwrap();
    }
}
