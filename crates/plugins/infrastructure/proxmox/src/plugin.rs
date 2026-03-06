use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    Plugin, PluginCapability, PluginType, command::CommandExecutor,
};

use crate::config::ProxmoxConfig;

/// Proxmox VE infrastructure plugin.
///
/// This plugin is a controller-side plugin that communicates with the Proxmox
/// VE REST API to discover VMs/CTs and match them to Uptrakit-managed hosts.
/// It does not run on agents and has no local command execution needs.
///
/// All user interaction goes through the Extensions framework (pages and panels).
pub struct ProxmoxPlugin {
    _config: ProxmoxConfig,
}

impl ProxmoxPlugin {
    /// Compile-time capabilities for the Proxmox VE plugin.
    ///
    /// This plugin has no agent-side capabilities — it operates entirely on
    /// the controller via extension actions.
    pub const CAPABILITIES: &'static [PluginCapability] = &[];

    /// Create a new Proxmox VE plugin instance.
    pub async fn new(
        config: ProxmoxConfig,
        _executor: Arc<dyn CommandExecutor>,
    ) -> uptrakit_plugin_infrastructure_core::Result<Self> {
        Ok(Self { _config: config })
    }
}

#[async_trait]
impl Plugin for ProxmoxPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::InfrastructureProxmox
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        Self::CAPABILITIES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{LocalCommandExecutor, SecretString};

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    #[tokio::test]
    async fn plugin_type_is_infrastructure_proxmox() {
        let config = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=secret".to_string()),
            ..ProxmoxConfig::default()
        };
        let plugin = ProxmoxPlugin::new(config, test_executor())
            .await
            .expect("create");
        assert_eq!(plugin.plugin_type(), PluginType::InfrastructureProxmox);
    }

    #[tokio::test]
    async fn capabilities_is_empty() {
        let config = ProxmoxConfig::default();
        let plugin = ProxmoxPlugin::new(config, test_executor())
            .await
            .expect("create");
        assert!(plugin.capabilities().is_empty());
    }
}
