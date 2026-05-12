use std::collections::HashMap;
use std::path::Path;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// Encryption master key configuration.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct MasterKeyConfig {
    /// Absolute path to the master key file.
    #[serde(default)]
    pub path: String,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl MasterKeyConfig {
    /// Create a new `MasterKeyConfig` with the given path.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            extra: HashMap::new(),
        }
    }

    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` is empty or is not an absolute path.
    pub fn validate(&self) -> Result<(), Report> {
        if self.path.is_empty() {
            bail!(ConfigReloadError::Validate(
                "master_key.path is empty".into()
            ));
        }
        if !Path::new(&self.path).is_absolute() {
            bail!(ConfigReloadError::Validate(format!(
                "master_key.path {:?} must be an absolute path",
                self.path
            )));
        }
        Ok(())
    }
}
