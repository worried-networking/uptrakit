use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use uptrakit_internal_wire::surfaces::{
    self, CapabilitySet, DataSourceDescriptor, DataSourceId, DataSourceKind, FormFieldDescriptor,
    FormSelectOption, FormUiDescriptor, FrameworkGeneration, InteractionDescriptor, InteractionId,
    InteractionKind, InteractionTransport, ProviderEncryptionAlgorithm, ProviderEncryptionMetadata,
    RefreshPolicy, SurfaceDescriptor, SurfaceNode, SurfaceRegistration, SurfaceTableColumn,
    SurfaceTableRowAction, Targeting,
};
use uptrakit_plugin_infrastructure_registry::{
    FormFieldDescriptor as PluginFormFieldDescriptor, FormFieldType as PluginFormFieldType,
    FormSelectOptionDescriptor as PluginFormSelectOptionDescriptor,
    FormSelectSourceDescriptor as PluginFormSelectSourceDescriptor, PluginFamily,
    SurfaceActionDescriptor, SurfaceActionUi, SurfaceFormDescriptor as PluginSurfaceFormDescriptor,
    SurfaceRowCondition as PluginSurfaceRowCondition,
    SurfaceRowVisibleWhen as PluginSurfaceRowVisibleWhen,
    SurfaceWorkflowStep as PluginSurfaceWorkflowStep, all_descriptors,
};
use uptrakit_shared_types::Permission;

use super::{
    SSH_HOSTS_COLUMNS, SSH_HOSTS_DATA_ACTION_ID, SSH_HOSTS_DEFAULT_PER_PAGE,
    SSH_HOSTS_PRIMARY_ACTION_ID, SSH_HOSTS_ROW_ACTION_IDS, SSH_HOSTS_SURFACE_ID,
    SSH_HOSTS_SURFACE_LABEL, SSH_HOSTS_SURFACE_PRIORITY,
};

static REGISTERED_INTERACTION_IDS: LazyLock<BTreeSet<String>> =
    LazyLock::new(collect_registered_interaction_ids);

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

pub fn build_actions() -> Vec<SurfaceActionDescriptor> {
    let mut actions = vec![
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
    let infra_actions: Vec<SurfaceActionDescriptor> = all_descriptors()
        .iter()
        .filter(|d| d.family == PluginFamily::Infrastructure)
        .filter_map(|d| d.surface_actions)
        .flat_map(|surface_actions| (surface_actions.actions)())
        .collect();
    actions.extend(infra_actions);
    actions
}

pub(crate) fn is_registered_interaction(action_id: &str) -> bool {
    REGISTERED_INTERACTION_IDS.contains(action_id)
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

fn collect_infra_primary_actions() -> Vec<String> {
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
        descriptor: SurfaceDescriptor {
            surface_id,
            label: SSH_HOSTS_SURFACE_LABEL.to_string(),
            priority,
            slot,
            scope: surfaces::Scope::Tenant,
            targeting,
            required_permission: Some(Permission::UpdateHosts.to_string()),
            provider_kind: surfaces::ProviderKind::Service,
            required_capabilities,
            root_node,
        },
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
        let action = action_index.get(action_id.as_str()).copied();
        let kind = match hint {
            InteractionHint::DataLoad => InteractionKind::DataLoad,
            InteractionHint::Action => action_kind_for_action(action),
        };
        let confirmation = if kind == InteractionKind::ConfirmableAction {
            Some(surfaces::InteractionConfirmation {
                title: format!(
                    "Confirm {}",
                    action
                        .map(|a| a.label.as_str())
                        .unwrap_or(action_id.as_str())
                ),
                message: "This action may modify existing data.".to_string(),
                confirm_label: None,
                cancel_label: None,
                severity: surfaces::ConfirmationSeverity::Danger,
            })
        } else {
            None
        };

        let timeout_seconds = action
            .and_then(|value| value.timeout_seconds)
            .map(|seconds| seconds.clamp(1, 300) as u16);
        let workflow_steps = match kind {
            InteractionKind::Workflow => workflow_steps_from_action(action),
            _ => vec![],
        };

        interactions.push(InteractionDescriptor {
            interaction_id,
            kind,
            label: action.map(|value| value.label.clone()),
            required_permission: action.and_then(|value| permission_or_none(&value.permission)),
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
            label: Some(step.label.clone()),
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
