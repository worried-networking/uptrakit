//! PKI listener reloadable subsystem.
//!
//! [`PkiListenerReloadable`] distributes updated PKI address strings to the
//! running PKI (certificate authority) listener via a [`tokio::sync::watch`]
//! channel so that consumers can react to address changes without a full
//! process restart.
//!
//! The channel carries `Arc<String>` — the raw `pki_addr` value from
//! `NetworkConfig`.  An empty string means the PKI listener is not configured.
//! An `http://` URL is an advertisement address managed outside this subsystem.
//! A bare `host:port` is both the bind address and the advertised address.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_config_reload::config::NetworkConfig;
use uptrakit_config_reload::defaults::WATCHDOG_PKI;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

use crate::reload::probe::pick_probe_addr;

/// A [`Reloadable`] subsystem that distributes updated PKI address strings.
///
/// Marked `#[non_exhaustive]` so that additional diagnostic fields can be
/// added without a semver break.
#[non_exhaustive]
pub(crate) struct PkiListenerReloadable {
    /// Sender half of the config broadcast channel.
    tx: watch::Sender<Arc<String>>,
    /// The previous address, stashed by `apply` so that `revert` can restore it.
    snapshot: Mutex<Option<Arc<String>>>,
    /// Set to `true` while a drain is in progress to suppress the pre-bind
    /// address probe during validation.
    draining: Mutex<bool>,
}

impl PkiListenerReloadable {
    /// Create a new `PkiListenerReloadable` with the given initial PKI address.
    ///
    /// Returns the reloadable together with a receiver that always holds the
    /// latest live address.
    pub(crate) fn new(initial: String) -> (Self, watch::Receiver<Arc<String>>) {
        let (tx, rx) = watch::channel(Arc::new(initial));
        let this = Self {
            tx,
            snapshot: Mutex::new(None),
            draining: Mutex::new(false),
        };
        (this, rx)
    }
}

impl Reloadable for PkiListenerReloadable {
    /// Receives the full `NetworkConfig`; only `pki_addr` is used.
    type Config = NetworkConfig;

    fn name(&self) -> &'static str {
        "pki_listener"
    }

    /// Validate that the incoming config can be applied.
    ///
    /// If the address is unchanged, or is an `http://` URL (external bind),
    /// or a drain is in progress, validation is a no-op.  Otherwise a pre-bind
    /// probe is attempted to detect port conflicts.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the new address cannot be bound.
    fn validate(&self, new: &NetworkConfig) -> Result<(), Report> {
        let current = self.tx.borrow().clone();
        if new.pki_addr == current.as_str() {
            return Ok(());
        }
        if *self.draining.lock() {
            return Ok(());
        }
        // http:// form is an advertisement URL, not a bind address — skip the probe.
        if new.pki_addr.starts_with("http://") {
            return Ok(());
        }
        if new.pki_addr.is_empty() {
            return Ok(());
        }
        let probe = std::net::TcpListener::bind(&new.pki_addr).map_err(|e| {
            report!(ConfigReloadError::Validate(format!(
                "network.pki_addr bind probe failed: {e}"
            )))
        })?;
        drop(probe);
        Ok(())
    }

    /// Broadcast the new PKI address to all subscribers.
    ///
    /// The current address is stashed so that `revert` can restore it.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())`.
    #[expect(
        clippy::let_underscore_must_use,
        reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
    )]
    async fn apply(&self, new: Arc<NetworkConfig>) -> Result<(), Report> {
        let current = self.tx.borrow().clone();
        {
            let mut guard = self.snapshot.lock();
            *guard = Some(current);
        } // guard dropped before the send/.await boundary

        tracing::info!(addr = %new.pki_addr, "pki listener config applied");
        let _ = self.tx.send(Arc::new(new.pki_addr.clone()));
        Ok(())
    }

    /// Restore the previously stashed address.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())`.
    #[expect(
        clippy::let_underscore_must_use,
        reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
    )]
    async fn revert(&self) -> Result<(), Report> {
        let prior = self.snapshot.lock().clone();
        if let Some(prior) = prior {
            tracing::info!(addr = %prior, "pki listener config reverted");
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    /// Confirm the PKI listener is accepting connections by probing the bound address.
    ///
    /// Skips the probe if the address is empty or an `http://` URL.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::HealthFailed`] if the probe times out or
    /// the connection is refused.
    async fn health_check(&self) -> Result<(), Report> {
        let cfg = self.tx.borrow().clone();
        // Skip probe for empty or http:// advertisement URLs — not a bind address.
        if cfg.is_empty() || cfg.starts_with("http://") {
            return Ok(());
        }
        let probe_addr = pick_probe_addr(&cfg)?;
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::net::TcpStream::connect(&probe_addr),
        )
        .await
        .map_err(|_elapsed| {
            report!(ConfigReloadError::HealthFailed {
                subsystem: "pki_listener".into(),
                message: format!("connect to {probe_addr} timed out after 1s"),
            })
        })?
        .map_err(|e| {
            report!(ConfigReloadError::HealthFailed {
                subsystem: "pki_listener".into(),
                message: e.to_string(),
            })
        })?;
        tracing::debug!(addr = %probe_addr, "pki listener health check ok");
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_PKI
    }
}

uptrakit_config_reload::reloadable_erased_impl!(PkiListenerReloadable, RuntimeConfigDelta::Network);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pki_reloadable_skip_pre_bind_on_same_addr() {
        let addr = "127.0.0.1:0".to_string();
        let (r, _rx) = PkiListenerReloadable::new(addr.clone());
        let mut net = NetworkConfig::default();
        net.pki_addr = addr;
        r.validate(&net).unwrap();
    }

    #[tokio::test]
    async fn pki_reloadable_apply_updates_receiver() {
        let (r, rx) = PkiListenerReloadable::new("127.0.0.1:0".to_string());
        let mut net = NetworkConfig::default();
        net.pki_addr = "127.0.0.1:9".to_string();
        r.apply(Arc::new(net)).await.unwrap();
        assert!(rx.has_changed().unwrap());
    }

    #[tokio::test]
    async fn pki_reloadable_revert_restores_prior() {
        let initial = "127.0.0.1:0".to_string();
        let (r, mut rx) = PkiListenerReloadable::new(initial.clone());

        let mut net = NetworkConfig::default();
        net.pki_addr = "127.0.0.1:9".to_string();
        r.apply(Arc::new(net)).await.unwrap();
        rx.changed().await.unwrap();

        r.revert().await.unwrap();
        assert!(rx.has_changed().unwrap());
        let restored = rx.borrow_and_update().clone();
        assert_eq!(restored.as_str(), initial.as_str());
    }

    #[tokio::test]
    async fn pki_reloadable_skip_pre_bind_while_draining() {
        let (r, _rx) = PkiListenerReloadable::new("127.0.0.1:0".to_string());
        *r.draining.lock() = true;
        let mut net = NetworkConfig::default();
        net.pki_addr = "127.0.0.1:9999".to_string();
        r.validate(&net).unwrap();
    }
}
