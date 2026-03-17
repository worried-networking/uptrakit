use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{PluginCapability, command::CommandExecutor};

use crate::config::ProxmoxConfig;

/// Proxmox VE infrastructure plugin.
///
/// Unified plugin struct used on both the controller side (with config) and
/// the agent side (without config). On the controller it communicates with
/// the Proxmox VE REST API to discover VMs/CTs and match them to
/// Uptrakit-managed hosts. On the agent it implements infrastructure subtraits
/// for host lifecycle, host reporting, and guest execution.
///
/// All user interaction goes through the Extensions framework (pages and panels).
pub struct ProxmoxPlugin {
    _config: Option<ProxmoxConfig>,
}

impl ProxmoxPlugin {
    /// Compile-time capabilities for the Proxmox VE plugin.
    ///
    /// When the `migrations` feature is enabled the plugin declares
    /// `ControllerMigrations` so the controller runs its DB schema.
    /// Without the feature the capability list is empty.
    ///
    /// NOTE: the `#[cfg(not)]` here is intentional — both branches define the
    /// same constant name for different feature combinations, which is the only
    /// supported way to provide divergent const values in Rust without a
    /// runtime conditional.
    #[cfg(all(feature = "migrations", not(feature = "agent-infra")))]
    pub const CAPABILITIES: &'static [PluginCapability] = &[PluginCapability::ControllerMigrations];

    #[cfg(feature = "agent-infra")]
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::HostLifecycle,
        PluginCapability::HostReport,
        PluginCapability::GuestExec,
        PluginCapability::ServiceMigrations,
    ];

    #[cfg(not(feature = "migrations"))]
    pub const CAPABILITIES: &'static [PluginCapability] = &[];

    /// Create a new controller-side Proxmox VE plugin instance (with config).
    pub async fn new(
        config: ProxmoxConfig,
        _executor: Arc<dyn CommandExecutor>,
    ) -> uptrakit_plugin_infrastructure_core::Result<Self> {
        Ok(Self {
            _config: Some(config),
        })
    }

    /// Create a new agent-side Proxmox VE plugin instance (no config).
    pub fn new_agent() -> Self {
        Self { _config: None }
    }
}

impl Default for ProxmoxPlugin {
    fn default() -> Self {
        Self::new_agent()
    }
}

// ── PluginBase implementation ────────────────────────────────────────────

#[async_trait::async_trait]
impl uptrakit_plugin_infrastructure_core::PluginBase for ProxmoxPlugin {
    fn plugin_type_id(&self) -> &str {
        "infrastructure_proxmox"
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        Self::CAPABILITIES.to_vec()
    }

