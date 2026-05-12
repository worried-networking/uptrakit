use std::collections::HashMap;

use rootcause::Report;
use serde::{Deserialize, Serialize};

/// Embedded services configuration (which services run inside the controller).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct EmbeddedServicesConfig {
    /// Whether the Agent service runs embedded in the controller binary.
    #[serde(default)]
    pub agent: bool,
    /// Whether the Agent-SSH service runs embedded in the controller binary.
    #[serde(default)]
    pub agent_ssh: bool,
    /// Whether the MQTT broker runs embedded in the controller binary.
    #[serde(default)]
    pub mqtt: bool,
    /// Whether the Scheduler service runs embedded in the controller binary.
    #[serde(default)]
    pub scheduler: bool,
    /// Unknown keys collected for `warn_about_extras`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl EmbeddedServicesConfig {
    /// Validate this config section.
    ///
    /// # Errors
    ///
    /// Currently infallible; reserved for future cross-field constraints.
    pub fn validate(&self) -> Result<(), Report> {
        Ok(())
    }
}
