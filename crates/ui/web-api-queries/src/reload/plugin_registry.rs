use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_config_reload::config::PluginsConfig;
use uptrakit_config_reload::defaults::WATCHDOG_PLUGINS;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::reloadable::Reloadable;

/// Reloadable wrapper for the plugin catalog subsystem.
///
/// Plugin configurations are DB-driven (not TOML-driven). The coordinator
/// triggers a reload when `ConfigReconciler` detects a `settings_version`
/// bump for plugin-related settings.
///
/// `health_check` always returns `Ok(())` per spec §10.6: single-shot
/// constructor-success validation is the only signal; there is no
/// `Plugin::health()` method.
pub struct PluginCatalogReloadable {
    tx: watch::Sender<Arc<PluginsConfig>>,
    snapshot: Mutex<Option<Arc<PluginsConfig>>>,
}

impl PluginCatalogReloadable {
    /// Create a new `PluginCatalogReloadable` with the given initial config.
    ///
    /// Returns the reloadable and a receiver that yields the current
    /// `PluginsConfig` on each reload.
    pub fn new(initial: PluginsConfig) -> (Self, watch::Receiver<Arc<PluginsConfig>>) {
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

impl Reloadable for PluginCatalogReloadable {
    type Config = PluginsConfig;

    fn name(&self) -> &'static str {
        "plugin_catalog"
    }

    fn validate(&self, _new: &PluginsConfig) -> Result<(), Report> {
        Ok(())
    }

    async fn apply(&self, new: Arc<PluginsConfig>) -> Result<(), Report> {
        let prior = self.tx.borrow().clone();
        *self.snapshot.lock() = Some(prior);
        tracing::info!(version = new.version, "plugin catalog reload triggered");
        #[expect(
            clippy::let_underscore_must_use,
            reason = "broadcast: no active receivers is expected during startup"
        )]
        let _ = self.tx.send(new);
        Ok(())
    }

    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.snapshot.lock().clone() {
            tracing::info!("plugin catalog reload reverted");
            #[expect(
                clippy::let_underscore_must_use,
                reason = "broadcast: no active receivers is expected during startup"
            )]
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Report> {
        // Per spec §10.6: no Plugin::health() method; constructor success is
        // the signal.
        tracing::debug!("plugin catalog health check ok (no-op)");
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_PLUGINS
    }
}

uptrakit_config_reload::reloadable_erased_impl!(
    PluginCatalogReloadable,
    RuntimeConfigDelta::Plugins
);

#[cfg(test)]
mod tests {
    use super::*;

    fn plugins_config(version: u64) -> PluginsConfig {
        let mut cfg = PluginsConfig::default();
        cfg.version = version;
        cfg
    }

    #[tokio::test(flavor = "current_thread")]
    async fn plugin_reloadable_apply_increments_version() {
        let (r, mut rx) = PluginCatalogReloadable::new(plugins_config(0));
        r.apply(Arc::new(plugins_config(1))).await.unwrap();
        rx.changed().await.unwrap();
        assert_eq!(rx.borrow().version, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn plugin_health_check_always_ok() {
        let (r, _) = PluginCatalogReloadable::new(PluginsConfig::default());
        r.health_check().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn plugin_reloadable_revert_restores_prior_version() {
        let (r, mut rx) = PluginCatalogReloadable::new(plugins_config(0));

        // Apply version 1
        r.apply(Arc::new(plugins_config(1))).await.unwrap();
        rx.changed().await.unwrap();
        assert_eq!(rx.borrow().version, 1);

        // Revert should restore version 0
        r.revert().await.unwrap();
        rx.changed().await.unwrap();
        assert_eq!(rx.borrow().version, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn plugin_reloadable_revert_noop_when_no_snapshot() {
        let (r, rx) = PluginCatalogReloadable::new(plugins_config(5));
        // Revert before any apply: should be a no-op
        r.revert().await.unwrap();
        // No change emitted
        assert!(rx.has_changed().is_ok_and(|c| !c));
        assert_eq!(rx.borrow().version, 5);
    }
}