    fn validate_config(&self, config: &serde_json::Value) -> std::result::Result<(), String> {
        let typed: ProxmoxConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("failed to parse config: {e}"))?;
        typed.validate().map_err(|e| e.to_string())
    }

    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        let Ok(cfg) = serde_json::from_value::<ProxmoxConfig>(config.clone()) else {
            return config.clone();
        };
        use uptrakit_plugin_infrastructure_core::SecretMasking;
        match serde_json::to_value(cfg.with_secrets_masked()) {
            Ok(masked) => masked,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "failed to serialize masked plugin config"
                );
                config.clone()
            }
        }
    }

    fn restore_config_secrets(
        &self,
        incoming: &serde_json::Value,
        stored: &serde_json::Value,
    ) -> serde_json::Value {
        let (Ok(mut inc), Ok(ex)) = (
            serde_json::from_value::<ProxmoxConfig>(incoming.clone()),
            serde_json::from_value::<ProxmoxConfig>(stored.clone()),
        ) else {
            return incoming.clone();
        };
        use uptrakit_plugin_infrastructure_core::SecretMasking;
        inc.restore_secrets_from(&ex);
        serde_json::to_value(&inc).unwrap_or_else(|_| incoming.clone())
    }

    fn form_schema(&self) -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        <ProxmoxConfig as uptrakit_plugin_infrastructure_core::ConfigFormSchema>::form_schema()
    }

    fn type_settings_form_schema(
        &self,
    ) -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        <ProxmoxConfig as uptrakit_plugin_infrastructure_core::ConfigFormSchema>::type_settings_form_schema()
    }

    fn type_settings_sample(&self) -> serde_json::Value {
        <ProxmoxConfig as uptrakit_plugin_infrastructure_core::ConfigFormSchema>::type_settings_sample()
    }

    fn sample_config(&self) -> serde_json::Value {
        serde_json::to_value(ProxmoxConfig::default()).unwrap_or_else(|_| serde_json::json!({}))
    }

    fn validate_package_identifier(&self, value: &str) -> std::result::Result<(), String> {
        ProxmoxConfig::validate_identifier(value)
    }

    fn extension_manifests() -> Vec<uptrakit_extension_framework::ExtensionManifest>
    where
        Self: Sized,
    {
        // Controller-side manifests are only relevant when not in agent mode.
        if cfg!(feature = "agent-infra") {
            // Agent mode: no top-level manifests — the Proxmox plugin contributes
            // actions to the SSH agent's existing `ssh-agent.hosts` manifest.
            return vec![];
        }
        crate::extensions::extension_manifests()
    }

    fn extension_actions() -> Vec<uptrakit_extension_framework::ActionDef>
    where
        Self: Sized,
    {
        let mut actions = Vec::new();
        // Controller-side actions (included when not in agent mode).
        if !cfg!(feature = "agent-infra") {
            actions.extend(crate::extensions::extension_actions());
        }
        // Agent-side actions (module only exists with the feature).
        #[cfg(feature = "agent-infra")]
        actions.extend(crate::agent::plugin::agent_extension_actions());
        actions
    }

    #[cfg(feature = "agent-infra")]
    fn primary_action_ids(&self) -> Vec<String> {
        vec!["bootstrap-proxmox-guest".to_string()]
    }

    #[cfg(feature = "agent-infra")]
    async fn handle_service_extension_action(
        &self,
        ctx: &uptrakit_plugin_infrastructure_core::agent_infra::InfraPluginContext<'_>,
        request: &uptrakit_extension_framework::ExtensionRequestPayload,
    ) -> Option<uptrakit_extension_framework::ExtensionResponsePayload> {
        crate::agent::extension_actions::handle_action(ctx, request).await
    }

    #[cfg(feature = "migrations")]
    fn controller_migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        crate::controller_migration::migrations()
    }

    #[cfg(feature = "migrations")]
    fn service_migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        #[cfg(feature = "agent-infra")]
        {
            vec![
                Box::new(crate::agent::migration::CreateProxmoxHostState),
                Box::new(crate::agent::migration::CreateProxmoxPendingMatches),
            ]
        }
        #[cfg(not(feature = "agent-infra"))]
        {
            vec![]
        }
    }

    #[cfg(feature = "agent-infra")]
    fn as_host_lifecycle(
        &self,
    ) -> Option<&dyn uptrakit_plugin_infrastructure_core::HostLifecyclePlugin> {
        Some(self)
    }

    #[cfg(feature = "agent-infra")]
    fn as_host_report(&self) -> Option<&dyn uptrakit_plugin_infrastructure_core::HostReportPlugin> {
        Some(self)
    }

    #[cfg(feature = "agent-infra")]
    fn as_guest_exec(&self) -> Option<&dyn uptrakit_plugin_infrastructure_core::GuestExecPlugin> {
        Some(self)
    }
}

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
            api_token: SecretString::new("root@pam!tok=secret"),
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

    #[test]
    fn agent_plugin_type_is_infrastructure_proxmox() {
        let plugin = ProxmoxPlugin::new_agent();
        assert_eq!(plugin.plugin_type_id(), "infrastructure_proxmox");
    }

    #[test]
    fn agent_capabilities_match_expected() {
        let plugin = ProxmoxPlugin::new_agent();
        assert_eq!(plugin.capabilities(), ProxmoxPlugin::CAPABILITIES);
    }

    #[test]
    fn default_creates_agent_variant() {
        let plugin = ProxmoxPlugin::default();
        assert_eq!(plugin.plugin_type_id(), "infrastructure_proxmox");
        assert!(plugin._config.is_none());
    }
}
