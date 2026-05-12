use std::collections::HashMap;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// Logging configuration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct LogConfig {
    /// Path to the log file.
    #[serde(default)]
    pub path: String,
    /// Log level: one of `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`.
    #[serde(default = "default_level")]
    pub level: String,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

fn default_level() -> String {
    "info".into()
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            level: default_level(),
            extra: HashMap::new(),
        }
    }
}

impl LogConfig {
    /// Create a new `LogConfig` with the given path and level.
    #[must_use]
    pub fn new(path: impl Into<String>, level: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            level: level.into(),
            extra: HashMap::new(),
        }
    }

    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` is empty or `level` is not a valid
    /// [`tracing::Level`].
    pub fn validate(&self) -> Result<(), Report> {
        if self.path.is_empty() {
            bail!(ConfigReloadError::Validate("log.path is empty".into()));
        }
        self.level.parse::<tracing::Level>().map_err(|e| {
            report!(ConfigReloadError::Validate(format!(
                "log.level {:?} is not a valid tracing level: {e}",
                self.level
            )))
        })?;
        Ok(())
    }
}
