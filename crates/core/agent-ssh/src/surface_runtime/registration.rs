use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

mod action_library;
mod capabilities;
mod form_adapters;
mod interactions;

use uptrakit_internal_wire::surfaces::{
    self, CapabilitySet, DataSourceDescriptor, DataSourceId, DataSourceKind, FrameworkGeneration,
    InteractionId, ProviderEncryptionAlgorithm, ProviderEncryptionMetadata, RefreshPolicy,
    SurfaceDescriptor, SurfaceNode, SurfaceRegistration, SurfaceTableColumn, SurfaceTableRowAction,
    Targeting,
};
use uptrakit_plugin_infrastructure_registry::{SurfaceActionDescriptor, SurfaceActionUi};
use uptrakit_shared_types::Permission;

use super::{
    SSH_HOSTS_COLUMNS, SSH_HOSTS_DATA_ACTION_ID, SSH_HOSTS_DEFAULT_PER_PAGE,
    SSH_HOSTS_ROW_ACTION_IDS, SSH_HOSTS_SURFACE_ID, SSH_HOSTS_SURFACE_LABEL,
    SSH_HOSTS_SURFACE_PRIORITY,
};
use action_library::{build_primary_actions, collect_infra_primary_actions};
use capabilities::compute_required_capabilities;
use form_adapters::{
    InteractionHint, InteractionRef, action_ui_to_form_ui, collect_select_source_action_refs,
    collect_workflow_step_refs, row_visible_when_from_extension, sensitive_fields_for_action,
};
use interactions::build_interactions;

pub use action_library::build_actions;

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
                    .map(|(key, label)| SurfaceTableColumn::new(*key, *label))
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
