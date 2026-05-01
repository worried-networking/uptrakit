use std::collections::BTreeSet;
use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{
    ConfigModel, HostRequirements, HostRuntime, PluginFamily, SurfaceActionDescriptor,
    declare_plugin, surfaces,
};
use uptrakit_shared_types::Permission;

use crate::config::ProxmoxConfig;
use crate::update_protection::{DEFAULT_BACKUP_TIMEOUT_SECONDS, DEFAULT_SNAPSHOT_TIMEOUT_SECONDS};

/// Proxmox VE infrastructure plugin.
///
/// Unified plugin struct used on both the controller side (with config) and
/// the agent side (without config). On the controller it communicates with
/// the Proxmox VE REST API to discover VMs/CTs and match them to
/// Uptrakit-managed hosts. On the agent it implements infrastructure subtraits
/// for host lifecycle, host reporting, and guest execution.
///
/// All user interaction goes through the shared-surface framework (pages and panels).
pub struct ProxmoxPlugin {
    pub(crate) _config: Option<ProxmoxConfig>,
}

impl ProxmoxPlugin {
    /// Create a new controller-side Proxmox VE plugin instance (with config).
    ///
    /// The constructor is synchronous — no I/O is performed. HTTP clients
    /// for the Proxmox VE API are created on-demand by the surface action
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

