//! SSH agent UI surface: host management via the shared surface runtime.
//!
//! Provides:
//! - Direct `ssh-agent.hosts` surface registration for the shared runtime
//! - Action library with `list-hosts`, `bootstrap`, `sync-host`, `remove-host` definitions
//! - Action handlers for each action
//! - ECIES decryption of sensitive parameters (auth password, private key)

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::LazyLock;

use uptrakit_internal_wire::{
    AuditEventPayload, ServiceMessage, ServiceTransport,
    surfaces::{
        self, CapabilitySet, DataSourceDescriptor, DataSourceId, DataSourceKind,
        FormFieldDescriptor, FormSelectOption, FormUiDescriptor, FrameworkGeneration,
        InteractionDescriptor, InteractionId, InteractionKind, InteractionTransport,
        ProviderEncryptionAlgorithm, ProviderEncryptionMetadata, RefreshPolicy, SurfaceActionError,
        SurfaceActionErrorCode, SurfaceActionRequest, SurfaceActionResponse, SurfaceDescriptor,
        SurfaceNode, SurfaceRegistration, SurfaceTableColumn, SurfaceTableRowAction, Targeting,
    },
};
use uptrakit_plugin_infrastructure_registry::agent_infra::{
    InfraActionInvokeError, InfraActionInvoker, InfraPluginContext,
};
use uptrakit_plugin_infrastructure_registry::{
    FormFieldDescriptor as PluginFormFieldDescriptor, FormFieldType as PluginFormFieldType,
    FormSelectOptionDescriptor as PluginFormSelectOptionDescriptor,
    FormSelectSourceDescriptor as PluginFormSelectSourceDescriptor, InfraBundle, PluginFamily,
    SurfaceActionDescriptor, SurfaceActionUi, SurfaceFormDescriptor as PluginSurfaceFormDescriptor,
    SurfaceRowCondition as PluginSurfaceRowCondition,
    SurfaceRowVisibleWhen as PluginSurfaceRowVisibleWhen,
    SurfaceWorkflowStep as PluginSurfaceWorkflowStep,
};
use uptrakit_shared_types::{Permission, SecretString};

use crate::host_ops;
use crate::operations::bootstrap::{self, BootstrapParams};
use crate::operations::bootstrap_proxmox::AgentGuestBootstrapExecutor;
use crate::operations::sync;
use crate::ssh_target::SshTarget;

/// Surface ID for SSH host management.
pub const SSH_HOSTS_SURFACE_ID: &str = "ssh-agent.hosts";

const SSH_HOSTS_SURFACE_LABEL: &str = "SSH Hosts";
const SSH_HOSTS_SURFACE_PRIORITY: i32 = 450;
const SSH_HOSTS_DATA_ACTION_ID: &str = "list-hosts";
const SSH_HOSTS_DEFAULT_PER_PAGE: u32 = 50;
const SSH_HOSTS_PRIMARY_ACTION_ID: &str = "bootstrap";
const SSH_HOSTS_ROW_ACTION_IDS: [&str; 2] = ["sync-host", "remove-host"];
const SSH_HOSTS_COLUMNS: [(&str, &str); 5] = [
    ("id", "ID"),
    ("name", "Name"),
    ("hostname", "Hostname"),
    ("port", "Port"),
    ("username", "Username"),
];

static REGISTERED_INTERACTION_IDS: LazyLock<BTreeSet<String>> =
    LazyLock::new(collect_registered_interaction_ids);

fn collect_infra_primary_actions() -> Vec<String> {
    use uptrakit_plugin_infrastructure_registry::all_descriptors;

    all_descriptors()
        .iter()
        .filter(|descriptor| descriptor.family == PluginFamily::Infrastructure)
        .filter_map(|descriptor| descriptor.surface_actions)
        .flat_map(|surface_actions| (surface_actions.actions)())
        .filter(|action| action.action_id == "bootstrap-proxmox-guest")
        .map(|action| action.action_id)
        .collect()
}

fn build_primary_actions(infra_primary_actions: &[String]) -> Vec<String> {
    let mut primary_actions = vec![SSH_HOSTS_PRIMARY_ACTION_ID.to_string()];
    primary_actions.extend(infra_primary_actions.iter().cloned());
    primary_actions
}

fn collect_registered_interaction_ids() -> BTreeSet<String> {
    let infra_primary_actions = collect_infra_primary_actions();
    let actions = build_actions();
    let action_index: BTreeMap<&str, &SurfaceActionDescriptor> = actions
        .iter()
        .map(|action| (action.action_id.as_str(), action))
        .collect();
    let Some(surface) = build_registered_surface(&action_index, &infra_primary_actions) else {
        return BTreeSet::new();
    };

    surface
        .interactions
        .into_iter()
        .map(|interaction| interaction.interaction_id.as_str().to_string())
        .collect()
}

fn is_registered_interaction(action_id: &str) -> bool {
    REGISTERED_INTERACTION_IDS.contains(action_id)
}

/// Build the service surface registration including surface descriptors, actions, and
/// optional encryption metadata for sensitive form fields.
pub fn build_surface_registration(
    encryption_public_key: Option<String>,
    _catalog: &uptrakit_plugin_infrastructure_registry::PluginCatalog,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
) -> SurfaceRegistration {
    let infra_primary_actions = collect_infra_primary_actions();
    let actions = build_actions();
    let action_index: BTreeMap<&str, &SurfaceActionDescriptor> = actions
        .iter()
        .map(|action| (action.action_id.as_str(), action))
        .collect();

    let mut registered_surfaces = Vec::new();
    if let Some(surface) = build_registered_surface(&action_index, &infra_primary_actions) {
        registered_surfaces.push(surface);
    }

    let mut registration_caps = BTreeSet::new();
    for surface in &registered_surfaces {
        registration_caps.extend(surface.descriptor.required_capabilities.0.iter().cloned());
    }

    let provider_id = service_id
        .map(|id| format!("service.uptrakit-agent-ssh.{id}"))
        .unwrap_or_else(|| "service.uptrakit-agent-ssh".to_string());

    SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id,
            provider_kind: surfaces::ProviderKind::Service,
            provider_namespace: "service".to_string(),
        },
        framework_generation: FrameworkGeneration::new(1, 0),
        capabilities: CapabilitySet(registration_caps),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Tenant,
            tenant_id: tenant_id.map(|id| id.to_string()),
        },
        surfaces: registered_surfaces,
        encryption_metadata: encryption_public_key.map(|public_key| ProviderEncryptionMetadata {
            key_id: service_id
                .map(|id| format!("ssh-agent-{id}"))
                .unwrap_or_else(|| "ssh-agent".to_string()),
            algorithm: ProviderEncryptionAlgorithm::EciesP256,
            public_key,
        }),
    }
}

