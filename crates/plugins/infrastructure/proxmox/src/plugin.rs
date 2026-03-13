use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{PluginCapability, command::CommandExecutor};

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
    /// The controller-side plugin owns controller database migrations for
    /// the `proxmox_host_mappings` table. It has no agent-side capabilities —
    /// it operates entirely on the controller via extension actions.
    #[cfg(feature = "migrations")]
    pub const CAPABILITIES: &'static [PluginCapability] = &[PluginCapability::ControllerMigrations];

    /// Compile-time capabilities for the Proxmox VE plugin (without migrations).
    #[cfg(not(feature = "migrations"))]
    pub const CAPABILITIES: &'static [PluginCapability] = &[];

    /// Create a new Proxmox VE plugin instance.
    pub async fn new(
        config: ProxmoxConfig,
        _executor: Arc<dyn CommandExecutor>,
    ) -> uptrakit_plugin_infrastructure_core::Result<Self> {
        Ok(Self { _config: config })
    }
}

// ── PluginBase implementation ────────────────────────────────────────────

uptrakit_plugin_infrastructure_core::impl_plugin_base_config!(
    ProxmoxPlugin,
    ProxmoxConfig,
    "infrastructure_proxmox",
    {
        fn capabilities(&self) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
            Self::CAPABILITIES.to_vec()
        }

        #[cfg(feature = "migrations")]
        fn controller_migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            crate::controller_migration::migrations()
        }
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{LocalCommandExecutor, PluginBase, SecretString};

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
        assert_eq!(plugin.plugin_type_id(), "infrastructure_proxmox");
    }

    #[tokio::test]
    async fn capabilities_match_expected() {
        let config = ProxmoxConfig::default();
        let plugin = ProxmoxPlugin::new(config, test_executor())
            .await
            .expect("create");
        assert_eq!(plugin.capabilities(), ProxmoxPlugin::CAPABILITIES);
    }
}