    /// Return surface action definitions for the Proxmox VE plugin.
    ///
    /// Separate function used as a function pointer in `declare_plugin!`.
    pub fn surface_actions_static() -> Vec<SurfaceActionDescriptor> {
        let mut actions = Vec::new();
        // Controller-side actions (included when not in agent mode).
        if !cfg!(feature = "agent-infra") {
            actions.extend(crate::surfaces::surface_actions());
        }
        // Agent-side actions (module only exists with the feature).
        #[cfg(feature = "agent-infra")]
        actions.extend(crate::agent::plugin::agent_surface_actions());
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

fn descriptor_surface_registrations()
-> Vec<uptrakit_plugin_infrastructure_core::surfaces::SurfaceRegistration> {
    proxmox_surface_registrations()
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
        proxmox_hosts_surface(),
        proxmox_host_info_surface(),
        proxmox_settings_update_protection_surface(),
        proxmox_software_item_update_protection_surface(),
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

fn proxmox_hosts_surface() -> surfaces::RegisteredSurface {
    let data_source_id = surfaces::DataSourceId::new("proxmox.hosts.mappings")
        .expect("literal data source id is valid");

    surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor::builder()
            .surface_id(
                surfaces::SurfaceId::new("proxmox.hosts").expect("literal surface id is valid"),
            )
            .label("Proxmox VE Hosts")
            .priority(650)
            .slot(surfaces::SLOT_SURFACE_PAGE)
            .scope(surfaces::Scope::Global)
            .targeting(surfaces::Targeting::Universal)
            .required_permission(Permission::UpdateHosts.to_string())
            .provider_kind(surfaces::ProviderKind::Plugin)
            .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::SectionNode,
                surfaces::Capability::ActionBarNode,
                surfaces::Capability::TableNode,
                surfaces::Capability::DataLoad,
                surfaces::Capability::FormSubmit,
                surfaces::Capability::MutationAction,
                surfaces::Capability::ConfirmableAction,
                surfaces::Capability::ProviderQueryDataSource,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::ContextSelector,
                surfaces::Capability::EntityLinkColumn,
            ]))
            .root_node(surfaces::SurfaceNode::Section {
                title: None,
                children: vec![
                    surfaces::SurfaceNode::ActionBar {
                        action_ids: vec![
                            surfaces::InteractionId::new("discover").expect("literal"),
                            surfaces::InteractionId::new("test-connection").expect("literal"),
                        ],
                    },
                    surfaces::SurfaceNode::Table {
                        data_source_id: data_source_id.clone(),
                        columns: vec![
                            surfaces::SurfaceTableColumn::new("proxmox_name", "Name"),
                            surfaces::SurfaceTableColumn::new("config_name", "Configuration"),
                            surfaces::SurfaceTableColumn::new("proxmox_node", "Node"),
                            surfaces::SurfaceTableColumn::new("proxmox_vmid", "VMID"),
                            surfaces::SurfaceTableColumn::new("proxmox_type", "Type"),
                            surfaces::SurfaceTableColumn::new("proxmox_status", "Status"),
                            surfaces::SurfaceTableColumn::new("hostname", "Hostname"),
                            {
                                let mut col = surfaces::SurfaceTableColumn::new(
                                    "matched_host",
                                    "Matched Host",
                                );
                                col.cell_type = Some(surfaces::SurfaceTableCellType::EntityLink {
                                    entity_type: surfaces::SurfaceEntityType::Host,
                                });
                                col
                            },
                            surfaces::SurfaceTableColumn::new("suggested_host", "Suggested Match"),
                        ],
                        row_actions: vec![
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("approve-match")
                                    .expect("literal"),
                                visible_when: Some(surfaces::SurfaceRowVisibleWhen {
                                    field: "suggested_host_id".to_string(),
                                    condition: surfaces::SurfaceRowCondition::Present,
                                }),
                            },
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("match")
                                    .expect("literal"),
                                visible_when: None,
                            },
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("unmatch")
                                    .expect("literal"),
                                visible_when: Some(surfaces::SurfaceRowVisibleWhen {
                                    field: "matched_host".to_string(),
                                    condition: surfaces::SurfaceRowCondition::Present,
                                }),
                            },
                        ],
                    },
                ],
            })
            .context_selector(surfaces::SurfaceContextSelectorDescriptor::new(
                "plugin_config_id",
                "Configuration",
                "All Configurations",
                "/api/v1/plugin-configs?plugin_type=infrastructure_proxmox",
                "id",
                "name",
                vec![
                    surfaces::InteractionId::new("discover").expect("literal"),
                    surfaces::InteractionId::new("test-connection").expect("literal"),
                ],
            ))
            .build(),
        interactions: vec![
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("list").expect("literal"),
                kind: surfaces::InteractionKind::DataLoad,
                label: "List Hosts".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: None,
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("discover").expect("literal"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Discover".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(120),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("test-connection").expect("literal"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Test Connection".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(30),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("approve-match").expect("literal"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Approve Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("match").expect("literal"),
                kind: surfaces::InteractionKind::FormSubmit,
                label: "Manual Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: Some(surfaces::FormUiDescriptor {
                    fields: vec![
                        surfaces::FormFieldDescriptor {
                            key: "mapping_id".to_string(),
                            label: "Mapping ID".to_string(),
                            field_type: "hidden".to_string(),
                            required: true,
                            placeholder: None,
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "host_id".to_string(),
                            label: "Host".to_string(),
                            field_type: "select".to_string(),
                            required: true,
                            placeholder: Some("Select a host".to_string()),
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: Some(surfaces::FormSelectSource::RestApi {
                                path: "/api/v1/hosts".to_string(),
                                value_field: "id".to_string(),
                                label_field: "friendly_name".to_string(),
                            }),
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                    ],
                    pre_load_interaction_id: None,
                }),
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("unmatch").expect("literal"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Remove Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: Some(surfaces::InteractionConfirmation {
                    title: "Remove Match".to_string(),
                    message: "Remove the host mapping for".to_string(),
                    confirm_label: Some("Remove".to_string()),
                    cancel_label: None,
                    severity: surfaces::ConfirmationSeverity::Danger,
                }),
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
        ],
        data_sources: vec![surfaces::DataSourceDescriptor {
            data_source_id,
            kind: surfaces::DataSourceKind::ProviderQuery {
                operation_id: "list".to_string(),
            },
            result_schema: surfaces::SchemaContract::Any,
            pagination: Some(surfaces::DataSourcePagination {
                default_page_size: 50,
                max_page_size: 200,
            }),
            sorting: None,
            filtering: None,
            refresh_policy: surfaces::RefreshPolicy::Manual,
            empty_state: Some(surfaces::DataSourceEmptyState {
                title: "No Proxmox guests found".to_string(),
                description: Some(
                    "Run Discover on a configuration to populate this table.".to_string(),
                ),
            }),
        }],
    }
}

