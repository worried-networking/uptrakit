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
    /// SPIFFE trust domain for mTLS identity.
    ///
    /// When set, the controller embeds a `spiffe://<trust_domain>/service/<id>` URI SAN in
    /// every signed agent certificate and validates incoming CSRs against this domain.
    /// Must contain only lowercase letters, digits, dots, and hyphens.
    /// When absent, `sans[0]` is used as the effective trust domain (legacy fallback).
    #[serde(default)]
    pub trust_domain: String,
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
            trust_domain: String::new(),
            extra: HashMap::new(),
        }
    }

    /// Returns the configured trust domain, or derives one from `sans`.
    ///
    /// Returns an empty string if no trust domain is configured and `sans` is empty.
    #[must_use]
    pub fn effective_trust_domain<'a>(&'a self, sans: &'a [String]) -> &'a str {
        if !self.trust_domain.is_empty() {
            return &self.trust_domain;
        }
        sans.first().map(String::as_str).unwrap_or("")
    }

    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Returns an error if `cert_path` or `key_path` is empty, or if
    /// `trust_domain` contains characters outside `[a-z0-9.-]`.
    pub fn validate(&self) -> Result<(), Report> {
        if self.cert_path.is_empty() {
            bail!(ConfigReloadError::Validate("tls.cert_path is empty".into()));
        }
        if self.key_path.is_empty() {
            bail!(ConfigReloadError::Validate("tls.key_path is empty".into()));
        }
        if !self.trust_domain.is_empty()
            && !self
                .trust_domain
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '.' | '-'))
        {
            bail!(ConfigReloadError::Validate(format!(
                "tls.trust_domain contains invalid characters: {}",
                self.trust_domain
            )));
        }
        Ok(())
    }
}
