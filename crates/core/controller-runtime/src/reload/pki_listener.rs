//! PKI listener address reload gate.
//!
//! [`PkiListenerReloadable`] is a validate-only gate: the PKI (certificate
//! authority) listener's bound address is fixed at boot, so this subsystem
//! never changes any runtime state. It exists solely to reject a config
//! reload that would silently leave a changed `network.pki_addr` unapplied.
//!
//! `pki_addr` accepts either a bare `host:port` socket address (used as both
//! bind and advertised address) or an `http://` URL (advertised only). Both
//! forms are compared as opaque strings — any change, of either form,
//! requires a restart.

use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_config_reload::config::NetworkConfig;
use uptrakit_config_reload::defaults::WATCHDOG_PKI;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

/// A [`Reloadable`] validate-reject gate for `network.pki_addr`.
///
/// Marked `#[non_exhaustive]` so that additional diagnostic fields can be
/// added without a semver break.
#[non_exhaustive]
pub(crate) struct PkiListenerReloadable {
    /// The PKI address this process booted with.
    boot_pki_addr: String,
}

impl PkiListenerReloadable {
    /// Create a new `PkiListenerReloadable` bound to the boot-time address.
    pub(crate) fn new(boot_pki_addr: String) -> Self {
        Self { boot_pki_addr }
    }
}

impl Reloadable for PkiListenerReloadable {
    /// Receives the full `NetworkConfig`; only `pki_addr` is used.
    type Config = NetworkConfig;

    fn name(&self) -> &'static str {
        "pki_listener"
    }

    /// Reject any change to `network.pki_addr` — the listener is bound once
    /// at boot and cannot be rebound without a full restart.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::Validate`] if the address changed.
    fn validate(&self, new: &NetworkConfig) -> Result<(), Report> {
        if new.pki_addr != self.boot_pki_addr {
            bail!(ConfigReloadError::Validate(
                "listener address change requires a full controller restart (network pki_addr)"
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

    #[test]
    fn pki_validate_rejects_addr_change() {
        let boot_addr = "127.0.0.1:0".to_string();
        let r = PkiListenerReloadable::new(boot_addr);
        let mut net = NetworkConfig::default();
        net.pki_addr = "127.0.0.1:9999".to_string();

        let err = r.validate(&net).unwrap_err();
        assert!(
            err.to_string()
                .contains("requires a full controller restart")
        );
    }

    #[test]
    fn pki_validate_accepts_unchanged_addr() {
        let boot_addr = "127.0.0.1:0".to_string();
        let r = PkiListenerReloadable::new(boot_addr.clone());
        let mut net = NetworkConfig::default();
        net.pki_addr = boot_addr;

        r.validate(&net).unwrap();
    }
}