fn proxmox_host_info_surface() -> surfaces::RegisteredSurface {
    let data_source_id = surfaces::DataSourceId::new("proxmox.host-info.primary")
        .expect("literal data source id is valid");
    surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor::builder()
            .surface_id(
                surfaces::SurfaceId::new("proxmox.host-info").expect("literal surface id is valid"),
            )
            .label("Proxmox VE Info")
            .priority(100)
            .slot(surfaces::SLOT_HOST_DETAIL_TABS)
            .scope(surfaces::Scope::Global)
            .targeting(surfaces::Targeting::Universal)
            .required_permission(Permission::UpdateHosts.to_string())
            .provider_kind(surfaces::ProviderKind::Plugin)
            .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::KeyValueNode,
                surfaces::Capability::DataLoad,
                surfaces::Capability::ProviderQueryDataSource,
                surfaces::Capability::UniversalTargeting,
            ]))
            .root_node(surfaces::SurfaceNode::KeyValue {
                data_source_id: data_source_id.clone(),
            })
            .build(),
        interactions: vec![surfaces::InteractionDescriptor {
            interaction_id: surfaces::InteractionId::new("get-info")
                .expect("literal interaction id is valid"),
            kind: surfaces::InteractionKind::DataLoad,
            label: "Get Info".to_string(),
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

fn proxmox_settings_update_protection_surface() -> surfaces::RegisteredSurface {
    let callout = "Backup targets in this form come from Proxmox discovery cache. \
        If the dropdown is empty, run Discover on the Proxmox VE Hosts page first."
        .to_string();

    surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor::builder()
            .surface_id(
                surfaces::SurfaceId::new("proxmox.settings.update-protection")
                    .expect("literal surface id is valid"),
            )
            .label("Proxmox Update Protection")
            .priority(720)
            .slot(surfaces::SLOT_SETTINGS_TABS)
            .scope(surfaces::Scope::Global)
            .targeting(surfaces::Targeting::Universal)
            .required_permission(Permission::ManageGlobalSettings.to_string())
            .provider_kind(surfaces::ProviderKind::Plugin)
            .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::SectionNode,
                surfaces::Capability::CalloutNode,
                surfaces::Capability::FormNode,
                surfaces::Capability::DataLoad,
                surfaces::Capability::MutationAction,
                surfaces::Capability::UniversalTargeting,
            ]))
            .root_node(surfaces::SurfaceNode::Section {
                title: None,
                children: vec![
                    surfaces::SurfaceNode::Callout {
                        level: surfaces::CalloutLevel::Info,
                        text: callout,
                    },
                    surfaces::SurfaceNode::Form {
                        interaction_id: surfaces::InteractionId::new("save-global-defaults")
                            .expect("literal interaction id is valid"),
                    },
                ],
            })
            .build(),
        interactions: vec![
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("preload-global-defaults")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::DataLoad,
                label: "Preload Global Defaults".to_string(),
                required_permission: Some(Permission::ManageGlobalSettings.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Object),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("load-backup-target-options")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::DataLoad,
                label: "Load Backup Target Options".to_string(),
                required_permission: Some(Permission::ManageGlobalSettings.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Object),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("save-global-defaults")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Save Global Defaults".to_string(),
                required_permission: Some(Permission::ManageGlobalSettings.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: Some(surfaces::FormUiDescriptor {
                    fields: vec![
                        surfaces::FormFieldDescriptor {
                            key: "plugin_config_id".to_string(),
                            label: "Proxmox Configuration".to_string(),
                            field_type: "select".to_string(),
                            required: true,
                            placeholder: None,
                            help_text: Some(
                                "Select the Proxmox plugin configuration this default applies to."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: Some(surfaces::FormSelectSource::RestApi {
                                path: "/api/v1/plugin-configs?plugin_type=infrastructure_proxmox".to_string(),
                                value_field: "id".to_string(),
                                label_field: "name".to_string(),
                            }),
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "mode".to_string(),
                            label: "Default Protection Mode".to_string(),
                            field_type: "select".to_string(),
                            required: true,
                            placeholder: None,
                            help_text: None,
                            default_value: Some("do_nothing".to_string()),
                            options: vec![
                                surfaces::FormSelectOption {
                                    value: "do_nothing".to_string(),
                                    label: "Do Nothing".to_string(),
                                },
                                surfaces::FormSelectOption {
                                    value: "snapshot".to_string(),
                                    label: "Snapshot".to_string(),
                                },
                                surfaces::FormSelectOption {
                                    value: "backup".to_string(),
                                    label: "Backup".to_string(),
                                },
                            ],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "backup_target_option".to_string(),
                            label: "Backup Target".to_string(),
                            field_type: "select".to_string(),
                            required: false,
                            placeholder: None,
                            help_text: Some(
                                "Loaded from Proxmox cache. If empty, run Discover on Proxmox VE Hosts."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: Some(surfaces::FormSelectSource::Action {
                                action_id: "load-backup-target-options".to_string(),
                            }),
                            sensitive: false,
                            list: false,
                            visible_when: Some(surfaces::FormVisibleWhen {
                                field: "mode".to_string(),
                                values: vec!["backup".to_string()],
                            }),
                        },
                        surfaces::FormFieldDescriptor {
                            key: "snapshot_timeout_seconds".to_string(),
                            label: "Snapshot timeout".to_string(),
                            field_type: "number".to_string(),
                            required: false,
                            placeholder: Some(DEFAULT_SNAPSHOT_TIMEOUT_SECONDS.to_string()),
                            help_text: Some(format!(
                                "Leave empty to use the built-in snapshot timeout of {DEFAULT_SNAPSHOT_TIMEOUT_SECONDS} seconds."
                            )),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: Some(surfaces::FormVisibleWhen {
                                field: "mode".to_string(),
                                values: vec!["snapshot".to_string()],
                            }),
                        },
                        surfaces::FormFieldDescriptor {
                            key: "backup_timeout_seconds".to_string(),
                            label: "Backup timeout".to_string(),
                            field_type: "number".to_string(),
                            required: false,
                            placeholder: Some(DEFAULT_BACKUP_TIMEOUT_SECONDS.to_string()),
                            help_text: Some(format!(
                                "Leave empty to use the built-in backup timeout of {DEFAULT_BACKUP_TIMEOUT_SECONDS} seconds."
                            )),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: Some(surfaces::FormVisibleWhen {
                                field: "mode".to_string(),
                                values: vec!["backup".to_string()],
                            }),
                        },
                    ],
                    pre_load_interaction_id: Some(
                        surfaces::InteractionId::new("preload-global-defaults")
                            .expect("literal interaction id is valid"),
                    ),
                }),
            },
        ],
        data_sources: vec![],
    }
}