/// Build the surface action library for runtime registration.
pub fn build_actions() -> Vec<SurfaceActionDescriptor> {
    use uptrakit_plugin_infrastructure_registry::all_descriptors;

    let mut actions = vec![
        // Data-load action: populates the hosts table.
        SurfaceActionDescriptor::new("list-hosts", "List Hosts")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(15),
        SurfaceActionDescriptor::new("remove-host", "Remove Host")
            .with_permission(Permission::UpdateHosts)
            .destructive()
            .with_confirm_entity_field("name")
            .with_timeout(30)
            .batch(),
        sync_host_action(),
        bootstrap_action(),
        // Internal wizard-step actions (not shown in UI directly).
        SurfaceActionDescriptor::new("bootstrap-connect", "Bootstrap Connect")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(60),
        SurfaceActionDescriptor::new("bootstrap-execute", "Bootstrap Execute")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(120),
        SurfaceActionDescriptor::new("sync-connect", "Sync Connect")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(60),
        SurfaceActionDescriptor::new("sync-execute", "Sync Execute")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(120),
    ];
    // Collect surface actions from infrastructure plugin descriptors.
    let infra_actions: Vec<SurfaceActionDescriptor> = all_descriptors()
        .iter()
        .filter(|d| d.family == PluginFamily::Infrastructure)
        .filter_map(|d| d.surface_actions)
        .flat_map(|surface_actions| (surface_actions.actions)())
        .collect();
    actions.extend(infra_actions);
    actions
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionHint {
    DataLoad,
    Action,
}

#[derive(Debug, Clone)]
struct InteractionRef {
    action_id: String,
    hint: InteractionHint,
    form_ui: Option<FormUiDescriptor>,
    sensitive_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionDisposition {
    Immediate,
    Form,
    Workflow,
    Unsupported,
}

fn build_registered_surface(
    action_index: &BTreeMap<&str, &SurfaceActionDescriptor>,
    infra_primary_actions: &[String],
) -> Option<surfaces::RegisteredSurface> {
    let surface_id = surfaces::SurfaceId::new(SSH_HOSTS_SURFACE_ID.to_string()).ok()?;
    let slot = surfaces::SLOT_SURFACE_PAGE.to_string();
    let priority =
        surfaces::slot_def(slot.as_str()).map_or(SSH_HOSTS_SURFACE_PRIORITY, |slot_def| {
            SSH_HOSTS_SURFACE_PRIORITY.clamp(
                slot_def.provider_priority_min,
                slot_def.provider_priority_max,
            )
        });
    let primary_actions = build_primary_actions(infra_primary_actions);
    let (root_node, data_sources, refs) = build_surface_parts(action_index, &primary_actions)?;
    let interactions = build_interactions(&refs, action_index);
    let targeting = Targeting::Targeted;
    let required_capabilities =
        compute_required_capabilities(&root_node, &targeting, &interactions, &data_sources);

    Some(surfaces::RegisteredSurface {
        descriptor: SurfaceDescriptor::builder()
            .surface_id(surface_id)
            .label(SSH_HOSTS_SURFACE_LABEL)
            .priority(priority)
            .slot(slot)
            .scope(surfaces::Scope::Tenant)
            .targeting(targeting)
            .required_permission(Permission::UpdateHosts.to_string())
            .provider_kind(surfaces::ProviderKind::Service)
            .required_capabilities(required_capabilities)
            .root_node(root_node)
            .build(),
        interactions,
        data_sources,
    })
}

fn build_surface_parts(
    action_index: &BTreeMap<&str, &SurfaceActionDescriptor>,
    primary_actions: &[String],
) -> Option<(SurfaceNode, Vec<DataSourceDescriptor>, Vec<InteractionRef>)> {
    let data_source_id = DataSourceId::new("data.primary").ok()?;
    let mut refs = vec![InteractionRef {
        action_id: SSH_HOSTS_DATA_ACTION_ID.to_string(),
        hint: InteractionHint::DataLoad,
        form_ui: None,
        sensitive_fields: vec![],
    }];
    let mut primary_ids = Vec::new();
    let mut row_ids = Vec::new();

    for action_id in primary_actions {
        let Some(action) = action_index.get(action_id.as_str()).copied() else {
            continue;
        };
        match action_disposition(action) {
            ActionDisposition::Unsupported => {}
            ActionDisposition::Immediate
            | ActionDisposition::Form
            | ActionDisposition::Workflow => {
                let Ok(interaction_id) = InteractionId::new(action_id.clone()) else {
                    continue;
                };
                primary_ids.push(interaction_id);
                let form_ui = action.ui.as_ref().and_then(action_ui_to_form_ui);
                refs.push(InteractionRef {
                    action_id: action_id.clone(),
                    hint: InteractionHint::Action,
                    form_ui: form_ui.clone(),
                    sensitive_fields: sensitive_fields_for_action(action),
                });
                collect_select_source_action_refs(form_ui.as_ref(), &mut refs);
                collect_workflow_step_refs(action, &mut refs);
            }
        }
    }

    for action_id in SSH_HOSTS_ROW_ACTION_IDS {
        let Some(action) = action_index.get(action_id).copied() else {
            continue;
        };
        match action_disposition(action) {
            ActionDisposition::Unsupported => {}
            ActionDisposition::Immediate
            | ActionDisposition::Form
            | ActionDisposition::Workflow => {
                let Ok(interaction_id) = InteractionId::new(action_id.to_string()) else {
                    continue;
                };
                row_ids.push(SurfaceTableRowAction {
                    interaction_id,
                    visible_when: action
                        .row_visible_when
                        .as_ref()
                        .map(row_visible_when_from_extension),
                });
                let form_ui = action.ui.as_ref().and_then(action_ui_to_form_ui);
                refs.push(InteractionRef {
                    action_id: action_id.to_string(),
                    hint: InteractionHint::Action,
                    form_ui: form_ui.clone(),
                    sensitive_fields: sensitive_fields_for_action(action),
                });
                collect_select_source_action_refs(form_ui.as_ref(), &mut refs);
                collect_workflow_step_refs(action, &mut refs);
            }
        }
    }

    let root = SurfaceNode::Section {
        title: None,
        children: vec![
            SurfaceNode::Table {
                data_source_id: data_source_id.clone(),
                columns: SSH_HOSTS_COLUMNS
                    .iter()
                    .map(|(key, label)| SurfaceTableColumn {
                        key: (*key).to_string(),
                        label: (*label).to_string(),
                    })
                    .collect(),
                row_actions: row_ids,
            },
            SurfaceNode::ActionBar {
                action_ids: primary_ids,
            },
        ],
    };

    let data_sources = vec![DataSourceDescriptor {
        data_source_id,
        kind: DataSourceKind::ProviderQuery {
            operation_id: SSH_HOSTS_DATA_ACTION_ID.to_string(),
        },
        result_schema: surfaces::SchemaContract::Any,
        pagination: Some(surfaces::DataSourcePagination {
            default_page_size: SSH_HOSTS_DEFAULT_PER_PAGE.min(1000) as u16,
            max_page_size: 1000,
        }),
        sorting: None,
        filtering: None,
        refresh_policy: RefreshPolicy::Manual,
        empty_state: None,
    }];

    Some((root, data_sources, refs))
}

fn action_disposition(action: &SurfaceActionDescriptor) -> ActionDisposition {
    match action.ui.as_ref() {
        Some(SurfaceActionUi::Form(_)) => ActionDisposition::Form,
        Some(SurfaceActionUi::Wizard { .. }) => ActionDisposition::Workflow,
        Some(_) => ActionDisposition::Unsupported,
        None => ActionDisposition::Immediate,
    }
}

fn collect_select_source_action_refs(
    form_ui: Option<&FormUiDescriptor>,
    refs: &mut Vec<InteractionRef>,
) {
    let Some(form_ui) = form_ui else {
        return;
    };
    for field in &form_ui.fields {
        let Some(surfaces::FormSelectSource::Action { action_id }) = field.select_source.as_ref()
        else {
            continue;
        };
        refs.push(InteractionRef {
            action_id: action_id.clone(),
            hint: InteractionHint::DataLoad,
            form_ui: None,
            sensitive_fields: vec![],
        });
    }
}

fn action_ui_to_form_ui(ui: &SurfaceActionUi) -> Option<FormUiDescriptor> {
    match ui {
        SurfaceActionUi::Form(form) => Some(form_ui_from_form(form)),
        _ => None,
    }
}

fn form_ui_from_form(form: &PluginSurfaceFormDescriptor) -> FormUiDescriptor {
    FormUiDescriptor {
        fields: form.fields.iter().map(field_from_extension).collect(),
        pre_load_interaction_id: form
            .pre_load_action
            .as_ref()
            .and_then(|id| InteractionId::new(id.clone()).ok()),
    }
}

fn collect_workflow_step_refs(action: &SurfaceActionDescriptor, refs: &mut Vec<InteractionRef>) {
    let Some(SurfaceActionUi::Wizard { steps }) = action.ui.as_ref() else {
        return;
    };

    let workflow_sensitive_fields = sensitive_fields_for_action(action);
    for step in steps {
        let form_ui = form_ui_from_form(&step.form);
        collect_select_source_action_refs(Some(&form_ui), refs);
        if let Some(pre_load_interaction_id) = form_ui.pre_load_interaction_id.as_ref() {
            refs.push(InteractionRef {
                action_id: pre_load_interaction_id.as_str().to_string(),
                hint: InteractionHint::DataLoad,
                form_ui: None,
                sensitive_fields: vec![],
            });
        }
        if let Some(submit_action) = &step.submit_action {
            refs.push(InteractionRef {
                action_id: submit_action.clone(),
                hint: InteractionHint::Action,
                form_ui: None,
                sensitive_fields: workflow_sensitive_fields.clone(),
            });
        }
    }
}

fn field_from_extension(field: &PluginFormFieldDescriptor) -> FormFieldDescriptor {
    FormFieldDescriptor {
        key: field.key.clone(),
        label: field.label.clone(),
        field_type: field.field_type.as_str().to_string(),
        required: field.required,
        placeholder: field.placeholder.clone(),
        help_text: field.help_text.clone(),
        default_value: field.default_value.as_ref().and_then(json_value_to_string),
        options: field
            .options
            .iter()
            .map(|option| FormSelectOption {
                value: option.value.clone(),
                label: option.label.clone(),
            })
            .collect(),
        select_source: field
            .select_source
            .as_ref()
            .and_then(|source| match source {
                PluginFormSelectSourceDescriptor::RestApi {
                    path,
                    value_field,
                    label_field,
                } => Some(surfaces::FormSelectSource::RestApi {
                    path: path.clone(),
                    value_field: value_field.clone(),
                    label_field: label_field.clone(),
                }),
                PluginFormSelectSourceDescriptor::Action { action_id } => {
                    Some(surfaces::FormSelectSource::Action {
                        action_id: action_id.clone(),
                    })
                }
                _ => None,
            }),
        sensitive: field.sensitive
            || matches!(
                field.field_type,
                PluginFormFieldType::Password | PluginFormFieldType::SshPrivateKey
            ),
        list: field.list,
        visible_when: field
            .visible_when
            .as_ref()
            .map(|visible_when| surfaces::FormVisibleWhen {
                field: visible_when.field.clone(),
                values: visible_when.values.clone(),
            }),
    }
}

fn json_value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(v) => Some(v.to_string()),
        serde_json::Value::Number(v) => Some(v.to_string()),
        serde_json::Value::String(v) => Some(v.clone()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(value).ok()
        }
    }
}

fn row_visible_when_from_extension(
    value: &PluginSurfaceRowVisibleWhen,
) -> surfaces::SurfaceRowVisibleWhen {
    surfaces::SurfaceRowVisibleWhen {
        field: value.field.clone(),
        condition: match value.condition {
            PluginSurfaceRowCondition::Present => surfaces::SurfaceRowCondition::Present,
            PluginSurfaceRowCondition::Absent => surfaces::SurfaceRowCondition::Absent,
            _ => surfaces::SurfaceRowCondition::Present,
        },
    }
}

fn sensitive_fields_for_action(action: &SurfaceActionDescriptor) -> Vec<String> {
    let mut sensitive = BTreeSet::new();
    match action.ui.as_ref() {
        Some(SurfaceActionUi::Form(form)) => {
            for field in &form.fields {
                if field.sensitive
                    || matches!(
                        field.field_type,
                        PluginFormFieldType::Password | PluginFormFieldType::SshPrivateKey
                    )
                {
                    sensitive.insert(field.key.clone());
                }
            }
        }
        Some(SurfaceActionUi::Wizard { steps }) => {
            for step in steps {
                for field in &step.form.fields {
                    if field.sensitive
                        || matches!(
                            field.field_type,
                            PluginFormFieldType::Password | PluginFormFieldType::SshPrivateKey
                        )
                    {
                        sensitive.insert(field.key.clone());
                    }
                }
            }
        }
        Some(_) | None => {}
    }
    sensitive.into_iter().collect()
}

fn build_interactions(
    refs: &[InteractionRef],
    action_index: &BTreeMap<&str, &SurfaceActionDescriptor>,
) -> Vec<InteractionDescriptor> {
    let mut merged: BTreeMap<
        String,
        (InteractionHint, Option<FormUiDescriptor>, BTreeSet<String>),
    > = BTreeMap::new();
    for reference in refs {
        let entry = merged.entry(reference.action_id.clone()).or_insert((
            reference.hint,
            None,
            BTreeSet::new(),
        ));
        if reference.hint == InteractionHint::DataLoad {
            entry.0 = InteractionHint::DataLoad;
        }
        if entry.1.is_none() {
            entry.1 = reference.form_ui.clone();
        }
        entry.2.extend(reference.sensitive_fields.iter().cloned());
    }

    let mut interactions = Vec::new();
    for (action_id, (hint, form_ui, sensitive_fields)) in merged {
        let Ok(interaction_id) = InteractionId::new(action_id.clone()) else {
            continue;
        };
        let Some(action) = action_index.get(action_id.as_str()).copied() else {
            continue;
        };
        let kind = match hint {
            InteractionHint::DataLoad => InteractionKind::DataLoad,
            InteractionHint::Action => action_kind_for_action(Some(action)),
        };
        let confirmation = if kind == InteractionKind::ConfirmableAction {
            Some(surfaces::InteractionConfirmation {
                title: format!("Confirm {}", action.label),
                message: "This action may modify existing data.".to_string(),
                confirm_label: None,
                cancel_label: None,
                severity: surfaces::ConfirmationSeverity::Danger,
            })
        } else {
            None
        };

        let timeout_seconds = action
            .timeout_seconds
            .map(|seconds| seconds.clamp(1, 300) as u16);
        let workflow_steps = match kind {
            InteractionKind::Workflow => workflow_steps_from_action(Some(action)),
            _ => vec![],
        };

        interactions.push(InteractionDescriptor {
            interaction_id,
            kind,
            label: action.label.clone(),
            required_permission: permission_or_none(&action.permission),
            input_schema: Some(surfaces::SchemaContract::Object),
            result_schema: Some(surfaces::SchemaContract::Any),
            sensitive_fields: sensitive_fields.into_iter().collect(),
            timeout_seconds,
            confirmation,
            transport: InteractionTransport::ProviderProxied,
            workflow_steps,
            form_ui,
        });
    }
    interactions
}

