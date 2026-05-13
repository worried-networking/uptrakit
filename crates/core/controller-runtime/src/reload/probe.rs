//! Shared probe-address helper for listener health checks.
//!
//! Both [`super::https_listener`] and the future PKI-listener reloadable need
//! to convert a bound TCP address into one suitable for loopback probe
//! connections.

use rootcause::prelude::*;
use uptrakit_config_reload::error::ConfigReloadError;

/// Resolve a bound TCP address to one suitable for probe connections.
///
/// Replaces an unspecified IP (`0.0.0.0` / `::`) with `127.0.0.1` so that
/// health-check probes can connect even when the listener is bound to all
/// interfaces.
///
/// # Errors
///
/// Returns [`ConfigReloadError::HealthFailed`] if `bound` is not a valid
/// [`std::net::SocketAddr`].
pub(crate) fn pick_probe_addr(bound: &str) -> Result<String, Report> {
    let sa: std::net::SocketAddr = bound.parse().map_err(|e: std::net::AddrParseError| {
        report!(ConfigReloadError::HealthFailed {
            subsystem: "listener".into(),
            message: e.to_string(),
        })
    })?;
    if sa.ip().is_unspecified() {
        Ok(format!("127.0.0.1:{}", sa.port()))
    } else {
        Ok(bound.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_probe_addr_replaces_unspecified() {
        assert_eq!(pick_probe_addr("0.0.0.0:8443").unwrap(), "127.0.0.1:8443");
    }

    #[test]
    fn pick_probe_addr_keeps_specific() {
        assert_eq!(
            pick_probe_addr("192.168.1.1:8443").unwrap(),
            "192.168.1.1:8443"
        );
    }

    #[test]
    fn pick_probe_addr_rejects_invalid() {
        assert!(pick_probe_addr("not-an-addr").is_err());
    }
}
