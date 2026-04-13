use std::collections::{BTreeSet, HashMap};

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ExtensionManifest, ExtensionPlacement, ExtensionTargeting, ExtensionUi,
    PanelPosition,
};
use uptrakit_internal_wire::surfaces;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionHint {
    DataLoad,
    Action,
}

pub fn build_plugin_surface_registrations_from_extensions(
    plugin_type_id: &str,
    manifests: Vec<ExtensionManifest>,
    actions: Vec<ActionDef>,
) -> Vec<surfaces::SurfaceRegistration> {
    if manifests.is_empty() {
        return vec![];
    }

    let action_index: HashMap<&str, &ActionDef> = actions
        .iter()
        .map(|action| (action.action_id.as_str(), action))
        .collect();

    let registered_surfaces: Vec<surfaces::RegisteredSurface> = manifests
        .into_iter()
        .filter_map(|manifest| build_registered_surface(manifest, &action_index))
        .collect();

    if registered_surfaces.is_empty() {
        return vec![];
    }

    let mut registration_caps = BTreeSet::new();
    for surface in &registered_surfaces {
        registration_caps.extend(surface.descriptor.required_capabilities.0.iter().cloned());
    }

    vec![surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: format!("plugin.{plugin_type_id}"),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet(registration_caps),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: registered_surfaces,
        encryption_metadata: None,
    }]
}

fn build_registered_surface(
    manifest: ExtensionManifest,
    action_index: &HashMap<&str, &ActionDef>,
) -> Option<surfaces::RegisteredSurface> {
    let surface_id = surfaces::SurfaceId::new(manifest.id.clone()).ok()?;
    let slot = slot_for_manifest(&manifest).to_string();
    let priority = surfaces::slot_def(slot.as_str()).map_or(manifest.priority, |slot_def| {
        manifest.priority.clamp(
            slot_def.provider_priority_min,
            slot_def.provider_priority_max,
        )
    });

    let (root_node, data_sources, interaction_refs) =
        build_surface_contract_parts(&manifest.id, &manifest.ui);
    let interactions = build_interactions(&interaction_refs, action_index);
    let targeting = targeting_for_manifest(&manifest.targeting);
    let required_capabilities =
        compute_required_capabilities(&root_node, &targeting, &interactions, &data_sources);

    Some(surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor {
            surface_id,
            label: manifest.label,
            priority,
            slot,
            scope: surfaces::Scope::Global,
            targeting,
            required_permission: permission_or_none(&manifest.required_permission),
            provider_kind: surfaces::ProviderKind::Plugin,
            required_capabilities,
            root_node,
        },
        interactions,
        data_sources,
    })
}

