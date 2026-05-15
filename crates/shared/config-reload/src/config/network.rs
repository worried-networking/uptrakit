use std::collections::HashMap;
use std::net::SocketAddr;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// Top-level network configuration grouping HTTPS and PKI listener settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct NetworkConfig {
    /// HTTPS listener configuration.
    pub https: HttpsConfig,
    /// PKI (certificate authority) listener configuration.
    pub pki: PkiConfig,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// HTTPS listener settings.
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
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// PKI (internal CA) listener settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct PkiConfig {
    /// TCP address and port to listen on (e.g. `0.0.0.0:8444`).
    pub addr: String,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
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
    /// Returns an error if `https.addr` or `pki.addr` is not a valid
    /// `SocketAddr`, or if the two addrs are identical.
    pub fn validate(&self) -> Result<(), Report> {
        self.https.addr.parse::<SocketAddr>().map_err(|e| {
            report!(ConfigReloadError::Validate(format!(
                "network.https.addr invalid: {e}"
            )))
        })?;
        // network.pki.addr is the public PKI URL (e.g. `http://hostname:8080`),
        // not a bind address — SocketAddr parsing does not apply. We still
        // reject the collision case where a misconfigured deployment puts
        // both listeners on the same string addr (would race for the port).
        if !self.pki.addr.is_empty() && self.pki.addr == self.https.addr {
            bail!(ConfigReloadError::Validate(format!(
                "network.pki.addr ({}) collides with network.https.addr",
                self.pki.addr,
            )));
        }
        Ok(())
    }
}
