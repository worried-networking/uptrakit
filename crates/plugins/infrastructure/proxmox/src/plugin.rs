use std::collections::BTreeSet;
use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{
    ActionDef, ConfigModel, ExtensionManifest, HostRequirements, HostRuntime, PluginFamily,
    declare_plugin, surfaces,
};
use uptrakit_shared_types::Permission;

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
    pub fn extension_manifests_static() -> Vec<ExtensionManifest> {
        // Controller-side manifests are only relevant when not in agent mode.
        if cfg!(feature = "agent-infra") {
            return vec![];
        }
        crate::extensions::extension_manifests()
    }

    /// Return extension action definitions for the Proxmox VE plugin.
    ///
    /// Separate function used as a function pointer in `declare_plugin!`.
    pub fn extension_actions_static() -> Vec<ActionDef> {
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

    /// Return plugin-backed shared-surface registrations authored natively.
    pub fn surface_registrations_static()
    -> Vec<uptrakit_plugin_infrastructure_core::surfaces::SurfaceRegistration> {
        if cfg!(feature = "agent-infra") {
            return vec![];
        }
        proxmox_surface_registrations()
    }

    /// Return controller-side migrations for the Proxmox VE plugin.
    #[cfg(feature = "migrations")]
    pub fn controller_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        crate::controller_migration::migrations()
    }
}

fn collect_registration_capabilities(
    surfaces: &[surfaces::RegisteredSurface],
) -> surfaces::CapabilitySet {
    let mut caps = BTreeSet::new();
    for surface in surfaces {
        caps.extend(surface.descriptor.required_capabilities.0.iter().cloned());
    }
    surfaces::CapabilitySet(caps)
}

fn proxmox_surface_registrations() -> Vec<surfaces::SurfaceRegistration> {
    let surfaces = vec![
        proxmox_hosts_selector_boundary_surface(),
        proxmox_host_info_surface(),
    ];
    vec![surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: "plugin.infrastructure_proxmox".to_string(),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: collect_registration_capabilities(&surfaces),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces,
        encryption_metadata: None,
    }]
}

