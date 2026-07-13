//! Zeroconf config reload gate.
//!
//! [`ZeroconfReloadable`] is a validate-only gate: the zero-configuration
//! auto-discovery advertiser is started once at boot from the config it was
//! given, so this subsystem never changes any runtime state. It exists
//! solely to reject a config reload that would silently leave a changed
//! `[zeroconf]` section unapplied.

use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_config_reload::config::ZeroconfConfig;
use uptrakit_config_reload::defaults::WATCHDOG_ZEROCONF;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

/// A [`Reloadable`] validate-reject gate for `[zeroconf]`.
#[non_exhaustive]
pub(crate) struct ZeroconfReloadable {
    /// The zeroconf config this process booted with.
    boot: ZeroconfConfig,
}

impl ZeroconfReloadable {
    /// Create a new `ZeroconfReloadable` bound to the boot-time config.
    pub(crate) fn new(boot: ZeroconfConfig) -> Self {
        Self { boot }
    }
}

impl Reloadable for ZeroconfReloadable {
    type Config = ZeroconfConfig;
    fn name(&self) -> &'static str {
        "zeroconf"
    }

    /// Reject any change to `[zeroconf]` — the advertiser is started once at
    /// boot and cannot be reconfigured without a full restart.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the new config is
    /// internally inconsistent (`enabled=true` with an empty `url`), or if
    /// it differs from the boot-time config.
    fn validate(&self, new: &ZeroconfConfig) -> Result<(), Report> {
        new.validate()?;
        if *new != self.boot {
            bail!(ConfigReloadError::Validate(
                "zeroconf config change requires restart".into()
            ));
        }
        Ok(())
    }

    /// No-op — validate already rejected any change that would require
    /// action here.
    async fn apply(&self, _new: std::sync::Arc<ZeroconfConfig>) -> Result<(), Report> {
        Ok(())
    }

    /// No-op — nothing was mutated by `apply`.
    async fn revert(&self) -> Result<(), Report> {
        Ok(())
    }

    /// Validate-only gate controls no live advertiser; health is always OK
    /// if validate passed.
    async fn health_check(&self) -> Result<(), Report> {
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
    fn zeroconf_validate_rejects_change() {
        let boot = ZeroconfConfig::default();
        let r = ZeroconfReloadable::new(boot);
        let mut new = ZeroconfConfig::default();
        new.enabled = true;
        new.url = "https://controller.example".into();

        let err = r.validate(&new).unwrap_err();
        assert!(
            err.to_string()
                .contains("zeroconf config change requires restart")
        );
    }

    #[test]
    fn zeroconf_validate_accepts_unchanged() {
        let boot = ZeroconfConfig::default();
        let r = ZeroconfReloadable::new(boot.clone());

        r.validate(&boot).unwrap();
    }

    #[test]
    fn zeroconf_validate_rejects_enabled_without_url() {
        let boot = ZeroconfConfig::default();
        let r = ZeroconfReloadable::new(boot);
        let mut new = ZeroconfConfig::default();
        new.enabled = true;

        r.validate(&new).unwrap_err();
    }
}
