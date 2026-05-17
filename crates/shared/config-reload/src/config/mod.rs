pub mod audit;
pub mod db;
pub mod embedded;
pub mod log;
pub mod nats;
pub mod network;
pub mod plugins;
pub mod scope;
pub mod tls;
pub mod zeroconf;

pub use audit::AuditConfig;
pub use db::DbConfig;
pub use embedded::EmbeddedServicesConfig;
pub use log::LogConfig;
pub use nats::NatsConfig;
pub use network::{HttpsConfig, NetworkConfig};
pub use plugins::PluginsConfig;
pub use scope::Scope;
pub use tls::TlsConfig;
pub use zeroconf::ZeroconfConfig;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;

use crate::error::ConfigReloadError;

/// Top-level runtime configuration for the uptrakit Controller.
///
/// Parsed from a TOML file via [`crate::loader::TomlConfigLoader`].
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct RuntimeConfig {
    /// Database connection and pool settings.
    #[serde(default)]
    pub db: DbConfig,
    /// Encryption master key source.
    ///
    /// Accepts three forms:
    /// - `"file:<path>"` — absolute path to a key file
    /// - `"env:<VAR>"` — name of an environment variable holding the key
    /// - `"<64-hex-chars>"` — inline key material (requires `chmod 0600` on the config file)
    #[serde(default)]
    pub master_key: SecretString,
    /// Network listener settings (HTTPS + PKI).
    pub network: NetworkConfig,
    /// TLS certificate and key settings. Optional: when omitted the
    /// controller falls back to managed self-signed certificates.
    #[serde(default)]
    pub tls: TlsConfig,
    /// NATS messaging server settings.
    pub nats: NatsConfig,
    /// Audit log settings.
    #[serde(default)]
    pub audit: AuditConfig,
    /// Logging settings.
    #[serde(default)]
    pub log: LogConfig,
    /// Zero-configuration auto-discovery settings.
    #[serde(default)]
    pub zeroconf: ZeroconfConfig,
    /// Which services run embedded inside the controller binary.
    #[serde(default)]
    pub embedded_services: EmbeddedServicesConfig,
}

impl RuntimeConfig {
    /// Validate all config sections.
    ///
    /// # Errors
    ///
    /// Returns the first validation error encountered across all sections.
    pub fn validate(&self) -> Result<(), Report> {
        self.db.validate()?;
        validate_master_key(self.master_key.expose_secret())?;
        self.network.validate()?;
        self.tls.validate()?;
        self.nats.validate()?;
        self.audit.validate()?;
        self.log.validate()?;
        self.zeroconf.validate()?;
        self.embedded_services.validate()?;
        Ok(())
    }

    /// Collect warnings about unknown keys found in each config section.
    ///
    /// Returns one warning string per unknown key, formatted as
    /// `"[section] unknown key `key` ignored"`.
    #[must_use]
    pub fn warn_about_extras(&self) -> Vec<String> {
        let mut out = Vec::new();
        for key in self.db.extra.keys() {
            out.push(format!("[db] unknown key `{key}` ignored"));
        }
        for key in self.network.extra.keys() {
            out.push(format!("[network] unknown key `{key}` ignored"));
        }
        for key in self.tls.extra.keys() {
            out.push(format!("[tls] unknown key `{key}` ignored"));
        }
        for key in self.nats.extra.keys() {
            out.push(format!("[nats] unknown key `{key}` ignored"));
        }
        for key in self.audit.extra.keys() {
            out.push(format!("[audit] unknown key `{key}` ignored"));
        }
        for key in self.log.extra.keys() {
            out.push(format!("[log] unknown key `{key}` ignored"));
        }
        for key in self.zeroconf.extra.keys() {
            out.push(format!("[zeroconf] unknown key `{key}` ignored"));
        }
        for key in self.embedded_services.extra.keys() {
            out.push(format!("[embedded_services] unknown key `{key}` ignored"));
        }
        out
    }
}

/// Validate the master key source string.
///
/// Accepts:
/// - `"file:<path>"` — the path portion must be non-empty
/// - `"env:<VAR>"` — the variable name must be non-empty
/// - `"<64-hex-chars>"` — exactly 64 lowercase or uppercase hex characters
///
/// # Errors
///
/// Returns an error if the value is empty or structurally invalid.
pub fn validate_master_key(key: &str) -> Result<(), Report> {
    if key.is_empty() {
        bail!(ConfigReloadError::Validate("master_key is empty".into()));
    }
    if let Some(path) = key.strip_prefix("file:") {
        if path.is_empty() {
            bail!(ConfigReloadError::Validate(
                "master_key file: path is empty".into()
            ));
        }
        return Ok(());
    }
    if let Some(var) = key.strip_prefix("env:") {
        if var.is_empty() {
            bail!(ConfigReloadError::Validate(
                "master_key env: variable name is empty".into()
            ));
        }
        return Ok(());
    }
    // Inline hex: must be exactly 64 hex chars
    if key.len() != 64 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!(ConfigReloadError::Validate(
            "master_key inline value must be exactly 64 hexadecimal characters".into()
        ));
    }
    Ok(())
}
