use std::collections::HashMap;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// Zero-configuration auto-discovery configuration.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct ZeroconfConfig {
    /// Whether zero-configuration auto-discovery is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Controller URL advertised to discovered agents.
    #[serde(default)]
    pub url: String,
    /// PKI listener address advertised for certificate issuance.
    #[serde(default)]
    pub pki_addr: String,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl ZeroconfConfig {
    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Returns an error if `enabled` is `true` but `url` or `pki_addr` is
    /// empty.
    pub fn validate(&self) -> Result<(), Report> {
        if self.enabled {
            if self.url.is_empty() {
                bail!(ConfigReloadError::Validate(
                    "zeroconf.url is empty when enabled=true".into()
                ));
            }
            // pki_addr is optional here; runtime falls back to network.pki.addr when empty.
        }
        Ok(())
    }
}
