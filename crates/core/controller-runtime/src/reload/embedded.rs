use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_config_reload::config::EmbeddedServicesConfig;
use uptrakit_config_reload::defaults::WATCHDOG_EMBEDDED;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

pub(crate) struct EmbeddedServicesReloadable {
    /// The topology at boot time; changes to these booleans force reexec.
    boot_topology: EmbeddedServicesConfig,
    tx: watch::Sender<Arc<EmbeddedServicesConfig>>,
    snapshot: Mutex<Option<Arc<EmbeddedServicesConfig>>>,
}

impl EmbeddedServicesReloadable {
    pub(crate) fn new(
        initial: EmbeddedServicesConfig,
    ) -> (Self, watch::Receiver<Arc<EmbeddedServicesConfig>>) {
        let boot_topology = initial.clone();
        let (tx, rx) = watch::channel(Arc::new(initial));
        (
            Self {
                boot_topology,
                tx,
                snapshot: Mutex::new(None),
            },
            rx,
        )
    }
}

impl Reloadable for EmbeddedServicesReloadable {
    type Config = EmbeddedServicesConfig;
    fn name(&self) -> &'static str {
        "embedded_services"
    }

    fn validate(&self, new: &EmbeddedServicesConfig) -> Result<(), Report> {
        // Topology booleans must not change without reexec
        if new.agent != self.boot_topology.agent
            || new.agent_ssh != self.boot_topology.agent_ssh
            || new.mqtt != self.boot_topology.mqtt
            || new.scheduler != self.boot_topology.scheduler
        {
            return Err(report!(ConfigReloadError::Validate(
                "embedded_services topology change requires reexec (Plan 3)".into()
            ))
            .into());
        }
        Ok(())
    }

    async fn apply(&self, new: Arc<EmbeddedServicesConfig>) -> Result<(), Report> {
        let prior = self.tx.borrow().clone();
        *self.snapshot.lock() = Some(prior);
        tracing::info!("embedded services config applied (topology unchanged)");
        #[expect(
            clippy::let_underscore_must_use,
            reason = "watch send failure is non-fatal"
        )]
        let _ = self.tx.send(new);
        Ok(())
    }

    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.snapshot.lock().clone() {
            tracing::info!("embedded services config reverted");
            #[expect(
                clippy::let_underscore_must_use,
                reason = "watch send failure is non-fatal"
            )]
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Report> {
        // Topology is static after boot; health is always OK if validate passed
        tracing::debug!("embedded services health check ok");
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_EMBEDDED
    }
}

uptrakit_config_reload::reloadable_erased_impl!(
    EmbeddedServicesReloadable,
    RuntimeConfigDelta::EmbeddedServices
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_validate_accepts_same_topology() {
        let cfg = EmbeddedServicesConfig::default();
        let (r, _) = EmbeddedServicesReloadable::new(cfg.clone());
        r.validate(&cfg).unwrap();
    }

    #[test]
    fn embedded_validate_rejects_topology_change() {
        let initial = EmbeddedServicesConfig::default(); // agent = false
        let (r, _) = EmbeddedServicesReloadable::new(initial);
        let mut changed = EmbeddedServicesConfig::default();
        changed.agent = true; // topology change
        r.validate(&changed).unwrap_err();
        assert!(
            r.validate(&changed)
                .unwrap_err()
                .to_string()
                .contains("reexec")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn embedded_apply_updates_receiver() {
        let cfg = EmbeddedServicesConfig::default();
        let (r, mut rx) = EmbeddedServicesReloadable::new(cfg.clone());
        let new = Arc::new(cfg);
        r.apply(new).await.unwrap();
        rx.changed().await.unwrap();
    }
}
