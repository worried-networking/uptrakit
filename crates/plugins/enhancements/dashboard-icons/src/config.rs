use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::PluginConfig;

/// Enhancement plugin config placeholder.
///
/// Dashboard Icons has no per-instance configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardIconsConfig;

impl PluginConfig for DashboardIconsConfig {}
