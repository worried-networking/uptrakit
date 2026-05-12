use std::collections::HashMap;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// TLS certificate and key configuration for the HTTPS listener.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct TlsConfig {
    /// Path to the TLS certificate file.
    #[serde(default)]
    pub cert_path: String,
    /// Path to the TLS private key file.
    #[serde(default)]
    pub key_path: String,
    /// Subject alternative names for the certificate.
    #[serde(default)]
    pub sans: Vec<String>,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl TlsConfig {
    /// Create a new `TlsConfig` with the given cert and key paths.
    #[must_use]
    pub fn new(cert_path: impl Into<String>, key_path: impl Into<String>) -> Self {
        Self {
            cert_path: cert_path.into(),
            key_path: key_path.into(),
            sans: Vec::new(),
            extra: HashMap::new(),
        }
    }

    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Returns an error if `cert_path` or `key_path` is empty.
    pub fn validate(&self) -> Result<(), Report> {
        if self.cert_path.is_empty() {
            bail!(ConfigReloadError::Validate("tls.cert_path is empty".into()));
        }
        if self.key_path.is_empty() {
            bail!(ConfigReloadError::Validate("tls.key_path is empty".into()));
        }
        Ok(())
    }
}
