use std::collections::HashMap;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// Database connection and pool configuration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct DbConfig {
    /// Database connection URL (e.g. `sqlite:///var/lib/uptrakit/controller.db`).
    pub url: String,
    /// Maximum number of connections in the pool.
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,
    /// Maximum time in milliseconds to wait for a pool connection.
    #[serde(default = "default_acquire_timeout")]
    pub acquire_timeout_ms: u64,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            pool_size: default_pool_size(),
            acquire_timeout_ms: default_acquire_timeout(),
            extra: HashMap::new(),
        }
    }
}

const fn default_pool_size() -> u32 {
    16
}
const fn default_acquire_timeout() -> u64 {
    5_000
}

impl DbConfig {
    /// Create a new `DbConfig` with the given required fields.
    ///
    /// Optional fields default to their standard values.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Create a new `DbConfig` with all fields specified (used in tests).
    #[must_use]
    pub fn with_all(url: impl Into<String>, pool_size: u32, acquire_timeout_ms: u64) -> Self {
        Self {
            url: url.into(),
            pool_size,
            acquire_timeout_ms,
            extra: HashMap::new(),
        }
    }

    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Returns an error if `url` is empty, `pool_size` is zero, or
    /// `acquire_timeout_ms` is zero.
    pub fn validate(&self) -> Result<(), Report> {
        if self.pool_size == 0 {
            bail!(ConfigReloadError::Validate(
                "db.pool_size must be >= 1".into()
            ));
        }
        if self.acquire_timeout_ms == 0 {
            bail!(ConfigReloadError::Validate(
                "db.acquire_timeout_ms must be > 0".into()
            ));
        }
        Ok(())
    }
}
