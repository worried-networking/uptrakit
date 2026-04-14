use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{
    ConfigModel, HostRequirements, HostRuntime, PluginFamily, declare_plugin,
};

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
    pub(crate) _config: Option<ProxmoxConfig>,
}

impl ProxmoxPlugin {
    /// Create a new controller-side Proxmox VE plugin instance (with config).
    ///
    /// The constructor is synchronous — no I/O is performed. HTTP clients
    /// for the Proxmox VE API are created on-demand by the extension action
    /// handlers.
    pub fn new(
        config: ProxmoxConfig,
        _runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        Ok(Self {
            _config: Some(config),
        })
    }

    /// Create a new agent-side Proxmox VE plugin instance (no config).
    pub fn new_agent() -> Self {
        Self { _config: None }
    }

    /// Return extension manifests for the Proxmox VE plugin.
    ///
    /// Separate function used as a function pointer in `declare_plugin!`.
    pub fn extension_manifests_static() -> Vec<uptrakit_extension_framework::ExtensionManifest> {
        // Controller-side manifests are only relevant when not in agent mode.
        if cfg!(feature = "agent-infra") {
            return vec![];
        }
        crate::extensions::extension_manifests()
    }

    /// Return extension action definitions for the Proxmox VE plugin.
    ///
    /// Separate function used as a function pointer in `declare_plugin!`.
    pub fn extension_actions_static() -> Vec<uptrakit_extension_framework::ActionDef> {
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

    /// Return plugin-backed surface registrations derived from extension contracts.
    pub fn surface_registrations_static()
    -> Vec<uptrakit_plugin_infrastructure_core::surfaces::SurfaceRegistration> {
        uptrakit_plugin_infrastructure_core::build_plugin_surface_registrations_from_extensions(
            "infrastructure_proxmox",
            Self::extension_manifests_static(),
            Self::extension_actions_static(),
        )
    }

    /// Return controller-side migrations for the Proxmox VE plugin.
    #[cfg(feature = "migrations")]
    pub fn controller_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        crate::controller_migration::migrations()
    }
}

impl Default for ProxmoxPlugin {
    fn default() -> Self {
        Self::new_agent()
    }
}

// ── declare_plugin! ──────────────────────────────────────────────────────

// Migrations function wrapper — adapts to whatever MigrationsFn type alias is active.
#[cfg(feature = "migrations")]
fn __proxmox_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
    ProxmoxPlugin::controller_migrations()
}
#[cfg(not(feature = "migrations"))]
fn __proxmox_migrations() -> Vec<Box<dyn std::any::Any>> {
    vec![]
}

