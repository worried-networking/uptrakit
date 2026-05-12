use crate::config::RuntimeConfig;

/// The result of loading and parsing a TOML config file.
#[non_exhaustive]
pub struct LoadedConfig {
    /// The parsed runtime configuration.
    pub config: RuntimeConfig,
    /// Warnings about unknown keys that were ignored.
    pub warnings: Vec<String>,
}

/// Loads and validates a TOML config file.
pub struct TomlConfigLoader;

impl TomlConfigLoader {
    /// Placeholder — full implementation in Task 7.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, parsed, or validated.
    pub fn load(_path: impl AsRef<std::path::Path>) -> Result<LoadedConfig, rootcause::Report> {
        use rootcause::prelude::*;
        bail!(crate::error::ConfigReloadError::Reconciler(
            "loader not yet implemented".into()
        ))
    }

    /// Placeholder — full implementation in Task 7.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, parsed, or validated.
    pub fn validate_only(_path: impl AsRef<std::path::Path>) -> Result<(), rootcause::Report> {
        use rootcause::prelude::*;
        bail!(crate::error::ConfigReloadError::Reconciler(
            "loader not yet implemented".into()
        ))
    }
}
