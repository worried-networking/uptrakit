#![allow(dead_code, reason = "items wired into coordinator in Task 14")]

//! Audit dispatcher reloadable subsystem.
//!
//! [`AuditDispatcherReloadable`] wraps a live [`AuditConfig`] and is distributed to
//! consumers via a [`tokio::sync::watch`] channel.  When the audit filter or retention
//! settings change, a new config snapshot is published and receivers pick it up atomically.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_audit_log::AuditLogDispatcher;
use uptrakit_config_reload::config::AuditConfig;
use uptrakit_config_reload::defaults::WATCHDOG_AUDIT;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

/// A [`Reloadable`] subsystem that manages the audit log configuration.
///
/// The dispatcher itself (`AuditLogDispatcher`) is cheaply cloneable (Arc-based)
/// and handles fire-and-forget audit log entry dispatch. Configuration changes
/// (filter mode, retention days) are broadcast to all consumers via a watch
/// channel. The health check verifies the dispatcher's background loop is still
/// running by checking if the mpsc sender is closed.
///
/// On `apply`, a new config snapshot is published via the `watch` channel, and
/// the previous config is stashed for `revert`. On `revert` the previous
/// config is re-published.
pub(crate) struct AuditDispatcherReloadable {
    /// The dispatcher instance shared with the audit system.
    dispatcher: AuditLogDispatcher,
    /// Sender half of the config broadcast channel.
    tx: watch::Sender<Arc<AuditConfig>>,
    /// The previous config, saved by `apply` so that `revert` can restore it.
    snapshot: Mutex<Option<Arc<AuditConfig>>>,
}

impl AuditDispatcherReloadable {
    /// Create a new `AuditDispatcherReloadable` wrapping an initial config and dispatcher.
    pub(crate) fn new(
        dispatcher: AuditLogDispatcher,
        initial: AuditConfig,
    ) -> (Self, watch::Receiver<Arc<AuditConfig>>) {
        let (tx, rx) = watch::channel(Arc::new(initial));
        (
            Self {
                dispatcher,
                tx,
                snapshot: Mutex::new(None),
            },
            rx,
        )
    }
}

impl Reloadable for AuditDispatcherReloadable {
    type Config = AuditConfig;

    fn name(&self) -> &'static str {
        "audit_dispatcher"
    }

    /// Validate that the incoming config is internally consistent.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the filter is not one of
    /// `"all"`, `"mutations"`, or `"none"`.
    fn validate(&self, new: &AuditConfig) -> Result<(), Report> {
        new.validate()?;
        Ok(())
    }

    /// Publish a new audit config snapshot via the watch channel. The previous
    /// config is stashed for a potential `revert`.
    ///
    /// # Errors
    ///
    /// This method always succeeds; if all receivers are dropped, the send is
    /// silently ignored (benign in this context).
    async fn apply(&self, new: Arc<AuditConfig>) -> Result<(), Report> {
        let prior = self.tx.borrow().clone();
        *self.snapshot.lock() = Some(prior);
        tracing::info!(
            filter = %new.filter,
            retention_days = new.retention_days,
            "audit config applied"
        );
        #[expect(
            clippy::let_underscore_must_use,
            reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
        )]
        let _ = self.tx.send(new);
        Ok(())
    }

    /// Restore the previous audit config if one was stashed by `apply`.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())` — if no snapshot exists there is nothing to
    /// revert and the subsystem remains in its current state.
    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.snapshot.lock().clone() {
            tracing::info!("audit config reverted");
            #[expect(
                clippy::let_underscore_must_use,
                reason = "watch::Sender::send returns Err only when all receivers are dropped; benign here"
            )]
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    /// Confirm the audit dispatcher is healthy after `apply`.
    ///
    /// Verifies that the dispatcher's background loop is still running by
    /// checking if the mpsc sender is closed.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::HealthFailed`] if the dispatcher channel
    /// is closed (background loop panicked or shut down).
    async fn health_check(&self) -> Result<(), Report> {
        if self.dispatcher.is_closed() {
            return Err(report!(ConfigReloadError::HealthFailed {
                subsystem: "audit_dispatcher".into(),
                message: "dispatcher channel is closed".into(),
            })
            .into());
        }
        tracing::debug!("audit dispatcher health check ok");
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_AUDIT
    }
}

uptrakit_config_reload::reloadable_erased_impl!(
    AuditDispatcherReloadable,
    RuntimeConfigDelta::Audit
);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_audit_log::NoopBackend;

    fn make_dispatcher() -> AuditLogDispatcher {
        AuditLogDispatcher::new(Arc::new(NoopBackend))
    }

    #[test]
    fn audit_validate_accepts_valid_config() {
        let cfg = AuditConfig::new("all", 90);
        cfg.validate().unwrap();
    }

    #[test]
    fn audit_validate_rejects_invalid_filter() {
        let cfg = AuditConfig::new("invalid", 90);
        assert!(cfg.validate().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn audit_apply_updates_receiver() {
        let dispatcher = make_dispatcher();
        let initial = AuditConfig::new("all", 90);
        let (r, mut rx) = AuditDispatcherReloadable::new(dispatcher, initial);
        let new = Arc::new(AuditConfig::new("mutations", 30));
        r.apply(new).await.unwrap();
        rx.changed().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn audit_health_check_passes_on_open_dispatcher() {
        let dispatcher = make_dispatcher();
        let initial = AuditConfig::new("all", 90);
        let (r, _rx) = AuditDispatcherReloadable::new(dispatcher, initial);
        r.health_check().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn audit_revert_restores_prior_config() {
        let dispatcher = make_dispatcher();
        let initial = AuditConfig::new("all", 90);
        let (r, mut rx) = AuditDispatcherReloadable::new(dispatcher, initial.clone());
        let new = Arc::new(AuditConfig::new("mutations", 30));
        r.apply(new).await.unwrap();
        rx.changed().await.unwrap();
        r.revert().await.unwrap();
        rx.changed().await.unwrap();
        let reverted = rx.borrow().clone();
        assert_eq!(reverted.filter, initial.filter);
        assert_eq!(reverted.retention_days, initial.retention_days);
    }
}