fn workflow_steps_from_action(
    action: Option<&SurfaceActionDescriptor>,
) -> Vec<surfaces::WorkflowStepDescriptor> {
    let Some(action) = action else {
        return vec![];
    };
    let Some(SurfaceActionUi::Wizard { steps }) = action.ui.as_ref() else {
        return vec![];
    };

    steps
        .iter()
        .map(|step| surfaces::WorkflowStepDescriptor {
            step_id: step.step_id.clone(),
            label: step.label.clone(),
            form_ui: Some(form_ui_from_form(&step.form)),
            submit_interaction_id: step
                .submit_action
                .as_ref()
                .and_then(|id| InteractionId::new(id.clone()).ok()),
            render_previous_response: step.render_previous_response,
            input_schema: surfaces::SchemaContract::Object,
            result_schema: surfaces::SchemaContract::Any,
        })
        .collect()
}

fn action_kind_for_action(action: Option<&SurfaceActionDescriptor>) -> InteractionKind {
    let Some(action) = action else {
        return InteractionKind::MutationAction;
    };
    if action.destructive && action.confirm_entity_field.is_some() {
        return InteractionKind::ConfirmableAction;
    }
    if matches!(action.ui.as_ref(), Some(SurfaceActionUi::Wizard { .. })) {
        return InteractionKind::Workflow;
    }
    if matches!(action.ui.as_ref(), Some(SurfaceActionUi::Form(_))) {
        return InteractionKind::FormSubmit;
    }
    InteractionKind::MutationAction
}

fn permission_or_none(permission: &str) -> Option<String> {
    let trimmed = permission.trim();
    if trimmed.is_empty() || trimmed == "none" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn compute_required_capabilities(
    root_node: &SurfaceNode,
    targeting: &Targeting,
    interactions: &[InteractionDescriptor],
    data_sources: &[DataSourceDescriptor],
) -> CapabilitySet {
    let mut caps = BTreeSet::new();
    collect_node_caps(root_node, &mut caps);
    for interaction in interactions {
        match interaction.kind {
            InteractionKind::MutationAction => {
                caps.insert(surfaces::Capability::MutationAction);
            }
            InteractionKind::FormSubmit => {
                caps.insert(surfaces::Capability::FormSubmit);
            }
            InteractionKind::Workflow => {
                caps.insert(surfaces::Capability::Workflow);
            }
            InteractionKind::Navigate => {
                caps.insert(surfaces::Capability::Navigate);
            }
            InteractionKind::DataLoad => {
                caps.insert(surfaces::Capability::DataLoad);
            }
            InteractionKind::ConfirmableAction => {
                caps.insert(surfaces::Capability::ConfirmableAction);
            }
        }
        if !interaction.sensitive_fields.is_empty() {
            caps.insert(surfaces::Capability::SensitiveFields);
        }
        if matches!(interaction.transport, InteractionTransport::ProviderProxied) {
            caps.insert(surfaces::Capability::ProviderInitiatedActions);
        }
    }
    for data_source in data_sources {
        match data_source.kind {
            DataSourceKind::Static { .. } => {
                caps.insert(surfaces::Capability::StaticDataSource);
            }
            DataSourceKind::ControllerQuery { .. } => {
                caps.insert(surfaces::Capability::ControllerQueryDataSource);
            }
            DataSourceKind::ProviderQuery { .. } => {
                caps.insert(surfaces::Capability::ProviderQueryDataSource);
            }
        }
    }
    match targeting {
        Targeting::Universal => {
            caps.insert(surfaces::Capability::UniversalTargeting);
        }
        Targeting::Targeted => {
            caps.insert(surfaces::Capability::TargetedTargeting);
        }
        _ => {
            tracing::warn!(
                ?targeting,
                "unknown Targeting variant; defaulting to UniversalTargeting capability"
            );
            caps.insert(surfaces::Capability::UniversalTargeting);
        }
    }
    CapabilitySet(caps)
}

fn collect_node_caps(node: &SurfaceNode, caps: &mut BTreeSet<surfaces::Capability>) {
    match node {
        SurfaceNode::Section { children, .. } => {
            caps.insert(surfaces::Capability::SectionNode);
            for child in children {
                collect_node_caps(child, caps);
            }
        }
        SurfaceNode::TextBlock { .. } => {
            caps.insert(surfaces::Capability::TextBlockNode);
        }
        SurfaceNode::KeyValue { .. } => {
            caps.insert(surfaces::Capability::KeyValueNode);
        }
        SurfaceNode::Table { .. } => {
            caps.insert(surfaces::Capability::TableNode);
        }
        SurfaceNode::Form { .. } => {
            caps.insert(surfaces::Capability::FormNode);
        }
        SurfaceNode::ActionBar { .. } => {
            caps.insert(surfaces::Capability::ActionBarNode);
        }
        SurfaceNode::Tabs { tabs } => {
            caps.insert(surfaces::Capability::TabsNode);
            for tab in tabs {
                collect_node_caps(&tab.root, caps);
            }
        }
        SurfaceNode::Callout { .. } => {
            caps.insert(surfaces::Capability::CalloutNode);
        }
        SurfaceNode::EmptyState { .. } => {
            caps.insert(surfaces::Capability::EmptyStateNode);
        }
        SurfaceNode::ModalTrigger { modal_nodes, .. } => {
            caps.insert(surfaces::Capability::ModalTriggerNode);
            for modal in modal_nodes {
                collect_node_caps(modal, caps);
            }
        }
        SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
            caps.insert(surfaces::Capability::WorkflowTriggerNode);
            for step in step_nodes {
                collect_node_caps(step, caps);
            }
        }
        _ => {
            tracing::warn!(?node, "unknown SurfaceNode variant; no capability inserted");
        }
    }
}

/// Build the sync-host action definition as a 3-step wizard.
fn sync_host_action() -> SurfaceActionDescriptor {
    let connect_step = PluginSurfaceWorkflowStep::new(
        "connect",
        "Connection & Authentication",
        PluginSurfaceFormDescriptor::new(vec![
            PluginFormFieldDescriptor::new("auth_method", "Auth Method")
                .with_type(PluginFormFieldType::Select)
                .with_default_value("stored")
                .with_options(vec![
                    PluginFormSelectOptionDescriptor::new("stored", "Stored Credentials"),
                    PluginFormSelectOptionDescriptor::new("password", "Password"),
                    PluginFormSelectOptionDescriptor::new("private_key", "Private Key"),
                ]),
            PluginFormFieldDescriptor::new("username", "SSH Username")
                .with_default_value("root")
                .with_help_text("User to connect as (e.g. root). Only used with custom auth.")
                .with_visible_when(
                    "auth_method",
                    vec!["password".to_string(), "private_key".to_string()],
                ),
            PluginFormFieldDescriptor::new("auth_password", "SSH Password")
                .with_type(PluginFormFieldType::Password)
                .with_help_text("Required when auth method is 'password'.")
                .sensitive()
                .with_visible_when("auth_method", vec!["password".to_string()]),
            PluginFormFieldDescriptor::new("auth_private_key", "SSH Private Key")
                .with_type(PluginFormFieldType::SshPrivateKey)
                .with_placeholder("-----BEGIN OPENSSH PRIVATE KEY-----")
                .with_help_text(
                    "PEM-encoded private key. Required when auth method is 'private_key'.",
                )
                .sensitive()
                .with_visible_when("auth_method", vec!["private_key".to_string()]),
            PluginFormFieldDescriptor::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            PluginFormFieldDescriptor::new("auto", "Auto")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Skip review and execute immediately."),
        ]),
    )
    .with_submit_action("sync-connect");

    let review_step = PluginSurfaceWorkflowStep::new(
        "review",
        "Review Plan",
        PluginSurfaceFormDescriptor::new(vec![]),
    )
    .with_render_previous_response();

    let execute_step = PluginSurfaceWorkflowStep::new(
        "execute",
        "Execute",
        PluginSurfaceFormDescriptor::new(vec![]),
    )
    .with_submit_action("sync-execute");

    SurfaceActionDescriptor::new("sync-host", "Sync Host")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(120)
        .with_ui(SurfaceActionUi::Wizard {
            steps: vec![connect_step, review_step, execute_step],
        })
        .batch()
}

/// Build the bootstrap host action definition as a 3-step wizard.
fn bootstrap_action() -> SurfaceActionDescriptor {
    let connect_step = PluginSurfaceWorkflowStep::new(
        "connect",
        "Connection & Authentication",
        PluginSurfaceFormDescriptor::new(vec![
            PluginFormFieldDescriptor::new("target", "SSH Target")
                .required()
                .with_placeholder("[user@]host[:port]")
                .with_help_text(
                    "SSH target in [user@]host[:port] format. Default user: root, port: 22.",
                ),
            PluginFormFieldDescriptor::new("name", "Host Name")
                .with_placeholder("my-server")
                .with_help_text("Optional. Defaults to the hostname from the SSH target."),
            PluginFormFieldDescriptor::new("auth_method", "Auth Method")
                .with_type(PluginFormFieldType::Select)
                .required()
                .with_default_value("password")
                .with_options(vec![
                    PluginFormSelectOptionDescriptor::new("password", "Password"),
                    PluginFormSelectOptionDescriptor::new("private_key", "Private Key"),
                ]),
            PluginFormFieldDescriptor::new("auth_password", "SSH Password")
                .with_type(PluginFormFieldType::Password)
                .with_help_text("Required when auth method is 'password'.")
                .sensitive()
                .with_visible_when("auth_method", vec!["password".to_string()]),
            PluginFormFieldDescriptor::new("auth_private_key", "SSH Private Key")
                .with_type(PluginFormFieldType::SshPrivateKey)
                .with_placeholder("-----BEGIN OPENSSH PRIVATE KEY-----")
                .with_help_text(
                    "PEM-encoded private key. Required when auth method is 'private_key'.",
                )
                .sensitive()
                .with_visible_when("auth_method", vec!["private_key".to_string()]),
            PluginFormFieldDescriptor::new("target_username", "Target Username")
                .with_help_text("User to create/use on the remote host.")
                .with_default_value("uptrakit"),
            PluginFormFieldDescriptor::new("host_key_fingerprint", "Host Key Fingerprint")
                .with_placeholder("SHA256:...")
                .with_help_text("Expected SHA-256 fingerprint of the host key."),
            PluginFormFieldDescriptor::new("strict_host_key_checking", "Strict Host Key Checking")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Require fingerprint match (disables TOFU)."),
            PluginFormFieldDescriptor::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            PluginFormFieldDescriptor::new("remove_stale_keys", "Remove Stale Keys")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Remove existing Uptrakit-managed keys before writing new ones."),
            PluginFormFieldDescriptor::new("auto", "Auto")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Skip review and execute immediately."),
        ]),
    )
    .with_submit_action("bootstrap-connect");

    let review_step = PluginSurfaceWorkflowStep::new(
        "review",
        "Review Plan",
        PluginSurfaceFormDescriptor::new(vec![]),
    )
    .with_render_previous_response();

    let execute_step = PluginSurfaceWorkflowStep::new(
        "execute",
        "Execute",
        PluginSurfaceFormDescriptor::new(vec![]),
    )
    .with_submit_action("bootstrap-execute");

    SurfaceActionDescriptor::new("bootstrap", "Bootstrap Host")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(120)
        .with_ui(SurfaceActionUi::Wizard {
            steps: vec![connect_step, review_step, execute_step],
        })
}

