use std::collections::HashMap;
use std::net::SocketAddr;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// Network listener settings — HTTPS and PKI advertisement address.
///
/// In TOML all fields appear directly under `[network]`:
/// ```toml
/// [network]
/// addr    = "0.0.0.0:8443"
/// pki_addr = "http://controller.example.com:8444"
/// ```
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct NetworkConfig {
    /// HTTPS listener settings (addr, proxy headers, …).
    ///
    /// Flattened into `[network]` in TOML — callers read e.g. `config.network.https.addr`.
    #[serde(flatten)]
    pub https: HttpsConfig,
    /// PKI advertisement address.
    ///
    /// Accepts either a bare `host:port` socket address (used as both bind and
    /// advertised address) or an `http://` URL (advertised only; the actual
    /// listener is managed separately). `https://` is explicitly rejected.
    #[serde(default)]
    pub pki_addr: String,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// HTTPS listener settings (embedded inside [`NetworkConfig`] via `#[serde(flatten)]`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct HttpsConfig {
    /// TCP address and port to listen on (e.g. `0.0.0.0:8443`).
    pub addr: String,
    /// CIDR ranges of trusted reverse proxies.
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    /// Header name carrying the real client IP (set by the proxy).
    #[serde(default = "default_real_ip")]
    pub real_ip_header: String,
    /// Header carrying forwarded client certificate info.
    #[serde(default = "default_fcc_info")]
    pub forwarded_client_cert_info_header: String,
    /// Header carrying the forwarded client certificate PEM.
    #[serde(default = "default_fcc_pem")]
    pub forwarded_client_cert_pem_header: String,
    // No `extra` field: NetworkConfig.extra is the single flatten catch-all.
}

fn default_real_ip() -> String {
    "x-forwarded-for".into()
}
fn default_fcc_info() -> String {
    "x-forwarded-client-cert".into()
}
fn default_fcc_pem() -> String {
    "x-forwarded-client-cert-pem".into()
}

impl NetworkConfig {
    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// - `addr` is not a valid `SocketAddr`
    /// - `pki_addr` uses the `https://` scheme (must be `http://` or a bare socket address)
    /// - `pki_addr` (bare socket addr form) collides with `addr`
    pub fn validate(&self) -> Result<(), Report> {
        self.https.addr.parse::<SocketAddr>().map_err(|e| {
            report!(ConfigReloadError::Validate(format!(
                "network.addr invalid: {e}"
            )))
        })?;
        if self.pki_addr.starts_with("https://") {
            bail!(ConfigReloadError::Validate(
                "network.pki_addr must not use https:// scheme; use http:// or a bare socket address".into()
            ));
        }
        if !self.pki_addr.is_empty() && self.pki_addr == self.https.addr {
            bail!(ConfigReloadError::Validate(format!(
                "network.pki_addr ({}) collides with network.addr",
                self.pki_addr,
            )));
        }
        Ok(())
    }
}