fn proxmox_software_item_update_protection_surface() -> surfaces::RegisteredSurface {
    let callout = "Per-item override values are stored in Proxmox policy tables. \
        Backup target options come from sync cache and stay empty until discover/sync populates them."
        .to_string();

    surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor::builder()
            .surface_id(
                surfaces::SurfaceId::new("proxmox.software-item.update-protection")
                    .expect("literal surface id is valid"),
            )
            .label("Proxmox Update Protection")
            .priority(520)
            .slot(surfaces::SLOT_SOFTWARE_ITEM_TABS)
            .scope(surfaces::Scope::Global)
            .targeting(surfaces::Targeting::Universal)
            .required_permission(Permission::ViewSoftware.to_string())
            .provider_kind(surfaces::ProviderKind::Plugin)
            .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::SectionNode,
                surfaces::Capability::CalloutNode,
                surfaces::Capability::FormNode,
                surfaces::Capability::DataLoad,
                surfaces::Capability::MutationAction,
                surfaces::Capability::UniversalTargeting,
            ]))
            .root_node(surfaces::SurfaceNode::Section {
                title: None,
                children: vec![
                    surfaces::SurfaceNode::Callout {
                        level: surfaces::CalloutLevel::Info,
                        text: callout,
                    },
                    surfaces::SurfaceNode::Form {
                        interaction_id: surfaces::InteractionId::new("save-item-overrides")
                            .expect("literal interaction id is valid"),
                    },
                ],
            })
            .build(),
        interactions: vec![
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("preload-item-overrides")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::DataLoad,
                label: "Preload Per-item Overrides".to_string(),
                required_permission: Some(Permission::ViewSoftware.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Object),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("load-backup-target-options")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::DataLoad,
                label: "Load Backup Target Options".to_string(),
                required_permission: Some(Permission::ViewSoftware.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Object),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("save-item-overrides")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Save Per-item Overrides".to_string(),
                required_permission: Some(Permission::UpdateSoftware.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: Some(surfaces::FormUiDescriptor {
                    fields: vec![
                        surfaces::FormFieldDescriptor {
                            key: "plugin_config_id".to_string(),
                            label: "Proxmox Configuration".to_string(),
                            field_type: "select".to_string(),
                            required: true,
                            placeholder: None,
                            help_text: Some(
                                "Select the Proxmox plugin configuration this override applies to."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: Some(surfaces::FormSelectSource::RestApi {
                                path: "/api/v1/plugin-configs?plugin_type=infrastructure_proxmox".to_string(),
                                value_field: "id".to_string(),
                                label_field: "name".to_string(),
                            }),
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "mode".to_string(),
                            label: "Override Mode".to_string(),
                            field_type: "select".to_string(),
                            required: true,
                            placeholder: None,
                            help_text: Some(
                                "Choose 'Inherit Global Default' to remove this software-item override."
                                    .to_string(),
                            ),
                            default_value: Some("inherit_global".to_string()),
                            options: vec![
                                surfaces::FormSelectOption {
                                    value: "inherit_global".to_string(),
                                    label: "Inherit Global Default".to_string(),
                                },
                                surfaces::FormSelectOption {
                                    value: "do_nothing".to_string(),
                                    label: "Do Nothing".to_string(),
                                },
                                surfaces::FormSelectOption {
                                    value: "snapshot".to_string(),
                                    label: "Snapshot".to_string(),
                                },
                                surfaces::FormSelectOption {
                                    value: "backup".to_string(),
                                    label: "Backup".to_string(),
                                },
                            ],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "backup_target_option".to_string(),
                            label: "Backup Target".to_string(),
                            field_type: "select".to_string(),
                            required: false,
                            placeholder: None,
                            help_text: Some(
                                "Loaded from Proxmox cache. If empty, run Discover on Proxmox VE Hosts."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: Some(surfaces::FormSelectSource::Action {
                                action_id: "load-backup-target-options".to_string(),
                            }),
                            sensitive: false,
                            list: false,
                            visible_when: Some(surfaces::FormVisibleWhen {
                                field: "mode".to_string(),
                                values: vec!["backup".to_string()],
                            }),
                        },
                        surfaces::FormFieldDescriptor {
                            key: "snapshot_timeout_seconds".to_string(),
                            label: "Snapshot timeout".to_string(),
                            field_type: "number".to_string(),
                            required: false,
                            placeholder: Some("120".to_string()),
                            help_text: Some(
                                "Leave empty to use the system-wide snapshot timeout for this mode."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: Some(surfaces::FormVisibleWhen {
                                field: "mode".to_string(),
                                values: vec!["snapshot".to_string()],
                            }),
                        },
                        surfaces::FormFieldDescriptor {
                            key: "backup_timeout_seconds".to_string(),
                            label: "Backup timeout".to_string(),
                            field_type: "number".to_string(),
                            required: false,
                            placeholder: Some("900".to_string()),
                            help_text: Some(
                                "Leave empty to use the system-wide backup timeout for this mode."
                                    .to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: Some(surfaces::FormVisibleWhen {
                                field: "mode".to_string(),
                                values: vec!["backup".to_string()],
                            }),
                        },
                    ],
                    pre_load_interaction_id: Some(
                        surfaces::InteractionId::new("preload-item-overrides")
                            .expect("literal interaction id is valid"),
                    ),
                }),
            },
        ],
        data_sources: vec![],
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

fn __proxmox_create_controller_update_protection(
    config: &uptrakit_plugin_infrastructure_core::CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::error::Result<
    std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::ControllerUpdateProtection>,
> {
    crate::update_protection::ControllerUpdateProtectionPlugin::create(config)
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
    controller_update_protection: __proxmox_create_controller_update_protection,
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
    owned_surface_ids: &["proxmox."],
    surface_actions: {
        actions: ProxmoxPlugin::surface_actions_static,
        handle_action: crate::surfaces::handle_surface_action,
    },
    surfaces: {
        registrations: descriptor_surface_registrations,
    },
    migrations: __proxmox_migrations,
});

// ── Stub trait implementations for declare_plugin! roles ─────────────────

// The `declare_plugin!` macro asserts that `ProxmoxPlugin` implements
// `ReleaseFetcher` and `UpdateExecutor`. These are controller-side stubs
// that the Proxmox plugin does not actually use for software updates
// (it uses surfaces instead). They satisfy the compile-time assertions.

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
    ) -> uptrakit_plugin_infrastructure_core::Result<
        uptrakit_plugin_infrastructure_core::ExecuteUpdateResult,
    > {
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
        assert!(DESCRIPTOR.roles.controller_update_protection.is_some());
    }

    // ── descriptor surfaces ─────────────────────────────────────────────

    #[test]
    fn descriptor_has_surfaces() {
        assert!(DESCRIPTOR.surface_actions.is_some());
        let surface_actions = DESCRIPTOR.surface_actions.unwrap();
        assert!(!surface_actions.owned_surface_ids().is_empty());
        assert_eq!(surface_actions.owned_surface_ids()[0], "proxmox.");
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
        assert!(
            all_surface_ids
                .iter()
                .any(|id| id == "proxmox.settings.update-protection"),
            "settings.tab Proxmox update-protection surface should be represented in shared surfaces"
        );
        assert!(
            all_surface_ids
                .iter()
                .any(|id| id == "proxmox.software-item.update-protection"),
            "software_item.tabs Proxmox update-protection surface should be represented in shared surfaces"
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

        let settings_policy = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| {
                surface.descriptor.surface_id.as_str() == "proxmox.settings.update-protection"
            })
            .expect("settings update-protection surface should be registered");
        assert_eq!(
            settings_policy.descriptor.slot,
            surfaces::SLOT_SETTINGS_TABS,
            "settings policy surface should render in settings.tabs"
        );
        assert_eq!(
            settings_policy.descriptor.required_permission.as_deref(),
            Some("manage_global_settings")
        );

        let software_policy = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| {
                surface.descriptor.surface_id.as_str() == "proxmox.software-item.update-protection"
            })
            .expect("software-item update-protection surface should be registered");
        assert_eq!(
            software_policy.descriptor.slot,
            surfaces::SLOT_SOFTWARE_ITEM_TABS,
            "software-item policy surface should render in software_item.tabs"
        );
        assert_eq!(
            software_policy.descriptor.required_permission.as_deref(),
            Some("view_software")
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
    fn proxmox_hosts_surface_has_full_table_layout() {
        let registrations = proxmox_surface_registrations();
        let reg = &registrations[0];
        let hosts = reg
            .surfaces
            .iter()
            .find(|s| s.descriptor.surface_id.as_str() == "proxmox.hosts")
            .expect("proxmox.hosts surface must be registered");

        // context_selector present
        let selector = hosts
            .descriptor
            .context_selector
            .as_ref()
            .expect("proxmox.hosts must declare a context_selector");
        assert_eq!(selector.param_key, "plugin_config_id");
        assert!(
            selector
                .required_for_interactions
                .iter()
                .any(|id| id.as_str() == "discover")
        );
        assert!(
            selector
                .required_for_interactions
                .iter()
                .any(|id| id.as_str() == "test-connection")
        );

        // root is section with action bar + table
        let children = match &hosts.descriptor.root_node {
            surfaces::SurfaceNode::Section { children, .. } => children,
            other => panic!("expected section root, got {other:?}"),
        };
        assert!(
            children
                .iter()
                .any(|n| matches!(n, surfaces::SurfaceNode::ActionBar { .. })),
            "root section must contain an ActionBar"
        );
        let row_actions = children
            .iter()
            .find_map(|n| match n {
                surfaces::SurfaceNode::Table { row_actions, .. } => Some(row_actions),
                _ => None,
            })
            .expect("root section must contain a Table node");

        let action_ids: Vec<&str> = row_actions
            .iter()
            .map(|ra| ra.interaction_id.as_str())
            .collect();
        assert!(action_ids.contains(&"approve-match"));
        assert!(action_ids.contains(&"match"));
        assert!(action_ids.contains(&"unmatch"));

        let unmatch = hosts
            .interactions
            .iter()
            .find(|i| i.interaction_id.as_str() == "unmatch")
            .expect("unmatch interaction must be declared");
        assert!(matches!(
            unmatch.confirmation.as_ref().map(|c| &c.severity),
            Some(surfaces::ConfirmationSeverity::Danger)
        ));

        assert!(
            hosts
                .interactions
                .iter()
                .any(|i| i.interaction_id.as_str() == "list"),
            "list interaction must be declared"
        );

        assert_eq!(hosts.data_sources.len(), 1);
        assert!(
            hosts.data_sources[0].pagination.is_some(),
            "data source must have pagination"
        );

        assert!(
            hosts
                .descriptor
                .required_capabilities
                .0
                .contains(&surfaces::Capability::ContextSelector),
            "must declare ContextSelector capability"
        );
    }

    #[test]
    fn policy_surfaces_keep_preload_and_backup_options_contract() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        let settings_policy = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| {
                surface.descriptor.surface_id.as_str() == "proxmox.settings.update-protection"
            })
            .expect("settings update-protection surface should be present");

        let save_global = settings_policy
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "save-global-defaults")
            .expect("save-global-defaults interaction should exist");
        assert_eq!(
            save_global
                .form_ui
                .as_ref()
                .and_then(|form_ui| form_ui.pre_load_interaction_id.as_ref())
                .map(|id| id.as_str()),
            Some("preload-global-defaults")
        );
        let backup_field = save_global
            .form_ui
            .as_ref()
            .expect("save-global-defaults should expose a form")
            .fields
            .iter()
            .find(|field| field.key == "backup_target_option")
            .expect("backup target field should exist");
        assert!(matches!(
            backup_field.select_source,
            Some(surfaces::FormSelectSource::Action { ref action_id })
                if action_id == "load-backup-target-options"
        ));

        let software_policy = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| {
                surface.descriptor.surface_id.as_str() == "proxmox.software-item.update-protection"
            })
            .expect("software-item update-protection surface should be present");
        let save_item = software_policy
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "save-item-overrides")
            .expect("save-item-overrides interaction should exist");
        assert_eq!(
            save_item.required_permission.as_deref(),
            Some("update_software")
        );
        assert_eq!(
            save_item
                .form_ui
                .as_ref()
                .and_then(|form_ui| form_ui.pre_load_interaction_id.as_ref())
                .map(|id| id.as_str()),
            Some("preload-item-overrides")
        );

        let fields = &save_global
            .form_ui
            .as_ref()
            .expect("save-global-defaults should expose a form")
            .fields;

        let snapshot_timeout = fields
            .iter()
            .find(|field| field.key == "snapshot_timeout_seconds")
            .expect("snapshot timeout field should exist");
        assert_eq!(snapshot_timeout.field_type, "number");
        assert_eq!(
            snapshot_timeout
                .visible_when
                .as_ref()
                .map(|rule| rule.field.as_str()),
            Some("mode")
        );
        assert_eq!(
            snapshot_timeout
                .visible_when
                .as_ref()
                .map(|rule| rule.values.as_slice()),
            Some(["snapshot".to_string()].as_slice())
        );

        let backup_timeout = fields
            .iter()
            .find(|field| field.key == "backup_timeout_seconds")
            .expect("backup timeout field should exist");
        assert_eq!(backup_timeout.field_type, "number");
        assert_eq!(
            backup_timeout
                .visible_when
                .as_ref()
                .map(|rule| rule.field.as_str()),
            Some("mode")
        );
        assert_eq!(
            backup_timeout
                .visible_when
                .as_ref()
                .map(|rule| rule.values.as_slice()),
            Some(["backup".to_string()].as_slice())
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