fn proxmox_hosts_selector_boundary_surface() -> surfaces::RegisteredSurface {
    let boundary_callout = "The selector-driven Proxmox hosts table still depends on extension \
        context selector/add-action semantics plus row data and is not available in this \
        shared-surface slice. This page currently supports only Add Configuration."
        .to_string();

    surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor {
            surface_id: surfaces::SurfaceId::new("proxmox.hosts")
                .expect("literal surface id is valid"),
            label: "Proxmox VE Hosts".to_string(),
            priority: 650,
            slot: surfaces::SLOT_EXTENSION_PAGE.to_string(),
            scope: surfaces::Scope::Global,
            targeting: surfaces::Targeting::Universal,
            required_permission: Some(Permission::ManageCommands.to_string()),
            provider_kind: surfaces::ProviderKind::Plugin,
            required_capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::SectionNode,
                surfaces::Capability::CalloutNode,
                surfaces::Capability::FormNode,
                surfaces::Capability::FormSubmit,
                surfaces::Capability::SensitiveFields,
                surfaces::Capability::UniversalTargeting,
            ]),
            root_node: surfaces::SurfaceNode::Section {
                title: None,
                children: vec![
                    surfaces::SurfaceNode::Callout {
                        level: surfaces::CalloutLevel::Info,
                        text: boundary_callout,
                    },
                    surfaces::SurfaceNode::Form {
                        interaction_id: surfaces::InteractionId::new("add-config")
                            .expect("literal interaction id is valid"),
                    },
                ],
            },
        },
        interactions: vec![surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("add-config")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::FormSubmit,
                label: Some("Add Configuration".to_string()),
                required_permission: Some(Permission::ManageCommands.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec!["api_token".to_string()],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: Some(surfaces::FormUiDescriptor {
                    fields: vec![
                        surfaces::FormFieldDescriptor {
                            key: "name".to_string(),
                            label: "Configuration Name".to_string(),
                            field_type: "text".to_string(),
                            required: true,
                            placeholder: Some("My Proxmox Cluster".to_string()),
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "api_url".to_string(),
                            label: "Proxmox VE URL".to_string(),
                            field_type: "text".to_string(),
                            required: true,
                            placeholder: Some("https://pve.example.com:8006".to_string()),
                            help_text: Some(
                                "HTTPS URL to your Proxmox VE API (port 8006 by default)."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "api_token".to_string(),
                            label: "API Token".to_string(),
                            field_type: "password".to_string(),
                            required: true,
                            placeholder: Some("user@realm!tokenid=secret".to_string()),
                            help_text: Some(
                                "PVE API token in USER@REALM!TOKENID=SECRET format.".to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: true,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "verify_tls".to_string(),
                            label: "Verify TLS Certificate".to_string(),
                            field_type: "toggle".to_string(),
                            required: false,
                            placeholder: None,
                            help_text: Some(
                                "Disable if your Proxmox VE uses a self-signed certificate."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "node_filter".to_string(),
                            label: "Node Filter".to_string(),
                            field_type: "text".to_string(),
                            required: false,
                            placeholder: Some("pve1,pve2".to_string()),
                            help_text: Some(
                                "Comma-separated list of node names to include. Leave blank for all nodes."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                    ],
                    pre_load_interaction_id: None,
                }),
            }],
        data_sources: vec![],
    }
}

fn proxmox_host_info_surface() -> surfaces::RegisteredSurface {
    let data_source_id = surfaces::DataSourceId::new("proxmox.host-info.primary")
        .expect("literal data source id is valid");
    surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor {
            surface_id: surfaces::SurfaceId::new("proxmox.host-info")
                .expect("literal surface id is valid"),
            label: "Proxmox VE Info".to_string(),
            priority: 100,
            slot: surfaces::SLOT_HOST_DETAIL_TABS.to_string(),
            scope: surfaces::Scope::Global,
            targeting: surfaces::Targeting::Universal,
            required_permission: Some(Permission::UpdateHosts.to_string()),
            provider_kind: surfaces::ProviderKind::Plugin,
            required_capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::KeyValueNode,
                surfaces::Capability::DataLoad,
                surfaces::Capability::ProviderQueryDataSource,
                surfaces::Capability::UniversalTargeting,
            ]),
            root_node: surfaces::SurfaceNode::KeyValue {
                data_source_id: data_source_id.clone(),
            },
        },
        interactions: vec![surfaces::InteractionDescriptor {
            interaction_id: surfaces::InteractionId::new("get-info")
                .expect("literal interaction id is valid"),
            kind: surfaces::InteractionKind::DataLoad,
            label: Some("Get Info".to_string()),
            required_permission: Some(Permission::UpdateHosts.to_string()),
            input_schema: None,
            result_schema: Some(surfaces::SchemaContract::Object),
            sensitive_fields: vec![],
            timeout_seconds: Some(10),
            confirmation: None,
            transport: surfaces::InteractionTransport::ControllerLocal,
            workflow_steps: vec![],
            form_ui: None,
        }],
        data_sources: vec![surfaces::DataSourceDescriptor {
            data_source_id,
            kind: surfaces::DataSourceKind::ProviderQuery {
                operation_id: "get-info".to_string(),
            },
            result_schema: surfaces::SchemaContract::Object,
            pagination: None,
            sorting: None,
            filtering: None,
            refresh_policy: surfaces::RefreshPolicy::Manual,
            empty_state: None,
        }],
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
    use uptrakit_plugin_infrastructure_core::{
        PluginCapability, PluginMeta, SecretString, surfaces,
    };

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
        let host_info = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| surface.descriptor.surface_id.as_str() == "proxmox.host-info")
            .expect("proxmox.host-info surface should be registered");
        assert_eq!(
            host_info.descriptor.required_permission.as_deref(),
            Some("update_hosts"),
            "host-detail surface visibility should be permission-gated"
        );
        let get_info = host_info
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "get-info")
            .expect("host-info data-load interaction should be present");
        assert_eq!(
            get_info.required_permission.as_deref(),
            Some("update_hosts"),
            "data-load interaction should preserve action permission metadata"
        );
    }

    #[test]
    fn controller_surfaces_are_gated_out_in_agent_builds() {
        let registrations = ProxmoxPlugin::surface_registrations_static();
        if cfg!(feature = "agent-infra") {
            assert!(
                registrations.is_empty(),
                "agent-infra builds must not expose controller-local shared surfaces"
            );
        } else {
            assert!(
                !registrations.is_empty(),
                "controller builds should keep shared-surface registrations"
            );
        }
    }

    #[test]
    fn proxmox_host_info_surface_uses_explicit_data_source_contract() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        let host_info = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| surface.descriptor.surface_id.as_str() == "proxmox.host-info")
            .expect("proxmox.host-info surface should be registered");

        assert_eq!(host_info.data_sources.len(), 1);
        let data_source = &host_info.data_sources[0];
        assert_eq!(
            data_source.data_source_id.as_str(),
            "proxmox.host-info.primary"
        );
        assert!(matches!(
            data_source.kind,
            surfaces::DataSourceKind::ProviderQuery { ref operation_id } if operation_id == "get-info"
        ));
        assert_eq!(data_source.result_schema, surfaces::SchemaContract::Object);

        match &host_info.descriptor.root_node {
            surfaces::SurfaceNode::KeyValue { data_source_id } => {
                assert_eq!(data_source_id.as_str(), "proxmox.host-info.primary");
            }
            other => panic!("expected key-value root node, got {other:?}"),
        }
    }

    #[test]
    fn proxmox_hosts_surface_makes_selector_boundary_explicit() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        let hosts = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| surface.descriptor.surface_id.as_str() == "proxmox.hosts")
            .expect("proxmox.hosts surface should be registered");

        assert!(
            hosts.data_sources.is_empty(),
            "selector-driven hosts page should remain non-table on shared surfaces until selector modeling exists"
        );
        assert!(
            hosts
                .interactions
                .iter()
                .all(|interaction| interaction.interaction_id.as_str() != "list"),
            "list data-load remains disabled on the selector-boundary fallback surface"
        );
        assert!(
            hosts.interactions.len() == 1
                && hosts.interactions[0].interaction_id.as_str() == "add-config",
            "selector/row-dependent interactions must not be exposed by the fallback surface"
        );

        match &hosts.descriptor.root_node {
            surfaces::SurfaceNode::Section { children, .. } => {
                assert!(
                    matches!(
                        children.first(),
                        Some(surfaces::SurfaceNode::Callout { text, .. })
                            if text.contains("supports only Add Configuration")
                    ),
                    "fallback surface should explicitly explain why selector-driven table hydration is not available yet"
                );
                assert!(
                    matches!(
                        children.get(1),
                        Some(surfaces::SurfaceNode::Form { interaction_id })
                            if interaction_id.as_str() == "add-config"
                    ),
                    "fallback surface should only expose the runnable add-config form"
                );
            }
            other => {
                panic!("expected section root node for selector-boundary surface, got {other:?}")
            }
        }

        let add_config = hosts
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "add-config")
            .expect("add-config interaction should remain available");
        assert_eq!(add_config.kind, surfaces::InteractionKind::FormSubmit);
        assert_eq!(
            add_config.required_permission.as_deref(),
            Some("manage_commands"),
            "controller-owned add-config flow must stay permission-hardened"
        );
        assert_eq!(
            hosts.descriptor.required_permission.as_deref(),
            Some("manage_commands"),
            "fallback surface visibility must match the only runnable interaction permission"
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