// ── Surface runtime context ──────────────────────────────────────────

/// Shared context for surface request handling.
///
/// Groups the handler-level state needed by action dispatch and background
/// bootstrap tasks, avoiding parameter-count explosion on public APIs.
pub struct SurfaceRuntimeContext<'a> {
    pub db: &'a sea_orm::DatabaseConnection,
    pub state_dir: &'a Path,
    pub private_key_der: Option<&'a [u8]>,
    pub service_id: Option<uuid::Uuid>,
    pub tenant_id: Option<uuid::Uuid>,
    pub bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
    pub surface_proxy: &'a Arc<uptrakit_service_sdk::ServiceSurfaceProxy>,
    pub infra_bundles: Arc<Vec<InfraBundle>>,
}

// ── InfraActionInvoker implementation ────────────────────────────────

/// [`InfraActionInvoker`] that routes calls through the `ServiceSurfaceProxy`.
///
/// Wraps `invoke_proxy_action` so that infrastructure plugins can invoke
/// controller-side surface actions without depending on `uptrakit-service-sdk`.
pub struct InfraActionInvokerImpl<'a> {
    proxy: &'a uptrakit_service_sdk::ServiceSurfaceProxy,
    bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
    tenant_id: Option<uuid::Uuid>,
}

impl<'a> InfraActionInvokerImpl<'a> {
    pub fn new(
        proxy: &'a uptrakit_service_sdk::ServiceSurfaceProxy,
        bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
        tenant_id: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            proxy,
            bg_tx,
            tenant_id,
        }
    }
}

#[async_trait]
impl InfraActionInvoker for InfraActionInvokerImpl<'_> {
    async fn invoke(
        &self,
        surface_id: &str,
        action_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<SurfaceActionResponse, InfraActionInvokeError> {
        invoke_proxy_surface_action(
            self.proxy,
            self.bg_tx,
            self.tenant_id,
            surface_id,
            action_id,
            params,
        )
        .await
        .map_err(|e| e.to_string().into())
    }
}

// ── Infra plugin action dispatch ─────────────────────────────────────

/// Spawn an infrastructure plugin action as a background task.
///
/// Iterates all registered infra plugins; the first one to return `Some`
/// wins. If no plugin handles the action, an error response is sent.
fn spawn_infra_plugin_action(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let proxy = std::sync::Arc::clone(ctx.surface_proxy);
    let infra_bundles = std::sync::Arc::clone(&ctx.infra_bundles);
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());

    tokio::spawn(async move {
        let db = match crate::db::init_db(&state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_surface_error_response(
                    request.request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return;
            }
        };

        let tenant_id_str = tenant_id.map(|t| t.to_string());
        let action_invoker = InfraActionInvokerImpl::new(&proxy, &bg_tx, tenant_id);
        let guest_bootstrap = AgentGuestBootstrapExecutor {
            state_dir: state_dir.clone(),
            service_id,
        };
        let plugin_ctx = InfraPluginContext {
            db: &db,
            tenant_id: tenant_id_str.as_deref(),
            service_id,
            state_dir: &state_dir,
            private_key_der: private_key_der.as_deref(),
            action_invoker: &action_invoker,
            guest_bootstrap: &guest_bootstrap,
        };

        let mut response: Option<SurfaceActionResponse> = None;
        for bundle in infra_bundles.iter() {
            if let Some(guest_exec) = bundle.guest_exec.as_ref()
                && let Some(resp) = guest_exec
                    .handle_service_extension_action(&plugin_ctx, &request)
                    .await
            {
                response = Some(resp);
                break;
            }
        }

        let resp = response.unwrap_or_else(|| {
            tracing::warn!(
                action_id = %request.interaction_id,
                surface_id = %request.surface_id,
                "no infrastructure plugin handled this action"
            );
            make_surface_error_response(request.request_id, "unknown action")
        });

        emit_surface_mutation_audit(
            &bg_tx,
            tenant_id,
            request.interaction_id.as_str(),
            request.request_id,
            &serde_json::Value::Object(request.params.clone()),
            &resp,
        )
        .await;

        if bg_tx
            .send(ServiceMessage::SurfaceActionResponse(resp))
            .await
            .is_err()
        {
            tracing::error!("failed to send infra plugin action result via bg_tx");
        }
    });
}

// ── Action dispatch ──────────────────────────────────────────────────

/// Dispatch a surface request to the appropriate handler.
///
/// Actions that complete quickly (`list-hosts`, `remove-host`) respond inline.
/// Long-running actions (`bootstrap`) are spawned as background tasks via `bg_tx`.
#[tracing::instrument(skip_all, fields(
    request_id = %request.request_id,
    surface_id = %request.surface_id,
    interaction_id = %request.interaction_id,
))]
pub async fn handle_surface_action_request(
    request: SurfaceActionRequest,
    ctx: &SurfaceRuntimeContext<'_>,
    conn: &mut dyn ServiceTransport,
) {
    handle_surface_request_internal(request, ctx, conn).await;
}

async fn handle_surface_request_internal(
    request: SurfaceActionRequest,
    ctx: &SurfaceRuntimeContext<'_>,
    conn: &mut dyn ServiceTransport,
) {
    if request.surface_id.as_str() != SSH_HOSTS_SURFACE_ID {
        tracing::warn!(
            surface_id = %request.surface_id,
            "received request for unknown surface"
        );
        let response = make_surface_error_response(request.request_id, "unknown surface");
        send_response(conn, response).await;
        return;
    }

    if !is_registered_interaction(request.interaction_id.as_str()) {
        tracing::warn!(
            surface_id = %request.surface_id,
            action_id = %request.interaction_id,
            "received request for unregistered interaction"
        );
        let response = make_surface_error_response(request.request_id, "unknown action");
        send_response(conn, response).await;
        return;
    }

    match request.interaction_id.as_str() {
        "list-hosts" => {
            let response = handle_list_hosts(request.request_id, &request.params, ctx.db).await;
            send_response(conn, response).await;
        }
        "remove-host" => {
            let response = handle_remove_host(request.request_id, &request.params, ctx.db).await;
            emit_surface_mutation_audit(
                ctx.bg_tx,
                ctx.tenant_id,
                "remove-host",
                request.request_id,
                &serde_json::Value::Object(request.params.clone()),
                &response,
            )
            .await;
            send_response(conn, response).await;
        }
        "bootstrap-connect" => {
            spawn_bootstrap_connect(request, ctx);
        }
        "bootstrap" => {
            let response = make_surface_error_response(
                request.request_id,
                "workflow entry interaction cannot be executed directly",
            );
            send_response(conn, response).await;
        }
        "bootstrap-execute" => {
            spawn_bootstrap_execute(request, ctx);
        }
        "sync-connect" => {
            spawn_sync_connect(request, ctx);
        }
        "sync-host" => {
            let response = make_surface_error_response(
                request.request_id,
                "workflow entry interaction cannot be executed directly",
            );
            send_response(conn, response).await;
        }
        "sync-execute" => {
            spawn_sync_execute(request, ctx);
        }
        _ => {
            // Delegate to infrastructure plugins.
            spawn_infra_plugin_action(request, ctx);
        }
    }
}

// ── Action handlers ──────────────────────────────────────────────────

/// List SSH hosts from the local database with pagination.
async fn handle_list_hosts(
    request_id: uuid::Uuid,
    params: &serde_json::Map<String, serde_json::Value>,
    db: &sea_orm::DatabaseConnection,
) -> SurfaceActionResponse {
    let page = params
        .get("page")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .clamp(1, 1000);

    match host_ops::list_hosts_paginated(db, page, per_page).await {
        Ok(result) => {
            let items: Vec<serde_json::Value> = result
                .items
                .into_iter()
                .map(|h| {
                    json!({
                        "id": h.id,
                        "name": h.name,
                        "hostname": h.hostname,
                        "port": h.port,
                        "username": h.username,
                    })
                })
                .collect();
            make_surface_success_response(
                request_id,
                json!({
                    "items": items,
                    "total": result.total,
                    "page": result.page,
                    "per_page": result.per_page,
                    "total_pages": result.total_pages,
                }),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list hosts");
            make_surface_error_response(request_id, "failed to list hosts")
        }
    }
}

/// Remove a host from the local database.
async fn handle_remove_host(
    request_id: uuid::Uuid,
    params: &serde_json::Map<String, serde_json::Value>,
    db: &sea_orm::DatabaseConnection,
) -> SurfaceActionResponse {
    let host_id = match params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return make_surface_error_response(request_id, "missing required field 'id'"),
    };

    match host_ops::remove_host(db, host_id).await {
        Ok(true) => make_surface_success_response(request_id, json!({ "removed": true })),
        Ok(false) => make_surface_error_response(request_id, "host not found"),
        Err(e) => {
            tracing::error!(error = %e, host = %host_id, "failed to remove host");
            make_surface_error_response(request_id, "failed to remove host")
        }
    }
}

// ── Background tasks ─────────────────────────────────────────────────

/// Spawn the bootstrap-connect (plan) step as a background task.
fn spawn_bootstrap_connect(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let response = run_bootstrap_connect(
            request_id,
            &params,
            sensitive_params_sealed.as_deref(),
            private_key_der.as_deref(),
            service_id,
            tenant_id,
            &state_dir,
        )
        .await;
        let msg = ServiceMessage::SurfaceActionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-connect result via bg_tx");
        }
    });
}

/// Spawn the bootstrap-execute step as a background task.
fn spawn_bootstrap_execute(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let response = run_bootstrap_execute(BootstrapExecuteArgs {
            request_id,
            params: &params,
            sensitive_params_sealed: sensitive_params_sealed.as_deref(),
            private_key_der: private_key_der.as_deref(),
            service_id,
            tenant_id,
            state_dir: &state_dir,
            bg_tx: &bg_tx,
        })
        .await;
        emit_surface_mutation_audit(
            &bg_tx,
            tenant_id,
            "bootstrap-execute",
            request_id,
            &params,
            &response,
        )
        .await;
        let msg = ServiceMessage::SurfaceActionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-execute result via bg_tx");
        }
    });
}

/// Spawn the sync-connect (plan) step as a background task.
fn spawn_sync_connect(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let db_state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let Some((host_id, auth_override)) = resolve_sync_auth(
            &params,
            sensitive_params_sealed.as_deref(),
            request_id,
            private_key_der.as_deref(),
            &bg_tx,
        )
        .await
        else {
            return;
        };

        let allow_all = param_bool(&params, "allow_all");

        let db = match crate::db::init_db(&db_state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_surface_error_response(
                    request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return;
            }
        };

        let response =
            match sync::sync_connect(&host_id, &db, tenant_id, auth_override.as_ref(), allow_all)
                .await
            {
                Ok(plan) => match serde_json::to_value(&plan) {
                    Ok(data) => make_surface_success_response(request_id, data),
                    Err(e) => make_surface_error_response(
                        request_id,
                        &format!("failed to serialize plan: {e}"),
                    ),
                },
                Err(e) => make_surface_error_response(request_id, &e),
            };
        let _ = bg_tx
            .send(ServiceMessage::SurfaceActionResponse(response))
            .await;
    });
}

