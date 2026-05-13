use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_config_reload::config::ZeroconfConfig;
use uptrakit_config_reload::defaults::WATCHDOG_ZEROCONF;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::reloadable::Reloadable;

pub(crate) struct ZeroconfReloadable {
    tx: watch::Sender<Arc<ZeroconfConfig>>,
    snapshot: Mutex<Option<Arc<ZeroconfConfig>>>,
}

impl ZeroconfReloadable {
    pub(crate) fn new(initial: ZeroconfConfig) -> (Self, watch::Receiver<Arc<ZeroconfConfig>>) {
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

impl Reloadable for ZeroconfReloadable {
    type Config = ZeroconfConfig;
    fn name(&self) -> &'static str {
        "zeroconf"
    }

    fn validate(&self, new: &ZeroconfConfig) -> Result<(), Report> {
        new.validate()?;
        Ok(())
    }

    async fn apply(&self, new: Arc<ZeroconfConfig>) -> Result<(), Report> {
        let prior = self.tx.borrow().clone();
        *self.snapshot.lock() = Some(prior);
        tracing::info!(enabled = new.enabled, url = %new.url, "zeroconf config applied");
        #[expect(
            clippy::let_underscore_must_use,
            reason = "watch send failure is non-fatal"
        )]
        let _ = self.tx.send(new);
        Ok(())
    }

    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.snapshot.lock().clone() {
            tracing::info!("zeroconf config reverted");
            #[expect(
                clippy::let_underscore_must_use,
                reason = "watch send failure is non-fatal"
            )]
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Report> {
        // Zeroconf health: config is valid (no live mDNS probe needed here;
        // the mDNS daemon handles its own liveness)
        let cfg = self.tx.borrow().clone();
        cfg.validate()?;
        tracing::debug!("zeroconf health check ok");
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_ZEROCONF
    }
}

uptrakit_config_reload::reloadable_erased_impl!(ZeroconfReloadable, RuntimeConfigDelta::Zeroconf);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroconf_validate_accepts_disabled() {
        let cfg = ZeroconfConfig::default(); // enabled = false
        ZeroconfReloadable::new(cfg.clone())
            .0
            .validate(&cfg)
            .unwrap();
    }

    #[test]
    fn zeroconf_validate_rejects_enabled_without_url() {
        let mut cfg = ZeroconfConfig::default();
        cfg.enabled = true;
        let (r, _) = ZeroconfReloadable::new(ZeroconfConfig::default());
        assert!(r.validate(&cfg).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn zeroconf_apply_updates_receiver() {
        let (r, mut rx) = ZeroconfReloadable::new(ZeroconfConfig::default());
        let new = Arc::new(ZeroconfConfig::default());
        r.apply(new).await.unwrap();
        rx.changed().await.unwrap();
    }
}