fn build_surface_contract_parts(
    manifest_id: &str,
    ui: &ExtensionUi,
) -> (
    surfaces::SurfaceNode,
    Vec<surfaces::DataSourceDescriptor>,
    Vec<(String, InteractionHint)>,
) {
    match ui {
        ExtensionUi::DataTable {
            data_action,
            row_actions,
            primary_actions,
            context_selector,
            ..
        } => {
            let mut refs = Vec::new();
            refs.push((data_action.clone(), InteractionHint::DataLoad));
            refs.extend(
                row_actions
                    .iter()
                    .cloned()
                    .map(|id| (id, InteractionHint::Action)),
            );
            refs.extend(
                primary_actions
                    .iter()
                    .cloned()
                    .map(|id| (id, InteractionHint::Action)),
            );
            if let Some(selector) = context_selector.as_ref()
                && let Some(add_action) = selector.add_action.as_ref()
            {
                refs.push((add_action.clone(), InteractionHint::Action));
            }

            let data_source_id = surfaces::DataSourceId::new(format!("{manifest_id}.table_data"));
            let data_sources = data_source_id
                .as_ref()
                .ok()
                .map(|id| {
                    vec![surfaces::DataSourceDescriptor {
                        data_source_id: id.clone(),
                        kind: surfaces::DataSourceKind::Static {
                            data: serde_json::json!([]),
                        },
                        result_schema: surfaces::SchemaContract::Array,
                        pagination: None,
                        sorting: None,
                        filtering: None,
                        refresh_policy: surfaces::RefreshPolicy::Manual,
                        empty_state: None,
                    }]
                })
                .unwrap_or_default();

            let action_ids: Vec<surfaces::InteractionId> = primary_actions
                .iter()
                .chain(row_actions.iter())
                .filter_map(|id| surfaces::InteractionId::new(id.clone()).ok())
                .collect();

            let root_node = if let Ok(data_source_id) = data_source_id {
                if action_ids.is_empty() {
                    surfaces::SurfaceNode::Table { data_source_id }
                } else {
                    surfaces::SurfaceNode::Section {
                        title: None,
                        children: vec![
                            surfaces::SurfaceNode::Table { data_source_id },
                            surfaces::SurfaceNode::ActionBar { action_ids },
                        ],
                    }
                }
            } else {
                text_fallback_node("Surface data source is unavailable.")
            };

            (root_node, data_sources, refs)
        }
        ExtensionUi::KeyValue { data_action } => {
            let refs = vec![(data_action.clone(), InteractionHint::DataLoad)];
            let data_source_id =
                surfaces::DataSourceId::new(format!("{manifest_id}.key_value_data"));
            let data_sources = data_source_id
                .as_ref()
                .ok()
                .map(|id| {
                    vec![surfaces::DataSourceDescriptor {
                        data_source_id: id.clone(),
                        kind: surfaces::DataSourceKind::Static {
                            data: serde_json::json!({}),
                        },
                        result_schema: surfaces::SchemaContract::Object,
                        pagination: None,
                        sorting: None,
                        filtering: None,
                        refresh_policy: surfaces::RefreshPolicy::Manual,
                        empty_state: None,
                    }]
                })
                .unwrap_or_default();
            let root_node = data_source_id
                .map(|id| surfaces::SurfaceNode::KeyValue { data_source_id: id })
                .unwrap_or_else(|_| text_fallback_node("Surface data source is unavailable."));
            (root_node, data_sources, refs)
        }
        ExtensionUi::Actions { actions } => {
            let refs = actions
                .iter()
                .cloned()
                .map(|id| (id, InteractionHint::Action))
                .collect::<Vec<_>>();
            let action_ids = actions
                .iter()
                .filter_map(|id| surfaces::InteractionId::new(id.clone()).ok())
                .collect::<Vec<_>>();
            let root_node = if action_ids.is_empty() {
                text_fallback_node("No actions are available for this surface.")
            } else {
                surfaces::SurfaceNode::ActionBar { action_ids }
            };
            (root_node, vec![], refs)
        }
        ExtensionUi::Form(form) => {
            let mut refs = Vec::new();
            if let Some(pre_load) = form.pre_load_action.as_ref() {
                refs.push((pre_load.clone(), InteractionHint::DataLoad));
            }
            refs.extend(
                form.footer_actions
                    .iter()
                    .cloned()
                    .map(|id| (id, InteractionHint::Action)),
            );
            let action_ids = form
                .footer_actions
                .iter()
                .filter_map(|id| surfaces::InteractionId::new(id.clone()).ok())
                .collect::<Vec<_>>();
            let root_node = if action_ids.is_empty() {
                text_fallback_node("This surface is backed by a form contract.")
            } else {
                surfaces::SurfaceNode::ActionBar { action_ids }
            };
            (root_node, vec![], refs)
        }
        _ => (
            text_fallback_node("This extension UI is not yet mapped to the surface runtime."),
            vec![],
            vec![],
        ),
    }
}