/// Spawn the sync-execute step as a background task.
fn spawn_sync_execute(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let db_state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let Some((host_id, auth_override)) = resolve_sync_auth(
            &params,
            sensitive_params_sealed.as_deref(),
            request_id,
            private_key_der.as_deref(),
            &bg_tx,
        )
        .await
        else {
            return;
        };

        let allow_all = param_bool(&params, "allow_all");
        let skip_actions = parse_skip_actions(&params);

        let db = match crate::db::init_db(&db_state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_surface_error_response(
                    request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return;
            }
        };

        let response = match sync::sync_execute(
            &host_id,
            &db,
            tenant_id,
            auth_override.as_ref(),
            allow_all,
            &skip_actions,
        )
        .await
        {
            Ok((summary, plugin_config_reports)) => {
                // Send any plugin config reports generated during sync (e.g.
                // a recreated PVE API token).
                for report in &plugin_config_reports {
                    let payload: uptrakit_internal_wire::ReportPluginConfigPayload =
                        serde_json::from_value(serde_json::json!({
                            "request_id": uuid::Uuid::now_v7().to_string(),
                            "plugin_type": report.plugin_type,
                            "name": report.name,
                            "config": report.config,
                        }))
                        .expect("ReportPluginConfigPayload JSON is always valid");
                    if bg_tx
                        .send(ServiceMessage::ReportPluginConfig(payload))
                        .await
                        .is_err()
                    {
                        tracing::error!("failed to send ReportPluginConfig via bg_tx during sync");
                    }
                }
                make_surface_success_response(request_id, serde_json::json!({ "summary": summary }))
            }
            Err(e) => make_surface_error_response(request_id, &e),
        };
        emit_surface_mutation_audit(
            &bg_tx,
            tenant_id,
            "sync-execute",
            request_id,
            &params,
            &response,
        )
        .await;
        let msg = ServiceMessage::SurfaceActionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send sync-execute result via bg_tx");
        }
    });
}

