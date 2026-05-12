pub mod audit;
pub mod db;
pub mod embedded;
pub mod log;
pub mod master_key;
pub mod nats;
pub mod network;
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
pub use scope::Scope;
pub use tls::TlsConfig;
pub use zeroconf::ZeroconfConfig;

// RuntimeConfig and cross-section validation added in Task 6.
/// Placeholder — expanded in Task 6.
pub struct RuntimeConfig;

impl RuntimeConfig {
    /// Placeholder validator.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<(), rootcause::Report> {
        Ok(())
    }

    /// Placeholder extras warning collector.
    #[must_use]
    pub fn warn_about_extras(&self) -> Vec<String> {
        Vec::new()
    }
}
