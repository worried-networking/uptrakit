use std::path::Path;

use rootcause::prelude::*;

use crate::config::RuntimeConfig;
use crate::error::ConfigReloadError;

/// The result of loading and parsing a TOML config file.
#[non_exhaustive]
pub struct LoadedConfig {
    /// The parsed and validated runtime configuration.
    pub config: RuntimeConfig,
    /// Warnings about unknown keys that were ignored during parse.
    pub warnings: Vec<String>,
}

/// Loads and validates a TOML config file from disk.
pub struct TomlConfigLoader;

impl TomlConfigLoader {
    /// Read, parse, validate, and return the config at `path`.
    ///
    /// Unknown keys are captured in each section's `extra` map and surfaced
    /// as [`LoadedConfig::warnings`] rather than causing a parse failure.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, the TOML is malformed,
    /// or any section fails [`RuntimeConfig::validate`].
    pub fn load(path: impl AsRef<Path>) -> Result<LoadedConfig, Report> {
        let path = path.as_ref();
        let bytes = std::fs::read_to_string(path).map_err(|e| {
            report!(ConfigReloadError::TomlIo {
                path: path.to_path_buf(),
                source_msg: e.to_string(),
            })
        })?;
        let config: RuntimeConfig = toml::from_str(&bytes).map_err(|e| {
            report!(ConfigReloadError::TomlParse {
                path: path.to_path_buf(),
                source_msg: e.to_string(),
            })
        })?;
        config.validate()?;
        let warnings = config.warn_about_extras();
        Ok(LoadedConfig { config, warnings })
    }

    /// Read, parse, and validate the config at `path` without returning it.
    ///
    /// Suitable for pre-flight checks (e.g. `uptrakit-ctl config validate`).
    /// Any unknown-key warnings are printed to stderr.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, the TOML is malformed,
    /// or validation fails.
    pub fn validate_only(path: impl AsRef<Path>) -> Result<(), Report> {
        let loaded = Self::load(path)?;
        for w in &loaded.warnings {
            eprintln!("warning: {w}");
        }
        Ok(())
    }
}