/// Invoke a surface action on the controller via the proxy.
///
/// Sends the request via `bg_tx` (which flows through the event loop to
/// `conn.send()`), then waits for the controller's response via the proxy's
/// oneshot channel.
pub(crate) async fn invoke_proxy_surface_action(
    proxy: &uptrakit_service_sdk::ServiceSurfaceProxy,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    tenant_id: Option<uuid::Uuid>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> Result<SurfaceActionResponse, uptrakit_service_sdk::ServiceSurfaceProxyError> {
    let Some(tenant_id) = tenant_id else {
        return Err(uptrakit_service_sdk::ServiceSurfaceProxyError::SendFailed);
    };
    let Ok(surface_id) = surfaces::SurfaceId::new(surface_id.to_string()) else {
        return Err(uptrakit_service_sdk::ServiceSurfaceProxyError::SendFailed);
    };
    let Ok(interaction_id) = surfaces::InteractionId::new(action_id.to_string()) else {
        return Err(uptrakit_service_sdk::ServiceSurfaceProxyError::SendFailed);
    };
    let params_map = params.as_object().cloned().unwrap_or_default();
    let pending = proxy.invoke(
        tenant_id,
        surface_id,
        interaction_id,
        &uuid::Uuid::now_v7().to_string(),
        surfaces::CallerOrigin::Provider {
            provider_id: "service.uptrakit-agent-ssh".to_string(),
        },
        params_map,
        None,
        None,
    );

    // Send the request to the controller via bg_tx.
    if bg_tx.send(pending.message.clone()).await.is_err() {
        return Err(uptrakit_service_sdk::ServiceSurfaceProxyError::SendFailed);
    }

    // Wait for the response (15s timeout for proxy calls).
    let response = pending
        .wait(proxy, std::time::Duration::from_secs(15))
        .await?;
    Ok(response)
}

/// Extract a boolean parameter from an extension params object.
///
/// Accepts both JSON booleans (`true`/`false`) and the string representations
/// `"true"`/`"false"` that form-based UIs may emit when all field values are
/// carried as strings. Returns `false` for absent or unrecognised values.
fn param_bool(params: &serde_json::Value, key: &str) -> bool {
    match params.get(key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    }
}

/// Parse `BootstrapParams` from extension request params and decrypted sensitive params.
fn parse_bootstrap_params(
    params: &serde_json::Value,
    sensitive: Option<&SensitiveAuthParams>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
) -> Result<BootstrapParams, String> {
    let target_str = params
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required field 'target'".to_string())?;

    let parsed_target: SshTarget = target_str
        .parse()
        .map_err(|e| format!("invalid target: {e}"))?;

    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| parsed_target.hostname.clone());

    let auth_method = params
        .get("auth_method")
        .and_then(|v| v.as_str())
        .unwrap_or("password");

    let auth_password = sensitive.and_then(|s| s.auth_password.clone().map(SecretString::new));
    let auth_private_key =
        sensitive.and_then(|s| s.auth_private_key.clone().map(SecretString::new));

    match auth_method {
        "password" if auth_password.is_none() => {
            return Err("auth_method is 'password' but no password provided".to_string());
        }
        "private_key" if auth_private_key.is_none() => {
            return Err("auth_method is 'private_key' but no private key provided".to_string());
        }
        _ => {}
    }

    let target_username = params
        .get("target_username")
        .and_then(|v| v.as_str())
        .unwrap_or("uptrakit")
        .to_string();

    let host_key_fingerprint = params
        .get("host_key_fingerprint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let strict_host_key_checking = param_bool(params, "strict_host_key_checking");
    let allow_all = param_bool(params, "allow_all");
    let remove_stale_keys = param_bool(params, "remove_stale_keys");

    let host_id = uuid::Uuid::now_v7();

    Ok(BootstrapParams {
        name,
        hostname: parsed_target.hostname,
        port: parsed_target.port.unwrap_or(22) as i32,
        auth_username: parsed_target.username.unwrap_or_else(|| "root".to_string()),
        auth_password,
        auth_private_key_pem: auth_private_key,
        use_ssh_agent: false,
        target_username,
        target_private_key_pem: None,
        host_key_fingerprint,
        strict_host_key_checking,
        allow_all,
        host_id,
        service_id,
        tenant_id,
        remove_stale_keys,
    })
}

/// The bootstrap-connect handler: probe the host and return a plan.
#[tracing::instrument(skip_all, fields(request_id = %request_id))]
async fn run_bootstrap_connect(
    request_id: uuid::Uuid,
    params: &serde_json::Value,
    sensitive_params_sealed: Option<&str>,
    private_key_der: Option<&[u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &Path,
) -> SurfaceActionResponse {
    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            sensitive_params_sealed,
            private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    let bootstrap_params =
        match parse_bootstrap_params(params, sensitive.as_ref(), service_id, tenant_id) {
            Ok(p) => p,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    match bootstrap::bootstrap_connect(state_dir, &bootstrap_params).await {
        Ok(plan) => match serde_json::to_value(&plan) {
            Ok(data) => make_surface_success_response(request_id, data),
            Err(e) => {
                make_surface_error_response(request_id, &format!("failed to serialize plan: {e}"))
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "bootstrap-connect failed");
            make_surface_error_response(request_id, &format!("bootstrap connect failed: {e}"))
        }
    }
}

/// Arguments for the bootstrap-execute handler, bundled to stay within the 7-arg clippy limit.
struct BootstrapExecuteArgs<'a> {
    request_id: uuid::Uuid,
    params: &'a serde_json::Value,
    sensitive_params_sealed: Option<&'a str>,
    private_key_der: Option<&'a [u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &'a Path,
    bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
}

/// The bootstrap-execute handler: execute the bootstrap with optional skip set.
#[tracing::instrument(skip_all, fields(request_id = %args.request_id))]
async fn run_bootstrap_execute(args: BootstrapExecuteArgs<'_>) -> SurfaceActionResponse {
    let request_id = args.request_id;
    let bg_tx = args.bg_tx;
    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            args.sensitive_params_sealed,
            args.private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    let bootstrap_params = match parse_bootstrap_params(
        args.params,
        sensitive.as_ref(),
        args.service_id,
        args.tenant_id,
    ) {
        Ok(p) => p,
        Err(msg) => return make_surface_error_response(request_id, &msg),
    };

    let host_id = bootstrap_params.host_id;
    let skip_actions = parse_skip_actions(args.params);

    match bootstrap::bootstrap_execute(args.state_dir, bootstrap_params, &skip_actions).await {
        Ok(result) => {
            tracing::info!(%host_id, "bootstrap completed successfully");

            // For each infra plugin that detected infrastructure, send
            // ReportPluginConfig if new credentials were created.
            send_infra_plugin_reports(bg_tx, host_id, &result.infra_results).await;

            let any_infra = result.infra_results.iter().any(|r| r.detected);
            let mut data = json!({ "host_id": host_id.to_string() });
            if any_infra {
                data["has_infrastructure"] = json!(true);
            }
            make_surface_success_response(request_id, data)
        }
        Err(e) => {
            tracing::error!(error = %e, "bootstrap failed");
            make_surface_error_response(request_id, &format!("bootstrap failed: {e}"))
        }
    }
}

/// Build a `SyncAuthOverride` from extension params and decrypted sensitive params.
fn build_sync_auth_override(
    params: &serde_json::Value,
    sensitive: Option<&SensitiveAuthParams>,
) -> Result<Option<sync::SyncAuthOverride>, String> {
    let auth_method = params
        .get("auth_method")
        .and_then(|v| v.as_str())
        .unwrap_or("stored");

    match auth_method {
        "stored" => Ok(None),
        "password" => {
            let password = sensitive.and_then(|s| s.auth_password.as_deref());
            match password {
                Some(pw) => Ok(Some(sync::SyncAuthOverride {
                    username: params
                        .get("username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("root")
                        .to_string(),
                    auth_password: Some(pw.to_string()),
                    auth_private_key_pem: None,
                })),
                None => Err("auth_method is 'password' but no password provided".to_string()),
            }
        }
        "private_key" => {
            let key = sensitive.and_then(|s| s.auth_private_key.as_deref());
            match key {
                Some(pem) => Ok(Some(sync::SyncAuthOverride {
                    username: params
                        .get("username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("root")
                        .to_string(),
                    auth_password: None,
                    auth_private_key_pem: Some(pem.to_string()),
                })),
                None => Err("auth_method is 'private_key' but no private key provided".to_string()),
            }
        }
        other => Err(format!("unknown auth_method '{other}'")),
    }
}

/// Parse `skip_actions` from params as a `HashSet<String>`.
///
/// Expects a JSON array of strings at `params["skip_actions"]`.
/// Returns an empty set if the key is absent or not an array.
fn parse_skip_actions(params: &serde_json::Value) -> HashSet<String> {
    params
        .get("skip_actions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

// ── Sensitive params ─────────────────────────────────────────────────

/// Sensitive authentication parameters extracted from the ECIES sealed box.
///
/// Used by both bootstrap and sync actions — any action that accepts SSH
/// credentials from the UI.
#[derive(Debug, Deserialize)]
struct SensitiveAuthParams {
    auth_password: Option<String>,
    auth_private_key: Option<String>,
}

// ── Shared helpers ───────────────────────────────────────────────────

/// Send a `ReportPluginConfig` message for each infra result that produced one.
///
/// Iterates `infra_results` and, for any result that carries a
/// `report_plugin_config`, constructs the wire payload and sends it via
/// `bg_tx`.  Results that refer to an existing config are logged at `info`
/// level instead.  Send failures are logged at `error` level.
async fn send_infra_plugin_reports(
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    host_id: uuid::Uuid,
    infra_results: &[uptrakit_plugin_infrastructure_registry::agent_infra::BootstrapInfraResult],
) {
    for infra in infra_results {
        if let Some(report) = &infra.report_plugin_config {
            let payload: uptrakit_internal_wire::ReportPluginConfigPayload =
                serde_json::from_value(json!({
                    "request_id": uuid::Uuid::now_v7().to_string(),
                    "plugin_type": report.plugin_type,
                    "name": report.name,
                    "config": report.config,
                }))
                .expect("ReportPluginConfigPayload JSON is always valid");
            let msg = ServiceMessage::ReportPluginConfig(payload);
            if bg_tx.send(msg).await.is_err() {
                tracing::error!("failed to send ReportPluginConfig via bg_tx");
            }
        } else if let Some(config_id) = &infra.existing_plugin_config_id {
            tracing::info!(
                %host_id,
                %config_id,
                "reusing existing plugin config for cluster node"
            );
        }
    }
}

fn classify_validation_failure(message: &str) -> bool {
    message.starts_with("missing required field")
        || message.starts_with("invalid target")
        || message == "no guests selected"
        || message.contains("no password provided")
        || message.contains("no private key provided")
        || message.contains("unknown auth_method")
}

fn classify_surface_mutation_outcome(
    interaction_id: &str,
    response: &SurfaceActionResponse,
) -> (&'static str, Option<&'static str>) {
    if response.success {
        if interaction_id == "bootstrap-proxmox-guest" {
            let failed = response
                .result
                .as_ref()
                .and_then(|result| result.get("failed"))
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            if failed > 0 {
                return ("partial", None);
            }
        }
        return ("success", None);
    }

    let message = response
        .error
        .as_ref()
        .map(|error| error.message.as_str())
        .unwrap_or("surface action failed");

    match interaction_id {
        "remove-host" => {
            if classify_validation_failure(message) {
                ("validation_failed", Some("invalid_request"))
            } else if message == "host not found" {
                ("denied", Some("host_not_found"))
            } else {
                ("failed", Some("storage_error"))
            }
        }
        "bootstrap-execute" => {
            if classify_validation_failure(message) {
                ("validation_failed", Some("invalid_request"))
            } else {
                ("failed", Some("bootstrap_failed"))
            }
        }
        "sync-execute" => {
            if classify_validation_failure(message) {
                ("validation_failed", Some("invalid_request"))
            } else if message == "host not found" {
                ("denied", Some("host_not_found"))
            } else {
                ("failed", Some("sync_failed"))
            }
        }
        "bootstrap-proxmox-guest" => {
            if classify_validation_failure(message) {
                ("validation_failed", Some("invalid_request"))
            } else {
                ("failed", Some("bootstrap_failed"))
            }
        }
        _ => ("failed", Some("unclassified_error")),
    }
}

fn surface_mutation_target_id(
    interaction_id: &str,
    params: &serde_json::Value,
    response: &SurfaceActionResponse,
) -> Option<String> {
    match interaction_id {
        "remove-host" | "sync-execute" => params
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        "bootstrap-execute" => response
            .result
            .as_ref()
            .and_then(|result| result.get("host_id"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        "bootstrap-proxmox-guest" => {
            let selected_guest_count = params
                .get("discovered_guests")
                .and_then(|value| value.as_array())
                .map_or(0, Vec::len);
            if selected_guest_count != 1 {
                return None;
            }
            let results = response
                .result
                .as_ref()
                .and_then(|result| result.get("results"))
                .and_then(|value| value.as_array())?;
            let successful: Vec<&serde_json::Value> = results
                .iter()
                .filter(|result| {
                    result.get("status").and_then(|value| value.as_str()) == Some("ok")
                })
                .collect();
            if successful.len() == 1 {
                successful[0]
                    .get("host_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn surface_mutation_target_display(
    interaction_id: &str,
    params: &serde_json::Value,
    response: &SurfaceActionResponse,
) -> Option<String> {
    if let Some(name) = params.get("name").and_then(|value| value.as_str()) {
        return Some(name.to_string());
    }

    match interaction_id {
        "remove-host" => params
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        "bootstrap-proxmox-guest" => {
            let selected_guest_count = params
                .get("discovered_guests")
                .and_then(|value| value.as_array())
                .map_or(0, Vec::len);
            if selected_guest_count == 0 {
                return None;
            }
            if selected_guest_count == 1
                && let Some(results) = response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("results"))
                    .and_then(|value| value.as_array())
                && let Some(name) = results
                    .iter()
                    .find(|result| {
                        result.get("status").and_then(|value| value.as_str()) == Some("ok")
                    })
                    .and_then(|result| result.get("name"))
                    .and_then(|value| value.as_str())
            {
                Some(name.to_string())
            } else {
                Some(format!("{selected_guest_count} guest(s)"))
            }
        }
        _ => None,
    }
}

fn build_surface_mutation_details(
    interaction_id: &str,
    params: &serde_json::Value,
    response: &SurfaceActionResponse,
    reason_code: Option<&'static str>,
) -> serde_json::Value {
    match interaction_id {
        "remove-host" => {
            let mut details = json!({
                "mutation_source": "ssh_surface.remove_host",
            });
            if let Some(host_id) = params.get("id").and_then(|value| value.as_str()) {
                details["host_id"] = json!(host_id);
            }
            if let Some(reason_code) = reason_code {
                details["reason_code"] = json!(reason_code);
            }
            details
        }
        "bootstrap-execute" => {
            let mut details = json!({
                "mutation_source": "ssh_surface.bootstrap_execute",
                "allow_all": param_bool(params, "allow_all"),
                "skip_action_count": parse_skip_actions(params).len(),
                "has_infrastructure": response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("has_infrastructure"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
            });
            if let Some(reason_code) = reason_code {
                details["reason_code"] = json!(reason_code);
            }
            details
        }
        "sync-execute" => {
            let mut details = json!({
                "mutation_source": "ssh_surface.sync_execute",
                "allow_all": param_bool(params, "allow_all"),
                "auth_method": params
                    .get("auth_method")
                    .and_then(|value| value.as_str())
                    .unwrap_or("stored"),
                "skip_action_count": parse_skip_actions(params).len(),
            });
            if let Some(host_id) = params.get("id").and_then(|value| value.as_str()) {
                details["host_id"] = json!(host_id);
            }
            if let Some(reason_code) = reason_code {
                details["reason_code"] = json!(reason_code);
            }
            details
        }
        "bootstrap-proxmox-guest" => {
            let mut details = json!({
                "mutation_source": "ssh_surface.bootstrap_proxmox_guest",
                "selected_guest_count": params
                    .get("discovered_guests")
                    .and_then(|value| value.as_array())
                    .map_or(0, Vec::len),
                "succeeded": response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("succeeded"))
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                "failed": response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("failed"))
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
            });
            if let Some(reason_code) = reason_code {
                details["reason_code"] = json!(reason_code);
            }
            details
        }
        _ => json!({}),
    }
}

async fn emit_surface_mutation_audit(
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    tenant_id: Option<uuid::Uuid>,
    interaction_id: &str,
    request_id: uuid::Uuid,
    params: &serde_json::Value,
    response: &SurfaceActionResponse,
) {
    let action_type = match interaction_id {
        "remove-host" => Some("host.deactivate"),
        "bootstrap-execute" | "sync-execute" | "bootstrap-proxmox-guest" => Some("host.update"),
        _ => None,
    };
    let Some(action_type) = action_type else {
        return;
    };

    let (outcome, reason_code) = classify_surface_mutation_outcome(interaction_id, response);
    let payload = AuditEventPayload {
        action_type: action_type.to_string(),
        tenant_id: tenant_id.map(|value| value.to_string()),
        target_type: Some("host".to_string()),
        target_id: surface_mutation_target_id(interaction_id, params, response),
        target_display: surface_mutation_target_display(interaction_id, params, response),
        outcome: outcome.to_string(),
        details_json: Some(
            build_surface_mutation_details(interaction_id, params, response, reason_code)
                .to_string(),
        ),
        request_id: Some(request_id.to_string()),
    };

    if bg_tx
        .send(ServiceMessage::AuditEvent(payload))
        .await
        .is_err()
    {
        tracing::warn!(
            action_id = interaction_id,
            "failed to enqueue surface mutation audit event"
        );
    }
}

/// Resolve `host_id`, decrypt sensitive params, and build the auth override.
///
/// This is the common setup for both `spawn_sync_connect` and
/// `spawn_sync_execute`. On any failure, a `SurfaceActionResponse` error is sent
/// via `bg_tx` and `None` is returned so the caller can bail early.
async fn resolve_sync_auth(
    params: &serde_json::Value,
    sensitive_params_sealed: Option<&str>,
    request_id: uuid::Uuid,
    private_key_der: Option<&[u8]>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) -> Option<(String, Option<sync::SyncAuthOverride>)> {
    let host_id = match params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let resp = make_surface_error_response(request_id, "missing required field 'id'");
            let _ = bg_tx
                .send(ServiceMessage::SurfaceActionResponse(resp))
                .await;
            return None;
        }
    };

    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            sensitive_params_sealed,
            private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => {
                let resp = make_surface_error_response(request_id, &msg);
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return None;
            }
        };

    let auth_override = match build_sync_auth_override(params, sensitive.as_ref()) {
        Ok(ov) => ov,
        Err(msg) => {
            let resp = make_surface_error_response(request_id, &msg);
            let _ = bg_tx
                .send(ServiceMessage::SurfaceActionResponse(resp))
                .await;
            return None;
        }
    };

    Some((host_id, auth_override))
}

// ── Helpers ──────────────────────────────────────────────────────────

fn make_surface_success_response(
    request_id: uuid::Uuid,
    data: serde_json::Value,
) -> SurfaceActionResponse {
    SurfaceActionResponse {
        request_id,
        success: true,
        result: Some(data),
        error: None,
    }
}

fn make_surface_error_response(request_id: uuid::Uuid, message: &str) -> SurfaceActionResponse {
    SurfaceActionResponse {
        request_id,
        success: false,
        result: None,
        error: Some(SurfaceActionError {
            code: SurfaceActionErrorCode::InvalidRequest,
            message: message.to_string(),
            details: None,
        }),
    }
}

async fn send_response(conn: &mut dyn ServiceTransport, response: SurfaceActionResponse) {
    if let Err(e) = conn
        .transport_send(ServiceMessage::SurfaceActionResponse(response))
        .await
    {
        tracing::error!(error = %e, "failed to send surface action response");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;

    use super::*;
    use sea_orm::{Database, DatabaseConnection};
    use uptrakit_internal_wire::{ControllerMessage, TransportClosePolicy, TransportError};

    fn test_catalog() -> uptrakit_plugin_infrastructure_registry::PluginCatalog {
        let config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        uptrakit_plugin_infrastructure_registry::build_catalog(&config)
            .expect("plugin catalog must build for tests")
    }

    #[test]
    fn surface_success_response_preserves_request_id_and_payload() {
        let request_id = uuid::Uuid::now_v7();
        let response = make_surface_success_response(request_id, json!({ "ok": true }));

        assert_eq!(response.request_id, request_id);
        assert!(response.success);
        assert_eq!(response.result, Some(json!({ "ok": true })));
        assert!(response.error.is_none());
    }

    #[test]
    fn surface_error_response_preserves_request_id_and_structured_error() {
        let request_id = uuid::Uuid::now_v7();
        let response = make_surface_error_response(request_id, "boom");

        assert_eq!(response.request_id, request_id);
        assert!(!response.success);
        assert!(response.result.is_none());
        let error = response.error.expect("error payload should be present");
        assert_eq!(error.code, SurfaceActionErrorCode::InvalidRequest);
        assert_eq!(error.message, "boom");
    }

    #[test]
    fn surface_registration_is_single_surface_and_tenant_bound() {
        let tenant_id = uuid::Uuid::now_v7();
        let registration = build_surface_registration(None, &test_catalog(), None, Some(tenant_id));

        assert_eq!(registration.surfaces.len(), 1);
        assert_eq!(
            registration.surfaces[0].descriptor.surface_id.as_str(),
            SSH_HOSTS_SURFACE_ID
        );
        assert_eq!(
            registration.effective_tenant_binding.scope,
            surfaces::Scope::Tenant
        );
        let tenant_id_str = tenant_id.to_string();
        assert_eq!(
            registration.effective_tenant_binding.tenant_id.as_deref(),
            Some(tenant_id_str.as_str())
        );
        assert!(
            registration
                .capabilities
                .0
                .contains(&surfaces::Capability::ProviderInitiatedActions),
            "provider-proxied ssh-agent surface actions must advertise provider_initiated_actions"
        );
    }

    #[test]
    fn ssh_hosts_surface_descriptor_and_data_source_parity_is_preserved() {
        let registration = build_surface_registration(None, &test_catalog(), None, None);
        let surface = registration
            .surfaces
            .iter()
            .find(|surface| surface.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
            .expect("ssh-agent.hosts surface is registered");

        assert_eq!(surface.descriptor.label, SSH_HOSTS_SURFACE_LABEL);
        assert_eq!(surface.descriptor.slot, surfaces::SLOT_SURFACE_PAGE);
        assert_eq!(surface.descriptor.priority, SSH_HOSTS_SURFACE_PRIORITY);
        assert_eq!(surface.descriptor.scope, surfaces::Scope::Tenant);
        assert_eq!(surface.descriptor.targeting, Targeting::Targeted);
        assert_eq!(
            surface.descriptor.required_permission.as_deref(),
            Some(Permission::UpdateHosts.as_str())
        );

        assert_eq!(surface.data_sources.len(), 1);
        let primary_data_source = &surface.data_sources[0];
        assert_eq!(primary_data_source.data_source_id.as_str(), "data.primary");
        assert_eq!(
            primary_data_source.kind,
            DataSourceKind::ProviderQuery {
                operation_id: SSH_HOSTS_DATA_ACTION_ID.to_string()
            }
        );
        assert_eq!(
            primary_data_source
                .pagination
                .as_ref()
                .map(|pagination| pagination.default_page_size),
            Some(SSH_HOSTS_DEFAULT_PER_PAGE as u16)
        );
        assert_eq!(
            primary_data_source
                .pagination
                .as_ref()
                .map(|pagination| pagination.max_page_size),
            Some(1000),
            "ssh-agent surface pagination should expose the full 1000-item limit"
        );
        assert_eq!(primary_data_source.refresh_policy, RefreshPolicy::Manual);

        let SurfaceNode::Section { children, .. } = &surface.descriptor.root_node else {
            panic!("root node should be a section");
        };
        let Some(SurfaceNode::Table {
            columns,
            row_actions,
            ..
        }) = children.first()
        else {
            panic!("first section child should be a table");
        };
        let actual_columns: Vec<(&str, &str)> = columns
            .iter()
            .map(|column| (column.key.as_str(), column.label.as_str()))
            .collect();
        assert_eq!(actual_columns, SSH_HOSTS_COLUMNS);
        let row_action_ids: Vec<&str> = row_actions
            .iter()
            .map(|action| action.interaction_id.as_str())
            .collect();
        assert_eq!(row_action_ids, SSH_HOSTS_ROW_ACTION_IDS);
    }

    #[test]
    fn dynamic_primary_action_is_included_in_action_bar_when_available() {
        let actions = build_actions();
        assert!(
            actions
                .iter()
                .any(|action| action.action_id == "bootstrap-proxmox-guest"),
            "expected infra action bootstrap-proxmox-guest to be present in action library"
        );

        let registration = build_surface_registration(None, &test_catalog(), None, None);
        let surface = registration
            .surfaces
            .iter()
            .find(|surface| surface.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
            .expect("ssh-agent.hosts surface is registered");

        let SurfaceNode::Section { children, .. } = &surface.descriptor.root_node else {
            panic!("root node should be a section");
        };
        let Some(SurfaceNode::ActionBar { action_ids }) = children.get(1) else {
            panic!("second section child should be an action bar");
        };
        let action_ids: BTreeSet<&str> = action_ids.iter().map(|id| id.as_str()).collect();
        assert!(action_ids.contains(SSH_HOSTS_PRIMARY_ACTION_ID));
        assert!(action_ids.contains("bootstrap-proxmox-guest"));
    }

    #[test]
    fn workflow_interactions_are_registered_with_truthful_steps() {
        let registration = build_surface_registration(None, &test_catalog(), None, None);
        let surface = registration
            .surfaces
            .iter()
            .find(|surface| surface.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
            .expect("ssh-agent.hosts surface is registered");

        let interactions: BTreeMap<&str, &InteractionDescriptor> = surface
            .interactions
            .iter()
            .map(|interaction| (interaction.interaction_id.as_str(), interaction))
            .collect();

        assert!(interactions.contains_key("list-hosts"));
        assert!(interactions.contains_key("remove-host"));

        let bootstrap = interactions
            .get("bootstrap")
            .copied()
            .expect("bootstrap workflow interaction is present");
        assert_eq!(bootstrap.kind, InteractionKind::Workflow);
        assert_eq!(bootstrap.workflow_steps.len(), 3);
        assert_eq!(bootstrap.workflow_steps[0].step_id, "connect");
        assert_eq!(
            bootstrap.workflow_steps[0]
                .submit_interaction_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("bootstrap-connect")
        );
        assert!(bootstrap.workflow_steps[1].render_previous_response);
        assert_eq!(
            bootstrap.workflow_steps[2]
                .submit_interaction_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("bootstrap-execute")
        );

        let sync_host = interactions
            .get("sync-host")
            .copied()
            .expect("sync-host workflow interaction is present");
        assert_eq!(sync_host.kind, InteractionKind::Workflow);
        assert_eq!(sync_host.workflow_steps.len(), 3);
        assert_eq!(sync_host.workflow_steps[0].step_id, "connect");
        assert_eq!(
            sync_host.workflow_steps[0]
                .submit_interaction_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("sync-connect")
        );
        assert!(sync_host.workflow_steps[1].render_previous_response);
        assert_eq!(
            sync_host.workflow_steps[2]
                .submit_interaction_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("sync-execute")
        );
    }

    #[test]
    fn workflow_step_submit_interactions_are_registered_for_dispatch() {
        let registration = build_surface_registration(None, &test_catalog(), None, None);
        let surface = registration
            .surfaces
            .iter()
            .find(|registered| registered.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
            .expect("ssh-agent.hosts surface is registered");
        let interaction_ids: BTreeSet<&str> = surface
            .interactions
            .iter()
            .map(|interaction| interaction.interaction_id.as_str())
            .collect();

        assert!(interaction_ids.contains("bootstrap-connect"));
        assert!(interaction_ids.contains("bootstrap-execute"));
        assert!(interaction_ids.contains("sync-connect"));
        assert!(interaction_ids.contains("sync-execute"));
    }

    #[derive(Default)]
    struct RecordingTransport {
        sent: Vec<ServiceMessage>,
    }

    #[async_trait]
    impl ServiceTransport for RecordingTransport {
        async fn transport_send(&mut self, msg: ServiceMessage) -> Result<(), TransportError> {
            self.sent.push(msg);
            Ok(())
        }

        async fn transport_send_best_effort(&mut self, msg: ServiceMessage) {
            self.sent.push(msg);
        }

        async fn transport_send_auto_paginate(
            &mut self,
            msg: ServiceMessage,
        ) -> Result<(), TransportError> {
            self.sent.push(msg);
            Ok(())
        }

        async fn transport_recv(&mut self) -> Option<ControllerMessage> {
            None
        }

        fn close_policy(&self) -> TransportClosePolicy {
            TransportClosePolicy::Reconnect { reason: None }
        }
    }

    #[tokio::test]
    async fn unregistered_interaction_is_rejected_before_infra_fallback() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory db");
        let state_dir = tempfile::tempdir().expect("tempdir");
        let (bg_tx, _bg_rx) = tokio::sync::mpsc::channel(4);
        let surface_proxy = Arc::new(uptrakit_service_sdk::ServiceSurfaceProxy::new());
        let infra_bundles = Arc::new(Vec::new());
        let ctx = SurfaceRuntimeContext {
            db: &db,
            state_dir: state_dir.path(),
            private_key_der: None,
            service_id: None,
            tenant_id: None,
            bg_tx: &bg_tx,
            surface_proxy: &surface_proxy,
            infra_bundles,
        };
        let request = SurfaceActionRequest {
            request_id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7().to_string(),
            surface_id: surfaces::SurfaceId::new(SSH_HOSTS_SURFACE_ID.to_string())
                .expect("surface id should be valid"),
            interaction_id: surfaces::InteractionId::new("non-registered-action".to_string())
                .expect("interaction id should be valid"),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "test".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        };
        let mut conn = RecordingTransport::default();

        handle_surface_action_request(request, &ctx, &mut conn).await;

        assert_eq!(conn.sent.len(), 1);
        let ServiceMessage::SurfaceActionResponse(response) = &conn.sent[0] else {
            panic!("expected surface action response");
        };
        assert!(!response.success);
        assert_eq!(
            response.error.as_ref().map(|error| error.message.as_str()),
            Some("unknown action")
        );
    }

    fn test_encrypted_key() -> uptrakit_crypto::EncryptedString {
        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x24u8; 32]));
        let _ = uptrakit_crypto::register_column_aad(&[uptrakit_crypto::ColumnAadEntry {
            table: "ssh_hosts",
            column: "private_key",
            aad: "uptrakit:ssh_hosts:private_key",
        }]);
        uptrakit_crypto::EncryptedString::new(
            "test-key-content".to_string(),
            "uptrakit:ssh_hosts:private_key",
        )
        .expect("master key initialized above")
    }

    async fn setup_surface_db() -> (tempfile::TempDir, DatabaseConnection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = crate::db::init_db(dir.path()).await.expect("init_db");
        (dir, db)
    }

    async fn insert_test_host(
        db: &DatabaseConnection,
        name: &str,
    ) -> crate::db::entity::ssh_host::Model {
        host_ops::add_host(
            db,
            host_ops::AddHostParams {
                host_id: uuid::Uuid::now_v7(),
                name: name.to_string(),
                hostname: format!("{name}.example.test"),
                port: 22,
                username: "root".to_string(),
                encrypted_key: test_encrypted_key(),
                key_type: crate::db::entity::ssh_host::SshKeyType::Ed25519,
                host_key_fingerprint: None,
            },
        )
        .await
        .expect("add host")
    }

    async fn recv_audit_event(
        bg_rx: &mut tokio::sync::mpsc::Receiver<ServiceMessage>,
    ) -> AuditEventPayload {
        let message = tokio::time::timeout(Duration::from_secs(1), bg_rx.recv())
            .await
            .expect("audit timeout")
            .expect("audit message");
        match message {
            ServiceMessage::AuditEvent(payload) => payload,
            other => panic!("expected audit event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn remove_host_success_emits_host_deactivate_audit_event() {
        let (state_dir, db) = setup_surface_db().await;
        let host = insert_test_host(&db, "removable").await;
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);
        let surface_proxy = Arc::new(uptrakit_service_sdk::ServiceSurfaceProxy::new());
        let infra_bundles = Arc::new(Vec::new());
        let tenant_id = uuid::Uuid::now_v7();
        let ctx = SurfaceRuntimeContext {
            db: &db,
            state_dir: state_dir.path(),
            private_key_der: None,
            service_id: None,
            tenant_id: Some(tenant_id),
            bg_tx: &bg_tx,
            surface_proxy: &surface_proxy,
            infra_bundles,
        };
        let request = SurfaceActionRequest {
            request_id: uuid::Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new(SSH_HOSTS_SURFACE_ID.to_string())
                .expect("surface id should be valid"),
            interaction_id: surfaces::InteractionId::new("remove-host".to_string())
                .expect("interaction id should be valid"),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "test".to_string(),
            },
            params: serde_json::Map::from_iter([
                ("id".to_string(), json!(host.id.to_string())),
                ("name".to_string(), json!("removable")),
            ]),
            encrypted_sensitive_params: None,
        };
        let mut conn = RecordingTransport::default();

        handle_surface_action_request(request, &ctx, &mut conn).await;

        let payload = recv_audit_event(&mut bg_rx).await;
        assert_eq!(payload.action_type, "host.deactivate");
        assert_eq!(
            payload.tenant_id.as_deref(),
            Some(tenant_id.to_string().as_str())
        );
        assert_eq!(
            payload.target_id.as_deref(),
            Some(host.id.to_string().as_str())
        );
        assert_eq!(payload.target_display.as_deref(), Some("removable"));
        assert_eq!(payload.outcome, "success");
        let details = serde_json::from_str::<serde_json::Value>(
            payload.details_json.as_deref().expect("details"),
        )
        .expect("valid details");
        assert_eq!(details["mutation_source"], json!("ssh_surface.remove_host"));
    }

    #[tokio::test]
    async fn remove_host_missing_host_emits_denied_audit_event() {
        let (state_dir, db) = setup_surface_db().await;
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);
        let surface_proxy = Arc::new(uptrakit_service_sdk::ServiceSurfaceProxy::new());
        let infra_bundles = Arc::new(Vec::new());
        let tenant_id = uuid::Uuid::now_v7();
        let ctx = SurfaceRuntimeContext {
            db: &db,
            state_dir: state_dir.path(),
            private_key_der: None,
            service_id: None,
            tenant_id: Some(tenant_id),
            bg_tx: &bg_tx,
            surface_proxy: &surface_proxy,
            infra_bundles,
        };
        let missing_id = uuid::Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: uuid::Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new(SSH_HOSTS_SURFACE_ID.to_string())
                .expect("surface id should be valid"),
            interaction_id: surfaces::InteractionId::new("remove-host".to_string())
                .expect("interaction id should be valid"),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "test".to_string(),
            },
            params: serde_json::Map::from_iter([("id".to_string(), json!(missing_id.to_string()))]),
            encrypted_sensitive_params: None,
        };
        let mut conn = RecordingTransport::default();

        handle_surface_action_request(request, &ctx, &mut conn).await;

        let payload = recv_audit_event(&mut bg_rx).await;
        assert_eq!(payload.action_type, "host.deactivate");
        assert_eq!(payload.outcome, "denied");
        let details = serde_json::from_str::<serde_json::Value>(
            payload.details_json.as_deref().expect("details"),
        )
        .expect("valid details");
        assert_eq!(details["reason_code"], json!("host_not_found"));
    }

    #[tokio::test]
    async fn bootstrap_execute_success_maps_to_host_update_audit_event() {
        let request_id = uuid::Uuid::now_v7();
        let host_id = uuid::Uuid::now_v7();
        let tenant_id = uuid::Uuid::now_v7();
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);

        emit_surface_mutation_audit(
            &bg_tx,
            Some(tenant_id),
            "bootstrap-execute",
            request_id,
            &json!({
                "name": "new-host",
                "allow_all": true,
                "skip_actions": ["precheck"],
            }),
            &make_surface_success_response(
                request_id,
                json!({
                    "host_id": host_id.to_string(),
                    "has_infrastructure": true,
                }),
            ),
        )
        .await;

        let payload = recv_audit_event(&mut bg_rx).await;
        assert_eq!(payload.action_type, "host.update");
        assert_eq!(payload.outcome, "success");
        assert_eq!(
            payload.target_id.as_deref(),
            Some(host_id.to_string().as_str())
        );
        let details = serde_json::from_str::<serde_json::Value>(
            payload.details_json.as_deref().expect("details"),
        )
        .expect("valid details");
        assert_eq!(
            details["mutation_source"],
            json!("ssh_surface.bootstrap_execute")
        );
        assert_eq!(details["has_infrastructure"], json!(true));
        assert_eq!(details["skip_action_count"], json!(1));
    }

    #[tokio::test]
    async fn bootstrap_execute_invalid_request_maps_to_validation_failed_audit_event() {
        let request_id = uuid::Uuid::now_v7();
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);

        emit_surface_mutation_audit(
            &bg_tx,
            None,
            "bootstrap-execute",
            request_id,
            &json!({}),
            &make_surface_error_response(request_id, "missing required field 'target'"),
        )
        .await;

        let payload = recv_audit_event(&mut bg_rx).await;
        assert_eq!(payload.action_type, "host.update");
        assert_eq!(payload.outcome, "validation_failed");
        let details = serde_json::from_str::<serde_json::Value>(
            payload.details_json.as_deref().expect("details"),
        )
        .expect("valid details");
        assert_eq!(details["reason_code"], json!("invalid_request"));
    }

    #[tokio::test]
    async fn sync_execute_success_maps_to_host_update_audit_event() {
        let request_id = uuid::Uuid::now_v7();
        let host_id = uuid::Uuid::now_v7();
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);

        emit_surface_mutation_audit(
            &bg_tx,
            None,
            "sync-execute",
            request_id,
            &json!({
                "id": host_id.to_string(),
                "auth_method": "stored",
                "skip_actions": ["refresh"],
            }),
            &make_surface_success_response(
                request_id,
                json!({
                    "summary": {
                        "updated": true,
                    }
                }),
            ),
        )
        .await;

        let payload = recv_audit_event(&mut bg_rx).await;
        assert_eq!(payload.action_type, "host.update");
        assert_eq!(payload.outcome, "success");
        assert_eq!(
            payload.target_id.as_deref(),
            Some(host_id.to_string().as_str())
        );
        let details = serde_json::from_str::<serde_json::Value>(
            payload.details_json.as_deref().expect("details"),
        )
        .expect("valid details");
        assert_eq!(
            details["mutation_source"],
            json!("ssh_surface.sync_execute")
        );
        assert_eq!(details["skip_action_count"], json!(1));
    }

    #[tokio::test]
    async fn sync_execute_missing_host_maps_to_denied_audit_event() {
        let request_id = uuid::Uuid::now_v7();
        let host_id = uuid::Uuid::now_v7();
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);

        emit_surface_mutation_audit(
            &bg_tx,
            None,
            "sync-execute",
            request_id,
            &json!({ "id": host_id.to_string() }),
            &make_surface_error_response(request_id, "host not found"),
        )
        .await;

        let payload = recv_audit_event(&mut bg_rx).await;
        assert_eq!(payload.action_type, "host.update");
        assert_eq!(payload.outcome, "denied");
        let details = serde_json::from_str::<serde_json::Value>(
            payload.details_json.as_deref().expect("details"),
        )
        .expect("valid details");
        assert_eq!(details["reason_code"], json!("host_not_found"));
    }

    #[tokio::test]
    async fn bootstrap_proxmox_guest_partial_success_maps_to_partial_audit_event() {
        let request_id = uuid::Uuid::now_v7();
        let host_id = uuid::Uuid::now_v7();
        let tenant_id = uuid::Uuid::now_v7();
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);

        emit_surface_mutation_audit(
            &bg_tx,
            Some(tenant_id),
            "bootstrap-proxmox-guest",
            request_id,
            &json!({
                "discovered_guests": ["guest-1", "guest-2"],
            }),
            &make_surface_success_response(
                request_id,
                json!({
                    "results": [
                        {
                            "mapping_id": "guest-1",
                            "name": "Guest One",
                            "host_id": host_id.to_string(),
                            "status": "ok",
                        },
                        {
                            "mapping_id": "guest-2",
                            "name": "Guest Two",
                            "status": "error",
                            "error": "bootstrap failed",
                        }
                    ],
                    "succeeded": 1,
                    "failed": 1,
                }),
            ),
        )
        .await;

        let payload = recv_audit_event(&mut bg_rx).await;
        assert_eq!(payload.action_type, "host.update");
        assert_eq!(payload.outcome, "partial");
        assert!(payload.target_id.is_none());
        assert_eq!(payload.target_display.as_deref(), Some("2 guest(s)"));
        let details = serde_json::from_str::<serde_json::Value>(
            payload.details_json.as_deref().expect("details"),
        )
        .expect("valid details");
        assert_eq!(
            details["mutation_source"],
            json!("ssh_surface.bootstrap_proxmox_guest")
        );
        assert_eq!(details["selected_guest_count"], json!(2));
        assert_eq!(details["succeeded"], json!(1));
        assert_eq!(details["failed"], json!(1));
    }

    #[tokio::test]
    async fn bootstrap_proxmox_guest_invalid_request_maps_to_validation_failed_audit_event() {
        let request_id = uuid::Uuid::now_v7();
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);

        emit_surface_mutation_audit(
            &bg_tx,
            None,
            "bootstrap-proxmox-guest",
            request_id,
            &json!({}),
            &make_surface_error_response(request_id, "no guests selected"),
        )
        .await;

        let payload = recv_audit_event(&mut bg_rx).await;
        assert_eq!(payload.action_type, "host.update");
        assert_eq!(payload.outcome, "validation_failed");
        let details = serde_json::from_str::<serde_json::Value>(
            payload.details_json.as_deref().expect("details"),
        )
        .expect("valid details");
        assert_eq!(details["reason_code"], json!("invalid_request"));
    }
}
