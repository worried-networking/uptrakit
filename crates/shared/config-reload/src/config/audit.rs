use std::collections::HashMap;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// Audit log configuration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct AuditConfig {
    /// Event filter: `"all"`, `"mutations"`, or `"none"`.
    #[serde(default = "default_filter")]
    pub filter: String,
    /// Number of days to retain audit log entries.
    #[serde(default = "default_retention")]
    pub retention_days: u32,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

fn default_filter() -> String {
    "all".into()
}
const fn default_retention() -> u32 {
    90
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            filter: default_filter(),
            retention_days: default_retention(),
            extra: HashMap::new(),
        }
    }
}

const VALID_FILTERS: &[&str] = &["all", "mutations", "none"];

impl AuditConfig {
    /// Create a new `AuditConfig` with the given filter and retention period.
    #[must_use]
    pub fn new(filter: impl Into<String>, retention_days: u32) -> Self {
        Self {
            filter: filter.into(),
            retention_days,
            extra: HashMap::new(),
        }
    }

    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Returns an error if `filter` is not one of `"all"`, `"mutations"`,
    /// or `"none"`.
    pub fn validate(&self) -> Result<(), Report> {
        if !VALID_FILTERS.contains(&self.filter.as_str()) {
            bail!(ConfigReloadError::Validate(format!(
                "audit.filter must be one of {VALID_FILTERS:?}, got {:?}",
                self.filter
            )));
        }
        Ok(())
    }
}
