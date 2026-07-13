//! HTTPS listener address reload gate.
//!
//! [`HttpsListenerReloadable`] is a validate-only gate: the HTTPS listener's
//! bound address is fixed at boot (the listener socket is created once and
//! handed to the reexec child unconditionally — see
//! `crate::reexec::listenfd`), so this subsystem never changes any runtime
//! state. It exists solely to reject a config reload that would silently
//! leave a changed `network.https.addr` unapplied.

use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_config_reload::config::{HttpsConfig, NetworkConfig};
use uptrakit_config_reload::defaults::WATCHDOG_HTTPS;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

/// A [`Reloadable`] validate-reject gate for `network.https.addr`.
///
/// Marked `#[non_exhaustive]` so that additional diagnostic fields can be
/// added without a semver break.
#[non_exhaustive]
pub(crate) struct HttpsListenerReloadable {
    /// The HTTPS config this process booted with.
    boot: HttpsConfig,
}

impl HttpsListenerReloadable {
    /// Create a new `HttpsListenerReloadable` bound to the boot-time config.
    pub(crate) fn new(boot: HttpsConfig) -> Self {
        Self { boot }
    }
}

impl Reloadable for HttpsListenerReloadable {
    /// Receives the full `NetworkConfig`; only the `https` field is used.
    type Config = NetworkConfig;

    fn name(&self) -> &'static str {
        "https_listener"
    }

    /// Reject any change to `network.https.addr` — the listener socket is
    /// bound once at boot and cannot be rebound without a full restart.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the address changed.
    fn validate(&self, new: &NetworkConfig) -> Result<(), Report> {
        if new.https.addr != self.boot.addr {
            bail!(ConfigReloadError::Validate(
                "listener address change requires a full controller restart (network https addr)"
                    .into()
            ));
        }
        Ok(())
    }

    /// No-op — validate already rejected any change that would require
    /// action here.
    async fn apply(&self, _new: std::sync::Arc<NetworkConfig>) -> Result<(), Report> {
        Ok(())
    }

    /// No-op — nothing was mutated by `apply`.
    async fn revert(&self) -> Result<(), Report> {
        Ok(())
    }

    /// Validate-only gate controls no socket; health is always OK if
    /// validate passed.
    async fn health_check(&self) -> Result<(), Report> {
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_HTTPS
    }
}

uptrakit_config_reload::reloadable_erased_impl!(
    HttpsListenerReloadable,
    RuntimeConfigDelta::Network
);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_https_cfg(addr: &str) -> HttpsConfig {
        let mut cfg = HttpsConfig::default();
        cfg.addr = addr.into();
        cfg
    }

    #[test]
    fn https_validate_rejects_addr_change() {
        let boot = make_https_cfg("127.0.0.1:0");
        let r = HttpsListenerReloadable::new(boot);
        let mut net = NetworkConfig::default();
        net.https = make_https_cfg("127.0.0.1:9999");

        let err = r.validate(&net).unwrap_err();
        assert!(
            err.to_string()
                .contains("requires a full controller restart")
        );
    }

    #[test]
    fn https_validate_accepts_unchanged_addr() {
        let boot = make_https_cfg("127.0.0.1:0");
        let r = HttpsListenerReloadable::new(boot.clone());
        let mut net = NetworkConfig::default();
        net.https = boot;

        r.validate(&net).unwrap();
    }
}
