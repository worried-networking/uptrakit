pub mod audit;
pub mod db;
pub mod embedded;
pub mod log;
pub mod master_key;
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
pub use master_key::MasterKeyConfig;
pub use nats::NatsConfig;
pub use network::{HttpsConfig, NetworkConfig, PkiConfig};
pub use plugins::PluginsConfig;
pub use scope::Scope;
pub use tls::TlsConfig;
pub use zeroconf::ZeroconfConfig;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

/// Top-level runtime configuration for the uptrakit Controller.
///
/// Parsed from a TOML file via [`crate::loader::TomlConfigLoader`].
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct RuntimeConfig {
    /// Database connection and pool settings.
    #[serde(default)]
    pub db: DbConfig,
    /// Encryption master key settings.
    pub master_key: MasterKeyConfig,
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
        self.master_key.validate()?;
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
        for key in self.master_key.extra.keys() {
            out.push(format!("[master_key] unknown key `{key}` ignored"));
        }
        for key in self.network.extra.keys() {
            out.push(format!("[network] unknown key `{key}` ignored"));
        }
        for key in self.network.https.extra.keys() {
            out.push(format!("[network.https] unknown key `{key}` ignored"));
        }
        for key in self.network.pki.extra.keys() {
            out.push(format!("[network.pki] unknown key `{key}` ignored"));
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
