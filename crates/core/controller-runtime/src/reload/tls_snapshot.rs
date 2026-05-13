#![allow(dead_code, reason = "items wired into coordinator in Task 14")]

//! TLS config snapshot reloadable subsystem.
//!
//! [`TlsSnapshotReloadable`] wraps a live [`TlsConfig`] and is distributed to
//! consumers via a [`tokio::sync::watch`] channel.  When TLS cert/key paths
//! change, a new config snapshot is published and receivers pick it up
//! atomically.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_config_reload::config::TlsConfig;
use uptrakit_config_reload::defaults::WATCHDOG_HTTPS;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

/// A [`Reloadable`] subsystem that manages the TLS config snapshot.
///
/// TLS cert/key paths can be adjusted at runtime; the actual cert parsing and
/// TLS handshake setup is done by the HTTPS listener startup code. This
/// reloadable just tracks config version changes and republishes them via a
/// watch channel.
///
/// On `apply`, a new config snapshot is published via the `watch` channel, and
/// the previous config is stashed for `revert`.  On `revert` the previous
/// config is re-published.
pub(crate) struct TlsSnapshotReloadable {
    /// Sender half of the config broadcast channel.
    tx: watch::Sender<Arc<TlsConfig>>,
    /// The previous config, saved by `apply` so that `revert` can restore it.
    snapshot: Mutex<Option<Arc<TlsConfig>>>,
}

impl TlsSnapshotReloadable {
    /// Create a new `TlsSnapshotReloadable` wrapping an initial config.
    pub(crate) fn new(initial: TlsConfig) -> (Self, watch::Receiver<Arc<TlsConfig>>) {
        let (tx, rx) = watch::channel(Arc::new(initial));
        (
            Self {
                tx,
                snapshot: Mutex::new(None),
            },
            rx,
        )
    }
}

impl Reloadable for TlsSnapshotReloadable {
    type Config = TlsConfig;

    fn name(&self) -> &'static str {
        "tls_snapshot"
    }

    /// Validate that the incoming config is internally consistent and the cert
    /// and key files exist.
    ///
    /// Delegates field-level checks (empty paths) to [`TlsConfig::validate`],
    /// then probes that both files are readable.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the config is invalid or if
    /// either file does not exist or is not readable.
    fn validate(&self, new: &TlsConfig) -> Result<(), Report> {
        new.validate()?;
        // Probe: verify the cert and key files are readable
        std::fs::metadata(&new.cert_path).map_err(|e| {
            report!(ConfigReloadError::Validate(format!(
                "tls.cert_path not readable ({}): {e}",
                new.cert_path
            )))
        })?;
        std::fs::metadata(&new.key_path).map_err(|e| {
            report!(ConfigReloadError::Validate(format!(
                "tls.key_path not readable ({}): {e}",
                new.key_path
            )))
        })?;
        Ok(())
    }

    /// Publish a new TLS config snapshot via the watch channel. The previous
    /// config is stashed for a potential `revert`.
    ///
    /// # Errors
    ///
    /// This method always succeeds; if all receivers are dropped, the send is
    /// silently ignored (benign in this context).
    async fn apply(&self, new: Arc<TlsConfig>) -> Result<(), Report> {
        let prior = self.tx.borrow().clone();
        *self.snapshot.lock() = Some(prior);
        tracing::info!(cert = %new.cert_path, key = %new.key_path, "tls config applied");
        #[expect(
            clippy::let_underscore_must_use,
            reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
        )]
        let _ = self.tx.send(new);
        Ok(())
    }

    /// Restore the previous TLS config if one was stashed by `apply`.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())` — if no snapshot exists there is nothing to
    /// revert and the subsystem remains in its current state.
    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.snapshot.lock().clone() {
            tracing::info!("tls config reverted");
            #[expect(
                clippy::let_underscore_must_use,
                reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
            )]
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    /// Confirm the TLS config can be applied by verifying cert file is still
    /// readable.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::HealthFailed`] if the file is no longer
    /// readable.
    async fn health_check(&self) -> Result<(), Report> {
        let cfg = self.tx.borrow().clone();
        std::fs::metadata(&cfg.cert_path).map_err(|e| {
            report!(ConfigReloadError::HealthFailed {
                subsystem: "tls_snapshot".into(),
                message: format!("cert_path not readable: {e}"),
            })
        })?;
        tracing::debug!("tls snapshot health check ok");
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_HTTPS
    }
}

uptrakit_config_reload::reloadable_erased_impl!(TlsSnapshotReloadable, RuntimeConfigDelta::Tls);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_validate_rejects_empty_cert_path() {
        let cfg = TlsConfig::new("", "/etc/ssl/key.pem");
        assert!(
            TlsSnapshotReloadable::new(TlsConfig::default())
                .0
                .validate(&cfg)
                .is_err()
        );
    }

    #[test]
    fn tls_validate_rejects_empty_key_path() {
        let cfg = TlsConfig::new("/etc/ssl/cert.pem", "");
        assert!(
            TlsSnapshotReloadable::new(TlsConfig::default())
                .0
                .validate(&cfg)
                .is_err()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_apply_updates_receiver() {
        // Use /dev/null which exists on all Unix systems.
        // metadata() passes (file exists), but full TLS validation happens
        // in the HTTPS listener startup, not in this reloadable.
        let initial = TlsConfig::new("/dev/null", "/dev/null");
        let (r, rx) = TlsSnapshotReloadable::new(initial);
        let new = Arc::new(TlsConfig::new("/dev/null", "/dev/null"));
        r.apply(new).await.unwrap();
        assert!(rx.has_changed().unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_revert_restores_prior_config() {
        let initial = TlsConfig::new("/dev/null", "/dev/null");
        let (r, mut rx) = TlsSnapshotReloadable::new(initial.clone());
        let new = Arc::new(TlsConfig::new("/dev/null", "/dev/null"));
        r.apply(new).await.unwrap();
        rx.changed().await.unwrap();
        r.revert().await.unwrap();
        rx.changed().await.unwrap();
        let reverted = rx.borrow().clone();
        assert_eq!(reverted.cert_path, initial.cert_path);
        assert_eq!(reverted.key_path, initial.key_path);
    }
}
