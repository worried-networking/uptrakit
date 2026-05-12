use std::collections::HashMap;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// NATS messaging server connection configuration.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct NatsConfig {
    /// NATS server URL (e.g. `nats://localhost:4222`).
    #[serde(default)]
    pub url: String,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl NatsConfig {
    /// Create a new `NatsConfig` with the given URL.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            extra: HashMap::new(),
        }
    }

    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Returns an error if `url` is empty.
    pub fn validate(&self) -> Result<(), Report> {
        if self.url.is_empty() {
            bail!(ConfigReloadError::Validate("nats.url is empty".into()));
        }
        Ok(())
    }
}
