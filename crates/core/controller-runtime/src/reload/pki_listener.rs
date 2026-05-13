//! PKI listener reloadable subsystem.
//!
//! [`PkiListenerReloadable`] distributes updated [`PkiConfig`] values to the
//! running PKI (certificate authority) listener via a [`tokio::sync::watch`]
//! channel so that consumers can react to address changes without a full
//! process restart.
//!
//! On `apply`, the new config is broadcast and the prior snapshot is stashed
//! for a potential `revert`.  A TCP probe to the bound address confirms
//! liveness after apply.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_config_reload::config::{NetworkConfig, PkiConfig};
use uptrakit_config_reload::defaults::WATCHDOG_PKI;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

use crate::reload::probe::pick_probe_addr;

/// A [`Reloadable`] subsystem that distributes updated [`PkiConfig`] values.
///
/// The watch channel carries `Arc<PkiConfig>` so that subscribers (e.g. the
/// PKI listener task) can cheaply clone the current value without copying the
/// full config.
///
/// Marked `#[non_exhaustive]` so that additional diagnostic fields can be
/// added without a semver break.
#[non_exhaustive]
pub(crate) struct PkiListenerReloadable {
    /// Sender half of the config broadcast channel.
    tx: watch::Sender<Arc<PkiConfig>>,
    /// The previous config, stashed by `apply` so that `revert` can restore it.
    snapshot: Mutex<Option<Arc<PkiConfig>>>,
    /// Set to `true` while a drain is in progress to suppress the pre-bind
    /// address probe during validation.
    draining: Mutex<bool>,
}

impl PkiListenerReloadable {
    /// Create a new `PkiListenerReloadable` with the given initial config.
    ///
    /// Returns the reloadable together with a receiver that always holds the
    /// latest live config.
    pub(crate) fn new(initial: PkiConfig) -> (Self, watch::Receiver<Arc<PkiConfig>>) {
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
    /// Receives the full `NetworkConfig`; only the `pki` field is used.
    type Config = NetworkConfig;

    fn name(&self) -> &'static str {
        "pki_listener"
    }

    /// Validate that the incoming config can be applied.
    ///
    /// If the address is unchanged, validation is a no-op.  Otherwise, a
    /// pre-bind probe is attempted to detect port conflicts before any state
    /// mutation occurs.  The probe is suppressed while a drain is in progress.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the new address cannot be
    /// bound.
    fn validate(&self, new: &NetworkConfig) -> Result<(), Report> {
        let current = self.tx.borrow().clone();
        if new.pki.addr == current.addr {
            return Ok(());
        }
        // Suppress the pre-bind probe during a drain: the listener may
        // still hold the port while connections are flushed.
        let is_draining = *self.draining.lock();
        if is_draining {
            return Ok(());
        }
        // Attempt to bind the new address as a lightweight port-conflict probe.
        // The listener is dropped immediately — we only care whether the bind
        // succeeds.
        let probe = std::net::TcpListener::bind(&new.pki.addr).map_err(|e| {
            report!(ConfigReloadError::Validate(format!(
                "network.pki.addr bind probe failed: {e}"
            )))
        })?;
        drop(probe);
        Ok(())
    }

    /// Broadcast the new PKI config to all subscribers.
    ///
    /// The current config is stashed so that `revert` can restore it.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())` — the watch send only fails when all receivers
    /// are dropped, which is benign.
    async fn apply(&self, new: Arc<NetworkConfig>) -> Result<(), Report> {
        // Stash the current config before replacing it.
        let current = self.tx.borrow().clone();
        {
            let mut guard = self.snapshot.lock();
            *guard = Some(current);
        } // guard dropped before the send/.await boundary

        tracing::info!(addr = %new.pki.addr, "pki listener config applied");

        #[expect(
            clippy::let_underscore_must_use,
            reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
        )]
        let _ = self.tx.send(Arc::new(new.pki.clone()));
        Ok(())
    }

    /// Restore the previously stashed config.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())` — if no snapshot exists there is nothing to
    /// revert and the subsystem remains in its current state.
    async fn revert(&self) -> Result<(), Report> {
        let prior = self.snapshot.lock().clone();
        // Guard dropped before .await implicitly (this block is sync)
        if let Some(prior) = prior {
            tracing::info!(addr = %prior.addr, "pki listener config reverted");

            #[expect(
                clippy::let_underscore_must_use,
                reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
            )]
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    /// Confirm the PKI listener is accepting connections by probing the bound address.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::HealthFailed`] if the probe times out or
    /// the connection is refused.
    async fn health_check(&self) -> Result<(), Report> {
        let cfg = self.tx.borrow().clone();
        let probe_addr = pick_probe_addr(&cfg.addr)?;
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

    fn make_pki_cfg(addr: &str) -> PkiConfig {
        let mut cfg = PkiConfig::default();
        cfg.addr = addr.into();
        cfg
    }

    #[tokio::test]
    async fn pki_reloadable_skip_pre_bind_on_same_addr() {
        let cfg = make_pki_cfg("127.0.0.1:0");
        let (r, _rx) = PkiListenerReloadable::new(cfg.clone());
        let mut net = NetworkConfig::default();
        net.pki = cfg;
        // same addr → no probe attempt
        r.validate(&net).unwrap();
    }

    #[tokio::test]
    async fn pki_reloadable_apply_updates_receiver() {
        let cfg = make_pki_cfg("127.0.0.1:0");
        let (r, rx) = PkiListenerReloadable::new(cfg);
        let mut net = NetworkConfig::default();
        net.pki = make_pki_cfg("127.0.0.1:9");
        r.apply(Arc::new(net)).await.unwrap();
        assert!(rx.has_changed().unwrap());
    }

    #[tokio::test]
    async fn pki_reloadable_revert_restores_prior() {
        let initial = make_pki_cfg("127.0.0.1:0");
        let (r, mut rx) = PkiListenerReloadable::new(initial.clone());

        // Apply a change.
        let mut net = NetworkConfig::default();
        net.pki = make_pki_cfg("127.0.0.1:9");
        r.apply(Arc::new(net)).await.unwrap();
        #[allow(clippy::let_underscore_must_use)]
        let _ = rx.changed().await.ok(); // consume the apply event

        // Revert should broadcast the original address.
        r.revert().await.unwrap();
        assert!(rx.has_changed().unwrap());
        let restored = rx.borrow_and_update().clone();
        assert_eq!(restored.addr, initial.addr);
    }

    #[tokio::test]
    async fn pki_reloadable_skip_pre_bind_while_draining() {
        let cfg = make_pki_cfg("127.0.0.1:0");
        let (r, _rx) = PkiListenerReloadable::new(cfg);
        // Signal drain.
        *r.draining.lock() = true;

        // Even though the address differs, validation should succeed because
        // the draining flag suppresses the bind probe.
        let mut net = NetworkConfig::default();
        net.pki = make_pki_cfg("127.0.0.1:9999");
        r.validate(&net).unwrap();
    }
}