/// Create the agent-side `InfraBundle` for Proxmox.
///
/// Returns all three narrow trait objects (lifecycle, report, guest_exec) from a
/// single `ProxmoxPlugin` instance, matching the old `PluginBase` downcasting
/// pattern but via explicit bundle fields.
#[cfg(feature = "agent-infra")]
fn __proxmox_create_infra(
    _config: &uptrakit_plugin_infrastructure_core::CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::error::Result<
    uptrakit_plugin_infrastructure_core::InfraBundle,
> {
    let plugin = std::sync::Arc::new(ProxmoxPlugin::new_agent());
    Ok(uptrakit_plugin_infrastructure_core::InfraBundle {
        lifecycle: Some(plugin.clone()),
        report: Some(plugin.clone()),
        guest_exec: Some(plugin),
    })
}

declare_plugin!(ProxmoxPlugin, ProxmoxConfig, "infrastructure_proxmox", {
    display_name: "Proxmox VE",
    family: PluginFamily::Infrastructure,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
    roles: [ReleaseFetcher, UpdateExecutor],
    infra: {
        create: __proxmox_create_infra,
        host_requirements: uptrakit_plugin_infrastructure_core::HostRequirements::new(
            &[uptrakit_shared_types::OsFamily::Linux],
            &[],
            false,
        ),
        capabilities: &[
            uptrakit_shared_types::PluginCapability::HostLifecycle,
            uptrakit_shared_types::PluginCapability::HostReport,
            uptrakit_shared_types::PluginCapability::GuestExec,
        ],
    },
    owned_extension_ids: &["proxmox."],
    extensions: {
        manifests: ProxmoxPlugin::extension_manifests_static,
        actions: ProxmoxPlugin::extension_actions_static,
        handle_action: crate::extensions::handle_action,
    },
    surfaces: {
        registrations: ProxmoxPlugin::surface_registrations_static,
    },
    migrations: __proxmox_migrations,
});

// ── Stub trait implementations for declare_plugin! roles ─────────────────

// The `declare_plugin!` macro asserts that `ProxmoxPlugin` implements
// `ReleaseFetcher` and `UpdateExecutor`. These are controller-side stubs
// that the Proxmox plugin does not actually use for software updates
// (it uses extensions instead). They satisfy the compile-time assertions.

#[async_trait::async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for ProxmoxPlugin {
    async fn fetch_releases(
        &self,
        _package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<
        Vec<uptrakit_plugin_infrastructure_core::UpstreamRelease>,
    > {
        Ok(vec![])
    }
}

#[async_trait::async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for ProxmoxPlugin {
    async fn execute_update(
        &self,
        _package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&uptrakit_plugin_infrastructure_core::ReleaseInfo>,
        _output_tx: &uptrakit_plugin_infrastructure_core::UpdateOutputSender,
    ) -> uptrakit_plugin_infrastructure_core::Result<String> {
        Err(rootcause::report!(
            uptrakit_plugin_infrastructure_core::PluginError::UnsupportedOperation(
                "Proxmox VE plugin does not execute software updates directly".to_string()
            )
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta, SecretString};

    #[test]
    fn plugin_type_is_infrastructure_proxmox() {
        let config = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=secret"),
            ..ProxmoxConfig::default()
        };
        let plugin = ProxmoxPlugin::new(config, test_runtime()).expect("create");
        assert_eq!(plugin.plugin_type_id().as_str(), "infrastructure_proxmox");
    }

    #[test]
    fn agent_plugin_type_is_infrastructure_proxmox() {
        let plugin = ProxmoxPlugin::new_agent();
        assert_eq!(plugin.plugin_type_id().as_str(), "infrastructure_proxmox");
    }

    #[test]
    fn default_creates_agent_variant() {
        let plugin = ProxmoxPlugin::default();
        assert_eq!(plugin.plugin_type_id().as_str(), "infrastructure_proxmox");
        assert!(plugin._config.is_none());
    }

    // ── descriptor capabilities ─────────────────────────────────────────

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ReleaseFetching)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::UpdateExecution)
        );
    }

    // ── descriptor roles ────────────────────────────────────────────────

    #[test]
    fn descriptor_has_expected_roles() {
        assert!(DESCRIPTOR.roles.release_fetcher.is_some());
        assert!(DESCRIPTOR.roles.update_executor.is_some());
        assert!(DESCRIPTOR.roles.discoverer.is_none());
        assert!(DESCRIPTOR.roles.version_detector.is_none());
        assert!(DESCRIPTOR.roles.package_indexer.is_none());
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
    }

    // ── descriptor extensions ───────────────────────────────────────────

    #[test]
    fn descriptor_has_extensions() {
        assert!(DESCRIPTOR.extensions.is_some());
        let ext = DESCRIPTOR.extensions.unwrap();
        assert!(!ext.owned_ids.is_empty());
        assert_eq!(ext.owned_ids[0], "proxmox.");
    }

    #[test]
    fn descriptor_has_plugin_surface_registrations() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        assert!(
            !registrations.is_empty(),
            "proxmox should contribute at least one shared-surface registration"
        );
        assert!(registrations.iter().all(|registration| {
            registration.provider.provider_kind
                == uptrakit_plugin_infrastructure_core::surfaces::ProviderKind::Plugin
        }));
        let all_surface_ids: Vec<String> = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .map(|surface| surface.descriptor.surface_id.to_string())
            .collect();
        assert!(
            all_surface_ids.iter().any(|id| id == "proxmox.hosts"),
            "page-level proxmox.hosts should remain represented in shared surfaces"
        );
        assert!(
            all_surface_ids.iter().any(|id| id == "proxmox.host-info"),
            "host-detail key-value panel should be represented in shared surfaces"
        );
    }

    // ── descriptor migrations ───────────────────────────────────────────

    #[cfg(feature = "migrations")]
    #[test]
    fn descriptor_has_migrations() {
        assert!(DESCRIPTOR.migrations.is_some());
        let migrations = (DESCRIPTOR.migrations.unwrap())();
        assert!(!migrations.is_empty());
    }

    /// Helper to create a test runtime.
    fn test_runtime() -> Arc<dyn HostRuntime> {
        use uptrakit_plugin_infrastructure_core::{
            HostCapabilities, LocalCommandExecutor, StandardHostRuntime,
        };
        let executor = Arc::new(LocalCommandExecutor)
            as Arc<dyn uptrakit_plugin_infrastructure_core::command::CommandExecutor>;
        let caps = HostCapabilities::default();
        Arc::new(StandardHostRuntime::new(executor, caps))
    }
}