fn build_interactions(
    interaction_refs: &[(String, InteractionHint)],
    action_index: &HashMap<&str, &ActionDef>,
) -> Vec<surfaces::InteractionDescriptor> {
    let mut seen = BTreeSet::new();
    let mut interactions = Vec::new();
    for (action_id, hint) in interaction_refs {
        if !seen.insert(action_id.clone()) {
            continue;
        }
        let Some(interaction_id) = surfaces::InteractionId::new(action_id.clone()).ok() else {
            continue;
        };
        let action = action_index.get(action_id.as_str()).copied();
        let kind = match hint {
            InteractionHint::DataLoad => surfaces::InteractionKind::DataLoad,
            InteractionHint::Action => action_kind_for_action(action),
        };
        let confirmation = if kind == surfaces::InteractionKind::ConfirmableAction {
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

        interactions.push(surfaces::InteractionDescriptor {
            interaction_id,
            kind,
            required_permission: action.and_then(|value| permission_or_none(&value.permission)),
            input_schema: action.and_then(|value| {
                if value.ui.is_some() || value.api_submit.is_some() {
                    Some(surfaces::SchemaContract::Object)
                } else {
                    None
                }
            }),
            result_schema: Some(surfaces::SchemaContract::Any),
            sensitive_fields: vec![],
            timeout_seconds,
            confirmation,
            transport: surfaces::InteractionTransport::ControllerLocal,
            workflow_steps: vec![],
        });
    }
    interactions
}

fn action_kind_for_action(action: Option<&ActionDef>) -> surfaces::InteractionKind {
    let Some(action) = action else {
        return surfaces::InteractionKind::MutationAction;
    };
    if action.destructive && action.confirm_entity_field.is_some() {
        return surfaces::InteractionKind::ConfirmableAction;
    }
    if matches!(
        action.ui.as_ref(),
        Some(ActionUi::Form(_) | ActionUi::Wizard { .. })
    ) {
        return surfaces::InteractionKind::FormSubmit;
    }
    surfaces::InteractionKind::MutationAction
}

fn compute_required_capabilities(
    root_node: &surfaces::SurfaceNode,
    targeting: &surfaces::Targeting,
    interactions: &[surfaces::InteractionDescriptor],
    data_sources: &[surfaces::DataSourceDescriptor],
) -> surfaces::CapabilitySet {
    let mut caps = BTreeSet::new();
    collect_node_capabilities(root_node, &mut caps);
    match targeting {
        surfaces::Targeting::Universal => {
            caps.insert(surfaces::Capability::UniversalTargeting);
        }
        surfaces::Targeting::Targeted => {
            caps.insert(surfaces::Capability::TargetedTargeting);
        }
    }
    for interaction in interactions {
        match interaction.kind {
            surfaces::InteractionKind::MutationAction => {
                caps.insert(surfaces::Capability::MutationAction);
            }
            surfaces::InteractionKind::FormSubmit => {
                caps.insert(surfaces::Capability::FormSubmit);
            }
            surfaces::InteractionKind::Workflow => {
                caps.insert(surfaces::Capability::Workflow);
            }
            surfaces::InteractionKind::Navigate => {
                caps.insert(surfaces::Capability::Navigate);
            }
            surfaces::InteractionKind::DataLoad => {
                caps.insert(surfaces::Capability::DataLoad);
            }
            surfaces::InteractionKind::ConfirmableAction => {
                caps.insert(surfaces::Capability::ConfirmableAction);
            }
        }
    }
    for data_source in data_sources {
        match &data_source.kind {
            surfaces::DataSourceKind::Static { .. } => {
                caps.insert(surfaces::Capability::StaticDataSource);
            }
            surfaces::DataSourceKind::ControllerQuery { .. } => {
                caps.insert(surfaces::Capability::ControllerQueryDataSource);
            }
            surfaces::DataSourceKind::ProviderQuery { .. } => {
                caps.insert(surfaces::Capability::ProviderQueryDataSource);
            }
        }
    }
    surfaces::CapabilitySet(caps)
}

fn collect_node_capabilities(
    node: &surfaces::SurfaceNode,
    out: &mut BTreeSet<surfaces::Capability>,
) {
    match node {
        surfaces::SurfaceNode::Section { children, .. } => {
            out.insert(surfaces::Capability::SectionNode);
            for child in children {
                collect_node_capabilities(child, out);
            }
        }
        surfaces::SurfaceNode::TextBlock { .. } => {
            out.insert(surfaces::Capability::TextBlockNode);
        }
        surfaces::SurfaceNode::KeyValue { .. } => {
            out.insert(surfaces::Capability::KeyValueNode);
        }
        surfaces::SurfaceNode::Table { .. } => {
            out.insert(surfaces::Capability::TableNode);
        }
        surfaces::SurfaceNode::Form { .. } => {
            out.insert(surfaces::Capability::FormNode);
        }
        surfaces::SurfaceNode::ActionBar { .. } => {
            out.insert(surfaces::Capability::ActionBarNode);
        }
        surfaces::SurfaceNode::Tabs { tabs } => {
            out.insert(surfaces::Capability::TabsNode);
            for tab in tabs {
                collect_node_capabilities(&tab.root, out);
            }
        }
        surfaces::SurfaceNode::Callout { .. } => {
            out.insert(surfaces::Capability::CalloutNode);
        }
        surfaces::SurfaceNode::EmptyState { .. } => {
            out.insert(surfaces::Capability::EmptyStateNode);
        }
        surfaces::SurfaceNode::ModalTrigger { modal_nodes, .. } => {
            out.insert(surfaces::Capability::ModalTriggerNode);
            for node in modal_nodes {
                collect_node_capabilities(node, out);
            }
        }
        surfaces::SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
            out.insert(surfaces::Capability::WorkflowTriggerNode);
            for node in step_nodes {
                collect_node_capabilities(node, out);
            }
        }
    }
}

fn slot_for_manifest(manifest: &ExtensionManifest) -> &'static str {
    match &manifest.placement {
        ExtensionPlacement::Page { .. } => surfaces::SLOT_EXTENSION_PAGE,
        ExtensionPlacement::Panel {
            target_page,
            position,
            ..
        } => {
            if target_page == "global-settings" {
                return surfaces::SLOT_SETTINGS_BELOW_GLOBAL;
            }
            if target_page == "settings" && matches!(position, PanelPosition::Tab) {
                return surfaces::SLOT_SETTINGS_TABS;
            }
            surfaces::SLOT_SOFTWARE_TABS
        }
        ExtensionPlacement::ContextMenuGroup { target_entity, .. } => {
            if target_entity == "software-item-host" {
                surfaces::SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU
            } else {
                surfaces::SLOT_SOFTWARE_TABS
            }
        }
        ExtensionPlacement::TableColumns { .. } => surfaces::SLOT_SOFTWARE_TABS,
        _ => surfaces::SLOT_EXTENSION_PAGE,
    }
}

fn targeting_for_manifest(targeting: &ExtensionTargeting) -> surfaces::Targeting {
    match targeting {
        // Compiled-in plugin providers currently register globally. Tenant-targeted
        // dispatch remains on the legacy extension runtime until service providers
        // are migrated.
        ExtensionTargeting::Targeted => surfaces::Targeting::Universal,
        ExtensionTargeting::Universal | ExtensionTargeting::Other(_) => {
            surfaces::Targeting::Universal
        }
        _ => surfaces::Targeting::Universal,
    }
}

fn text_fallback_node(text: &str) -> surfaces::SurfaceNode {
    surfaces::SurfaceNode::TextBlock {
        text: text.to_string(),
    }
}

fn permission_or_none(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}
