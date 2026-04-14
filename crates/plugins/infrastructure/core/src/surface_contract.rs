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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionDisposition {
    Immediate,
    Form,
    Unsupported,
}

#[derive(Debug, Clone)]
struct InteractionRef {
    action_id: String,
    hint: InteractionHint,
    sensitive_fields: Vec<String>,
    form_ui: Option<surfaces::FormUiDescriptor>,
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
    let slot = slot_for_manifest(&manifest)?.to_string();
    let priority = surfaces::slot_def(slot.as_str()).map_or(manifest.priority, |slot_def| {
        manifest.priority.clamp(
            slot_def.provider_priority_min,
            slot_def.provider_priority_max,
        )
    });

    let (root_node, data_sources, interaction_refs) =
        build_surface_contract_parts(&manifest, action_index)?;
    let interactions = build_interactions(manifest.id.as_str(), &interaction_refs, action_index);
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
    manifest: &ExtensionManifest,
    action_index: &HashMap<&str, &ActionDef>,
) -> Option<(
    surfaces::SurfaceNode,
    Vec<surfaces::DataSourceDescriptor>,
    Vec<InteractionRef>,
)> {
    match &manifest.ui {
        ExtensionUi::DataTable {
            columns,
            data_action,
            row_actions,
            primary_actions,
            context_selector,
            default_per_page,
            ..
        } => {
            if context_selector.is_none() {
                let data_source_id = surfaces::DataSourceId::new("data.primary").ok()?;
                let mut refs = vec![InteractionRef {
                    action_id: data_action.clone(),
                    hint: InteractionHint::DataLoad,
                    sensitive_fields: vec![],
                    form_ui: None,
                }];
                let mut primary_action_ids = Vec::new();
                let mut table_row_actions = Vec::new();

                for action_id in primary_actions {
                    let Some(action) = action_index.get(action_id.as_str()).copied() else {
                        continue;
                    };
                    match action_disposition(manifest.id.as_str(), action_id.as_str(), action) {
                        ActionDisposition::Immediate | ActionDisposition::Form => {
                            let Some(interaction_id) =
                                surfaces::InteractionId::new(action_id.clone()).ok()
                            else {
                                continue;
                            };
                            primary_action_ids.push(interaction_id);
                            refs.push(InteractionRef {
                                action_id: action_id.clone(),
                                hint: InteractionHint::Action,
                                sensitive_fields: vec![],
                                form_ui: action
                                    .ui
                                    .as_ref()
                                    .and_then(surface_form_ui_from_action_ui),
                            });
                            if let Some(pre_load_action) =
                                action.ui.as_ref().and_then(pre_load_action_for_action_ui)
                            {
                                refs.push(InteractionRef {
                                    action_id: pre_load_action.to_string(),
                                    hint: InteractionHint::DataLoad,
                                    sensitive_fields: vec![],
                                    form_ui: None,
                                });
                            }
                        }
                        ActionDisposition::Unsupported => {}
                    }
                }

                for action_id in row_actions {
                    let Some(action) = action_index.get(action_id.as_str()).copied() else {
                        continue;
                    };
                    match action_disposition(manifest.id.as_str(), action_id.as_str(), action) {
                        ActionDisposition::Immediate | ActionDisposition::Form => {
                            let Some(interaction_id) =
                                surfaces::InteractionId::new(action_id.clone()).ok()
                            else {
                                continue;
                            };
                            table_row_actions.push(surfaces::SurfaceTableRowAction {
                                interaction_id,
                                visible_when: action
                                    .row_visible_when
                                    .as_ref()
                                    .map(surface_row_visible_when_from_extension),
                            });
                            refs.push(InteractionRef {
                                action_id: action_id.clone(),
                                hint: InteractionHint::Action,
                                sensitive_fields: vec![],
                                form_ui: action
                                    .ui
                                    .as_ref()
                                    .and_then(surface_form_ui_from_action_ui),
                            });
                            if let Some(pre_load_action) =
                                action.ui.as_ref().and_then(pre_load_action_for_action_ui)
                            {
                                refs.push(InteractionRef {
                                    action_id: pre_load_action.to_string(),
                                    hint: InteractionHint::DataLoad,
                                    sensitive_fields: vec![],
                                    form_ui: None,
                                });
                            }
                        }
                        ActionDisposition::Unsupported => {}
                    }
                }

                let root_node = surfaces::SurfaceNode::Section {
                    title: None,
                    children: {
                        let mut children = Vec::new();
                        if !primary_action_ids.is_empty() {
                            children.push(surfaces::SurfaceNode::ActionBar {
                                action_ids: primary_action_ids,
                            });
                        }
                        children.push(surfaces::SurfaceNode::Table {
                            data_source_id: data_source_id.clone(),
                            columns: columns
                                .iter()
                                .map(|column| surfaces::SurfaceTableColumn {
                                    key: column.key.clone(),
                                    label: column.label.clone(),
                                })
                                .collect(),
                            row_actions: table_row_actions,
                        });
                        children
                    },
                };
                let default_page_size = default_per_page
                    .as_ref()
                    .copied()
                    .unwrap_or(20)
                    .clamp(1, 200) as u16;
                let data_sources = vec![surfaces::DataSourceDescriptor {
                    data_source_id,
                    kind: surfaces::DataSourceKind::ProviderQuery {
                        operation_id: data_action.clone(),
                    },
                    result_schema: surfaces::SchemaContract::Array,
                    pagination: Some(surfaces::DataSourcePagination {
                        default_page_size,
                        max_page_size: 200,
                    }),
                    sorting: None,
                    filtering: None,
                    refresh_policy: surfaces::RefreshPolicy::Manual,
                    empty_state: None,
                }];
                return Some((root_node, data_sources, refs));
            }

            let mut action_order = Vec::new();
            action_order.extend(primary_actions.iter().cloned());
            action_order.extend(row_actions.iter().cloned());
            if let Some(selector) = context_selector.as_ref()
                && let Some(add_action) = selector.add_action.as_ref()
            {
                action_order.push(add_action.clone());
            }
            dedupe_preserve_order(&mut action_order);

            let mut refs = Vec::new();
            let mut action_ids = Vec::new();
            let mut has_unsupported_actions = false;

            for action_id in action_order {
                let Some(action) = action_index.get(action_id.as_str()).copied() else {
                    has_unsupported_actions = true;
                    continue;
                };
                if action.api_submit.is_some()
                    && !is_allowlisted_controller_local_api_submit_action(
                        manifest.id.as_str(),
                        action_id.as_str(),
                    )
                {
                    has_unsupported_actions = true;
                    continue;
                }
                if let Some(interaction_id) = surfaces::InteractionId::new(action_id.clone()).ok() {
                    action_ids.push(interaction_id);
                    refs.push(InteractionRef {
                        action_id,
                        hint: InteractionHint::Action,
                        sensitive_fields: vec![],
                        form_ui: None,
                    });
                }
            }

            let action_forms_node = if action_ids.len() == 1 {
                surfaces::SurfaceNode::Form {
                    interaction_id: action_ids[0].clone(),
                }
            } else {
                let tabs = action_ids
                    .iter()
                    .filter_map(|interaction_id| {
                        let tab_id = surfaces::SurfaceTabId::new(format!(
                            "action.{}",
                            interaction_id.as_str()
                        ))
                        .ok()?;
                        let label = action_index
                            .get(interaction_id.as_str())
                            .map(|action| action.label.clone())
                            .unwrap_or_else(|| interaction_id.to_string());
                        Some(surfaces::SurfaceTab {
                            id: tab_id,
                            label,
                            root: surfaces::SurfaceNode::Form {
                                interaction_id: interaction_id.clone(),
                            },
                        })
                    })
                    .collect::<Vec<_>>();
                if tabs.is_empty() {
                    if has_unsupported_actions {
                        text_fallback_node("No runnable actions are available for this surface.")
                    } else {
                        text_fallback_node("No actions are available for this surface.")
                    }
                } else {
                    surfaces::SurfaceNode::Tabs { tabs }
                }
            };

            let root_node = surfaces::SurfaceNode::Section {
                title: None,
                children: vec![
                    surfaces::SurfaceNode::Callout {
                        level: surfaces::CalloutLevel::Info,
                        text: "Tabular plugin data is not yet hydrated in the shared-surface read runtime. Action forms remain available; include required row/context fields in the JSON payload when applicable.".to_string(),
                    },
                    action_forms_node,
                ],
            };
            Some((root_node, vec![], refs))
        }
        ExtensionUi::KeyValue { data_action } => {
            if !matches!(
                manifest.placement,
                ExtensionPlacement::Panel {
                    ref target_page,
                    ref position,
                    ..
                } if target_page == "host-detail" && matches!(position, PanelPosition::Tab)
            ) {
                return None;
            }
            let data_source_id = surfaces::DataSourceId::new("data.primary").ok()?;
            let root_node = surfaces::SurfaceNode::KeyValue {
                data_source_id: data_source_id.clone(),
            };
            let data_sources = vec![surfaces::DataSourceDescriptor {
                data_source_id,
                kind: surfaces::DataSourceKind::ProviderQuery {
                    operation_id: data_action.clone(),
                },
                result_schema: surfaces::SchemaContract::Object,
                pagination: None,
                sorting: None,
                filtering: None,
                refresh_policy: surfaces::RefreshPolicy::Manual,
                empty_state: None,
            }];
            let refs = vec![InteractionRef {
                action_id: data_action.clone(),
                hint: InteractionHint::DataLoad,
                sensitive_fields: vec![],
                form_ui: None,
            }];
            Some((root_node, data_sources, refs))
        }
        ExtensionUi::Actions { actions } => {
            let mut runnable_action_ids = Vec::new();
            let mut refs = Vec::new();
            let mut single_form_action_id: Option<String> = None;
            let mut has_unsupported_actions = false;

            for action_id in actions {
                let Some(action) = action_index.get(action_id.as_str()).copied() else {
                    has_unsupported_actions = true;
                    continue;
                };

                match action_disposition(manifest.id.as_str(), action_id.as_str(), action) {
                    ActionDisposition::Immediate => {
                        if let Some(interaction_id) =
                            surfaces::InteractionId::new(action_id.clone()).ok()
                        {
                            runnable_action_ids.push(interaction_id);
                            refs.push(InteractionRef {
                                action_id: action_id.clone(),
                                hint: InteractionHint::Action,
                                sensitive_fields: vec![],
                                form_ui: None,
                            });
                        }
                    }
                    ActionDisposition::Form => {
                        if actions.len() == 1 {
                            single_form_action_id = Some(action_id.clone());
                            let form_ui =
                                action.ui.as_ref().and_then(surface_form_ui_from_action_ui);
                            refs.push(InteractionRef {
                                action_id: action_id.clone(),
                                hint: InteractionHint::Action,
                                sensitive_fields: vec![],
                                form_ui,
                            });
                            if let Some(pre_load_action) =
                                action.ui.as_ref().and_then(pre_load_action_for_action_ui)
                            {
                                refs.push(InteractionRef {
                                    action_id: pre_load_action.to_string(),
                                    hint: InteractionHint::DataLoad,
                                    sensitive_fields: vec![],
                                    form_ui: None,
                                });
                            }
                        } else {
                            has_unsupported_actions = true;
                        }
                    }
                    ActionDisposition::Unsupported => {
                        has_unsupported_actions = true;
                    }
                }
            }

            let root_node = if let Some(action_id) = single_form_action_id {
                let interaction_id = surfaces::InteractionId::new(action_id).ok()?;
                surfaces::SurfaceNode::Form { interaction_id }
            } else if !runnable_action_ids.is_empty() {
                surfaces::SurfaceNode::ActionBar {
                    action_ids: runnable_action_ids,
                }
            } else if has_unsupported_actions {
                text_fallback_node("No runnable actions are available for this surface.")
            } else {
                text_fallback_node("No actions are available for this surface.")
            };
            Some((root_node, vec![], refs))
        }
        ExtensionUi::Form(form) => {
            let Some(submit_action_id) = resolve_form_submit_action(form, action_index) else {
                return None;
            };
            let submit_interaction_id =
                surfaces::InteractionId::new(submit_action_id.clone()).ok()?;
            let submit_sensitive_fields = sensitive_fields_for_form(form);
            let mut refs = Vec::new();
            if let Some(pre_load) = form.pre_load_action.as_ref() {
                refs.push(InteractionRef {
                    action_id: pre_load.clone(),
                    hint: InteractionHint::DataLoad,
                    sensitive_fields: vec![],
                    form_ui: None,
                });
            }
            refs.extend(
                form.footer_actions
                    .iter()
                    .cloned()
                    .map(|id| InteractionRef {
                        sensitive_fields: if id == submit_action_id {
                            submit_sensitive_fields.clone()
                        } else {
                            vec![]
                        },
                        action_id: id,
                        hint: InteractionHint::Action,
                        form_ui: None,
                    }),
            );
            if !form.footer_actions.iter().any(|id| id == &submit_action_id) {
                refs.push(InteractionRef {
                    action_id: submit_action_id.clone(),
                    hint: InteractionHint::Action,
                    sensitive_fields: submit_sensitive_fields,
                    form_ui: Some(surface_form_ui_from_form(form)),
                });
            }

            let footer_action_ids = form
                .footer_actions
                .iter()
                .filter(|id| id.as_str() != submit_action_id)
                .filter_map(|id| surfaces::InteractionId::new(id.clone()).ok())
                .collect::<Vec<_>>();

            let root_node = if footer_action_ids.is_empty() {
                surfaces::SurfaceNode::Form {
                    interaction_id: submit_interaction_id,
                }
            } else {
                surfaces::SurfaceNode::Section {
                    title: None,
                    children: vec![
                        surfaces::SurfaceNode::Form {
                            interaction_id: submit_interaction_id,
                        },
                        surfaces::SurfaceNode::ActionBar {
                            action_ids: footer_action_ids,
                        },
                    ],
                }
            };
            Some((root_node, vec![], refs))
        }
        _ => None,
    }
}

fn resolve_form_submit_action(
    form: &uptrakit_extension_framework::FormDef,
    action_index: &HashMap<&str, &ActionDef>,
) -> Option<String> {
    if let Some(pre_load_action) = form.pre_load_action.as_deref()
        && let Some(suffix) = pre_load_action.strip_prefix("get_")
    {
        let inferred = format!("save_{suffix}");
        if action_index.contains_key(inferred.as_str()) {
            return Some(inferred);
        }
    }

    if action_index.contains_key("save") {
        return Some("save".to_string());
    }
    if action_index.contains_key("submit") {
        return Some("submit".to_string());
    }

    let mut save_actions: Vec<&str> = action_index
        .keys()
        .copied()
        .filter(|id| id.starts_with("save_"))
        .collect();
    save_actions.sort_unstable();
    (save_actions.len() == 1).then(|| save_actions[0].to_string())
}

fn sensitive_fields_for_form(form: &uptrakit_extension_framework::FormDef) -> Vec<String> {
    let mut fields = form
        .fields
        .iter()
        .filter(|field| field_is_sensitive(field))
        .map(|field| field.key.clone())
        .collect::<Vec<_>>();
    fields.sort();
    fields.dedup();
    fields
}

fn surface_form_ui_from_action_ui(
    ui: &uptrakit_extension_framework::ActionUi,
) -> Option<surfaces::FormUiDescriptor> {
    match ui {
        uptrakit_extension_framework::ActionUi::Form(form) => Some(surface_form_ui_from_form(form)),
        _ => None,
    }
}

fn pre_load_action_for_action_ui(ui: &uptrakit_extension_framework::ActionUi) -> Option<&str> {
    match ui {
        uptrakit_extension_framework::ActionUi::Form(form) => form.pre_load_action.as_deref(),
        _ => None,
    }
}

fn surface_form_ui_from_form(
    form: &uptrakit_extension_framework::FormDef,
) -> surfaces::FormUiDescriptor {
    surfaces::FormUiDescriptor {
        fields: form
            .fields
            .iter()
            .map(surface_form_field_from_extension)
            .collect(),
        pre_load_interaction_id: form
            .pre_load_action
            .as_ref()
            .and_then(|action_id| surfaces::InteractionId::new(action_id.clone()).ok()),
    }
}

fn surface_form_field_from_extension(
    field: &uptrakit_extension_framework::FieldDef,
) -> surfaces::FormFieldDescriptor {
    surfaces::FormFieldDescriptor {
        key: field.key.clone(),
        label: field.label.clone(),
        field_type: field_type_name(&field.field_type).to_string(),
        required: field.required,
        placeholder: field.placeholder.clone(),
        help_text: field.help_text.clone(),
        default_value: field
            .default_value
            .as_ref()
            .and_then(surface_default_value_from_extension),
        options: field
            .options
            .iter()
            .map(|option| surfaces::FormSelectOption {
                value: option.value.clone(),
                label: option.label.clone(),
            })
            .collect(),
        select_source: field
            .select_source
            .as_ref()
            .and_then(surface_select_source_from_extension),
        sensitive: field.sensitive,
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

fn surface_select_source_from_extension(
    select_source: &uptrakit_extension_framework::SelectSource,
) -> Option<surfaces::FormSelectSource> {
    match select_source {
        uptrakit_extension_framework::SelectSource::RestApi {
            path,
            value_field,
            label_field,
        } => Some(surfaces::FormSelectSource::RestApi {
            path: path.clone(),
            value_field: value_field.clone(),
            label_field: label_field.clone(),
        }),
        uptrakit_extension_framework::SelectSource::Action { action_id } => {
            Some(surfaces::FormSelectSource::Action {
                action_id: action_id.clone(),
            })
        }
        _ => None,
    }
}

fn surface_row_visible_when_from_extension(
    visible_when: &uptrakit_extension_framework::RowVisibleWhen,
) -> surfaces::SurfaceRowVisibleWhen {
    surfaces::SurfaceRowVisibleWhen {
        field: visible_when.field.clone(),
        condition: match visible_when.condition {
            uptrakit_extension_framework::RowCondition::Present => {
                surfaces::SurfaceRowCondition::Present
            }
            uptrakit_extension_framework::RowCondition::Absent
            | uptrakit_extension_framework::RowCondition::Other(_)
            | _ => surfaces::SurfaceRowCondition::Absent,
        },
    }
}

fn field_type_name(field_type: &uptrakit_extension_framework::FieldType) -> &'static str {
    match field_type {
        uptrakit_extension_framework::FieldType::Text => "text",
        uptrakit_extension_framework::FieldType::Password => "password",
        uptrakit_extension_framework::FieldType::Number => "number",
        uptrakit_extension_framework::FieldType::Select => "select",
        uptrakit_extension_framework::FieldType::MultiSelect => "multi_select",
        uptrakit_extension_framework::FieldType::Textarea => "textarea",
        uptrakit_extension_framework::FieldType::Toggle => "toggle",
        uptrakit_extension_framework::FieldType::Hidden => "hidden",
        uptrakit_extension_framework::FieldType::SshPrivateKey => "ssh_private_key",
        _ => "text",
    }
}

fn surface_default_value_from_extension(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(value).ok()
        }
    }
}

fn action_disposition(surface_id: &str, action_id: &str, action: &ActionDef) -> ActionDisposition {
    if action.api_submit.is_some()
        && !is_allowlisted_controller_local_api_submit_action(surface_id, action_id)
    {
        return ActionDisposition::Unsupported;
    }

    match action.ui.as_ref() {
        Some(ActionUi::Form(_)) => ActionDisposition::Form,
        Some(ActionUi::Wizard { .. }) => ActionDisposition::Unsupported,
        _ => ActionDisposition::Immediate,
    }
}

fn is_allowlisted_controller_local_api_submit_action(surface_id: &str, action_id: &str) -> bool {
    is_allowlisted_notification_channel_api_submit_action(surface_id, action_id)
        || is_allowlisted_proxmox_add_config_action(surface_id, action_id)
}

fn is_allowlisted_notification_channel_api_submit_action(
    surface_id: &str,
    action_id: &str,
) -> bool {
    matches!(action_id, "create" | "edit" | "test" | "delete")
        && matches!(
            surface_id,
            "notifications.email" | "notifications.telegram" | "notifications.webhook"
        )
}

fn is_allowlisted_proxmox_add_config_action(surface_id: &str, action_id: &str) -> bool {
    surface_id == "proxmox.hosts" && action_id == "add-config"
}

fn build_interactions(
    surface_id: &str,
    interaction_refs: &[InteractionRef],
    action_index: &HashMap<&str, &ActionDef>,
) -> Vec<surfaces::InteractionDescriptor> {
    let mut aggregated: std::collections::BTreeMap<
        String,
        (
            InteractionHint,
            BTreeSet<String>,
            Option<surfaces::FormUiDescriptor>,
        ),
    > = std::collections::BTreeMap::new();

    for reference in interaction_refs {
        let entry = aggregated.entry(reference.action_id.clone()).or_insert((
            reference.hint,
            BTreeSet::new(),
            None,
        ));
        if reference.hint == InteractionHint::DataLoad {
            entry.0 = InteractionHint::DataLoad;
        }
        entry.1.extend(reference.sensitive_fields.iter().cloned());
        if entry.2.is_none() {
            entry.2 = reference.form_ui.clone();
        }
    }

    let mut interactions = Vec::new();
    for (action_id, (hint, referenced_sensitive_fields, form_ui)) in aggregated {
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

        let mut sensitive_fields = referenced_sensitive_fields.into_iter().collect::<Vec<_>>();
        sensitive_fields.extend(sensitive_fields_for_action_ui(action));
        sensitive_fields.sort();
        sensitive_fields.dedup();

        interactions.push(surfaces::InteractionDescriptor {
            interaction_id,
            kind,
            label: action.map(|value| value.label.clone()),
            required_permission: if is_allowlisted_proxmox_add_config_action(
                surface_id,
                action_id.as_str(),
            ) {
                Some("manage_commands".to_string())
            } else {
                action.and_then(|value| permission_or_none(&value.permission))
            },
            input_schema: action.and_then(|value| {
                if value.ui.is_some() || value.api_submit.is_some() {
                    Some(surfaces::SchemaContract::Object)
                } else {
                    None
                }
            }),
            result_schema: Some(surfaces::SchemaContract::Any),
            sensitive_fields,
            timeout_seconds,
            confirmation,
            transport: surfaces::InteractionTransport::ControllerLocal,
            workflow_steps: vec![],
            form_ui,
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

fn sensitive_fields_for_action_ui(action: Option<&ActionDef>) -> Vec<String> {
    let Some(action) = action else {
        return vec![];
    };
    let mut fields = BTreeSet::new();
    match action.ui.as_ref() {
        Some(ActionUi::Form(form)) => {
            fields.extend(
                form.fields
                    .iter()
                    .filter(|field| field_is_sensitive(field))
                    .map(|field| field.key.clone()),
            );
        }
        Some(ActionUi::Wizard { steps }) => {
            for step in steps {
                fields.extend(
                    step.form
                        .fields
                        .iter()
                        .filter(|field| field_is_sensitive(field))
                        .map(|field| field.key.clone()),
                );
            }
        }
        Some(_) | None => {}
    }
    fields.into_iter().collect()
}

fn field_is_sensitive(field: &uptrakit_extension_framework::FieldDef) -> bool {
    field.sensitive
        || matches!(
            field.field_type,
            uptrakit_extension_framework::FieldType::Password
                | uptrakit_extension_framework::FieldType::SshPrivateKey
        )
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
        if !interaction.sensitive_fields.is_empty() {
            caps.insert(surfaces::Capability::SensitiveFields);
        }
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

fn slot_for_manifest(manifest: &ExtensionManifest) -> Option<&'static str> {
    match &manifest.placement {
        ExtensionPlacement::Page { .. } => Some(surfaces::SLOT_EXTENSION_PAGE),
        ExtensionPlacement::Panel {
            target_page,
            position,
            ..
        } => {
            if target_page == "global-settings" {
                return Some(surfaces::SLOT_SETTINGS_BELOW_GLOBAL);
            }
            if target_page == "settings" && matches!(position, PanelPosition::Tab) {
                return Some(surfaces::SLOT_SETTINGS_TABS);
            }
            if target_page == "software" {
                return Some(surfaces::SLOT_SOFTWARE_TABS);
            }
            if target_page == "host-detail" && matches!(position, PanelPosition::Tab) {
                return Some(surfaces::SLOT_HOST_DETAIL_TABS);
            }
            None
        }
        ExtensionPlacement::ContextMenuGroup { target_entity, .. } => {
            if target_entity == "software-item-host" {
                Some(surfaces::SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU)
            } else {
                None
            }
        }
        ExtensionPlacement::TableColumns { .. } => None,
        _ => None,
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

fn dedupe_preserve_order(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_extension_framework::{FieldDef, FieldType, FormDef};
    use uptrakit_shared_types::Permission;

    fn data_table_manifest() -> ExtensionManifest {
        ExtensionManifest::new(
            "notifications.webhook",
            "Webhook Channels",
            500,
            ExtensionPlacement::Panel {
                target_page: "settings".to_string(),
                position: PanelPosition::Tab,
                tab_group: None,
            },
            ExtensionUi::DataTable {
                columns: vec![uptrakit_extension_framework::TableColumn::new(
                    "name", "Name",
                )],
                data_action: "list".to_string(),
                row_actions: vec!["delete".to_string()],
                primary_actions: vec!["create".to_string()],
                context_selector: None,
                default_per_page: Some(20),
            },
        )
    }

    fn key_value_manifest_on_host_detail() -> ExtensionManifest {
        ExtensionManifest::new(
            "proxmox.host-info",
            "Proxmox Host Info",
            211,
            ExtensionPlacement::Panel {
                target_page: "host-detail".to_string(),
                position: PanelPosition::Tab,
                tab_group: None,
            },
            ExtensionUi::KeyValue {
                data_action: "get-info".to_string(),
            },
        )
    }

    #[test]
    fn data_table_manifest_without_context_selector_maps_to_renderable_table_surface() {
        let registrations = build_plugin_surface_registrations_from_extensions(
            "notifications_webhook",
            vec![data_table_manifest()],
            vec![
                ActionDef::new("list", "List"),
                ActionDef::new("create", "Create"),
                ActionDef::new("delete", "Delete"),
            ],
        );
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].surfaces.len(), 1);
        let surface = &registrations[0].surfaces[0];
        assert!(
            !surface.interactions.is_empty(),
            "data-table contract should yield runnable shared-surface interactions"
        );
        assert!(
            surface
                .interactions
                .iter()
                .any(|interaction| interaction.kind == surfaces::InteractionKind::DataLoad),
            "notification-style data tables should expose their list action as a shared-surface data-load interaction"
        );
        assert_eq!(surface.data_sources.len(), 1);
        assert!(matches!(
            surface.data_sources[0].kind,
            surfaces::DataSourceKind::ProviderQuery { ref operation_id }
                if operation_id == "list"
        ));
        assert!(matches!(
            surface.data_sources[0].pagination,
            Some(surfaces::DataSourcePagination {
                default_page_size: 20,
                max_page_size: 200
            })
        ));
        match &surface.descriptor.root_node {
            surfaces::SurfaceNode::Section { children, .. } => {
                assert!(
                    matches!(
                        children.first(),
                        Some(surfaces::SurfaceNode::ActionBar { .. })
                    ),
                    "notification tables should keep their primary actions above the table"
                );
                assert!(matches!(
                    children.get(1),
                    Some(surfaces::SurfaceNode::Table {
                        columns,
                        row_actions,
                        ..
                    }) if columns.len() == 1
                        && columns[0].key == "name"
                        && row_actions.len() == 1
                        && row_actions[0].interaction_id.as_str() == "delete"
                ));
            }
            other => panic!("expected section-root table surface, got {other:?}"),
        }
    }

    #[test]
    fn data_table_manifest_keeps_allowlisted_notification_api_submit_actions_runnable() {
        let manifest = data_table_manifest();

        let registrations = build_plugin_surface_registrations_from_extensions(
            "notifications_webhook",
            vec![manifest],
            vec![
                ActionDef::new("list", "List"),
                ActionDef::new("create", "Create").with_api_submit(
                    uptrakit_extension_framework::ApiSubmitDef::new(
                        "POST",
                        "/api/v1/notifications/channels",
                        serde_json::json!({ "name": "{{name}}" }),
                    ),
                ),
                ActionDef::new("delete", "Delete"),
            ],
        );

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].surfaces.len(), 1);
        let surface = &registrations[0].surfaces[0];
        assert!(
            surface
                .interactions
                .iter()
                .any(|interaction| interaction.interaction_id.as_str() == "create"),
            "allowlisted notification api_submit actions must be preserved as runnable shared-surface interactions"
        );
        assert!(
            surface
                .interactions
                .iter()
                .any(|interaction| interaction.interaction_id.as_str() == "delete"),
            "other data-table actions should remain available"
        );
    }

    #[test]
    fn data_table_manifest_filters_non_allowlisted_api_submit_actions() {
        let manifest = ExtensionManifest::new(
            "custom.integration",
            "Custom",
            400,
            ExtensionPlacement::Panel {
                target_page: "settings".to_string(),
                position: PanelPosition::Tab,
                tab_group: None,
            },
            ExtensionUi::DataTable {
                columns: vec![uptrakit_extension_framework::TableColumn::new(
                    "name", "Name",
                )],
                data_action: "list".to_string(),
                row_actions: vec!["delete".to_string()],
                primary_actions: vec!["create".to_string()],
                context_selector: None,
                default_per_page: Some(20),
            },
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "custom_provider",
            vec![manifest],
            vec![
                ActionDef::new("list", "List"),
                ActionDef::new("create", "Create").with_api_submit(
                    uptrakit_extension_framework::ApiSubmitDef::new(
                        "POST",
                        "/api/v1/custom",
                        serde_json::json!({ "name": "{{name}}" }),
                    ),
                ),
                ActionDef::new("delete", "Delete"),
            ],
        );

        assert_eq!(registrations.len(), 1);
        let surface = &registrations[0].surfaces[0];
        assert!(
            surface
                .interactions
                .iter()
                .all(|interaction| interaction.interaction_id.as_str() != "create"),
            "non-allowlisted api_submit actions must continue to be filtered"
        );
    }

    #[test]
    fn proxmox_hosts_add_config_api_submit_stays_runnable_and_permission_hardened() {
        let manifest = ExtensionManifest::new(
            "proxmox.hosts",
            "Proxmox VE Hosts",
            650,
            ExtensionPlacement::Page {
                nav_section: "infrastructure".to_string(),
                icon: Some("server".to_string()),
            },
            ExtensionUi::DataTable {
                columns: vec![uptrakit_extension_framework::TableColumn::new(
                    "name", "Name",
                )],
                data_action: "list".to_string(),
                row_actions: vec![],
                primary_actions: vec!["discover".to_string()],
                context_selector: Some(Box::new(
                    uptrakit_extension_framework::ContextSelectorDef::new(
                        "plugin_config_id",
                        "Configuration",
                        uptrakit_extension_framework::ContextSelectorSource::PluginConfigs {
                            plugin_type: "infrastructure_proxmox".to_string(),
                        },
                    )
                    .with_add_action("add-config"),
                )),
                default_per_page: Some(50),
            },
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "infrastructure_proxmox",
            vec![manifest],
            vec![
                ActionDef::new("list", "List"),
                ActionDef::new("discover", "Discover"),
                ActionDef::new("add-config", "Add Configuration")
                    .with_permission(Permission::UpdateHosts)
                    .with_ui(ActionUi::Form(FormDef::new(vec![
                        FieldDef::new("name", "Configuration Name").required(),
                    ])))
                    .with_api_submit(uptrakit_extension_framework::ApiSubmitDef::new(
                        "POST",
                        "/api/v1/plugin-configs",
                        serde_json::json!({
                            "name": "{{name}}",
                            "plugin_type": "infrastructure_proxmox",
                            "enabled": true,
                            "config": {
                                "api_url": "{{api_url}}",
                                "api_token": "{{api_token}}",
                                "verify_tls": "{{verify_tls:bool}}",
                                "node_filter": "{{node_filter:csv_array}}"
                            }
                        }),
                    )),
            ],
        );

        assert_eq!(registrations.len(), 1);
        let surface = &registrations[0].surfaces[0];
        let add_config = surface
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "add-config")
            .expect("proxmox add-config must remain actionable after shared-surface conversion");
        assert_eq!(add_config.kind, surfaces::InteractionKind::FormSubmit);
        assert_eq!(
            add_config.required_permission.as_deref(),
            Some("manage_commands"),
            "controller-owned proxmox add-config path must retain manage_commands-level safety"
        );
    }

    #[test]
    fn host_detail_panel_key_value_manifest_maps_to_data_load_key_value_surface() {
        let registrations = build_plugin_surface_registrations_from_extensions(
            "infrastructure_proxmox",
            vec![key_value_manifest_on_host_detail()],
            vec![ActionDef::new("get-info", "Get Info")],
        );
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].surfaces.len(), 1);
        let surface = &registrations[0].surfaces[0];
        assert_eq!(surface.descriptor.surface_id.as_str(), "proxmox.host-info");
        assert_eq!(surface.descriptor.slot, surfaces::SLOT_HOST_DETAIL_TABS);
        assert!(
            matches!(
                surface.descriptor.root_node,
                surfaces::SurfaceNode::KeyValue { .. }
            ),
            "host-detail key-value panels should map to key-value surface nodes"
        );
        assert_eq!(surface.data_sources.len(), 1);
        assert!(matches!(
            surface.data_sources[0].kind,
            surfaces::DataSourceKind::ProviderQuery { .. }
        ));
        assert_eq!(surface.interactions.len(), 1);
        assert_eq!(
            surface.interactions[0].interaction_id.as_str(),
            "get-info",
            "data_action should remain invokable via surface interaction endpoint"
        );
        assert_eq!(
            surface.interactions[0].kind,
            surfaces::InteractionKind::DataLoad
        );
    }

    #[test]
    fn form_manifest_with_inferred_save_action_maps_to_form_node() {
        let manifest = ExtensionManifest::new(
            "notifications.telegram.global_settings",
            "Telegram Defaults",
            601,
            ExtensionPlacement::Panel {
                target_page: "global-settings".to_string(),
                position: PanelPosition::Below,
                tab_group: None,
            },
            ExtensionUi::Form(
                FormDef::new(vec![
                    FieldDef::new("bot_token", "Bot Token").with_type(FieldType::Password),
                ])
                .with_pre_load_action("get_global_telegram"),
            ),
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "notifications_telegram",
            vec![manifest],
            vec![
                ActionDef::new("get_global_telegram", "Get"),
                ActionDef::new("save_global_telegram", "Save"),
            ],
        );

        let root_node = &registrations[0].surfaces[0].descriptor.root_node;
        match root_node {
            surfaces::SurfaceNode::Form { interaction_id } => {
                assert_eq!(interaction_id.as_str(), "save_global_telegram");
            }
            other => panic!("expected form node, got {other:?}"),
        }
    }

    #[test]
    fn form_manifest_propagates_sensitive_fields_to_submit_interaction() {
        let manifest = ExtensionManifest::new(
            "notifications.telegram.global_settings",
            "Telegram Defaults",
            601,
            ExtensionPlacement::Panel {
                target_page: "global-settings".to_string(),
                position: PanelPosition::Below,
                tab_group: None,
            },
            ExtensionUi::Form(
                FormDef::new(vec![
                    FieldDef::new("bot_token", "Bot Token")
                        .with_type(FieldType::Password)
                        .sensitive(),
                ])
                .with_pre_load_action("get_global_telegram"),
            ),
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "notifications_telegram",
            vec![manifest],
            vec![
                ActionDef::new("get_global_telegram", "Get"),
                ActionDef::new("save_global_telegram", "Save"),
            ],
        );
        let submit = registrations[0].surfaces[0]
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "save_global_telegram")
            .expect("submit interaction should exist");
        assert!(
            submit
                .sensitive_fields
                .iter()
                .any(|field| field == "bot_token"),
            "form sensitive fields must be forwarded to interaction descriptor"
        );
    }

    #[test]
    fn form_manifest_treats_password_type_fields_as_sensitive() {
        let manifest = ExtensionManifest::new(
            "notifications.telegram.global_settings",
            "Telegram Defaults",
            601,
            ExtensionPlacement::Panel {
                target_page: "global-settings".to_string(),
                position: PanelPosition::Below,
                tab_group: None,
            },
            ExtensionUi::Form(
                FormDef::new(vec![
                    FieldDef::new("bot_token", "Bot Token").with_type(FieldType::Password),
                ])
                .with_pre_load_action("get_global_telegram"),
            ),
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "notifications_telegram",
            vec![manifest],
            vec![
                ActionDef::new("get_global_telegram", "Get"),
                ActionDef::new("save_global_telegram", "Save"),
            ],
        );
        let submit = registrations[0].surfaces[0]
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "save_global_telegram")
            .expect("submit interaction should exist");
        assert!(
            submit
                .sensitive_fields
                .iter()
                .any(|field| field == "bot_token"),
            "password fields should be treated as sensitive in generated interactions"
        );
    }

    #[test]
    fn form_manifest_without_submit_action_is_skipped() {
        let manifest = ExtensionManifest::new(
            "notifications.telegram.global_settings",
            "Telegram Defaults",
            601,
            ExtensionPlacement::Panel {
                target_page: "global-settings".to_string(),
                position: PanelPosition::Below,
                tab_group: None,
            },
            ExtensionUi::Form(
                FormDef::new(vec![
                    FieldDef::new("bot_token", "Bot Token").with_type(FieldType::Password),
                ])
                .with_pre_load_action("get_global_telegram"),
            ),
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "notifications_telegram",
            vec![manifest],
            vec![ActionDef::new("get_global_telegram", "Get")],
        );

        assert!(
            registrations.is_empty(),
            "form manifests without a resolvable submit action should be skipped"
        );
    }

    #[test]
    fn actions_manifest_with_single_form_action_carries_preload_metadata() {
        let manifest = ExtensionManifest::new(
            "docker.item-host-actions",
            "Docker",
            100,
            ExtensionPlacement::ContextMenuGroup {
                target_entity: "software-item-host".to_string(),
                group_label: "Docker".to_string(),
            },
            ExtensionUi::Actions {
                actions: vec!["switch-tag".to_string()],
            },
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "releases_docker",
            vec![manifest],
            vec![
                ActionDef::new("switch-tag", "Switch Tag").with_ui(ActionUi::Form(
                    FormDef::new(vec![
                        FieldDef::new("software_item_id", "")
                            .with_type(FieldType::Hidden)
                            .required(),
                        FieldDef::new("host_id", "")
                            .with_type(FieldType::Hidden)
                            .required(),
                        FieldDef::new("new_image_ref", "New Image Reference").required(),
                    ])
                    .with_pre_load_action("get-current-tag"),
                )),
                ActionDef::new("get-current-tag", ""),
            ],
        );

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].surfaces.len(), 1);
        let surface = &registrations[0].surfaces[0];
        match &surface.descriptor.root_node {
            surfaces::SurfaceNode::Form { interaction_id } => {
                assert_eq!(interaction_id.as_str(), "switch-tag");
            }
            other => panic!("expected Form node for single form-backed action, got {other:?}"),
        }

        let submit = surface
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "switch-tag")
            .expect("switch-tag interaction should exist");
        assert_eq!(submit.kind, surfaces::InteractionKind::FormSubmit);
        let preload = surface
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "get-current-tag")
            .expect("get-current-tag preload interaction should exist");
        assert_eq!(preload.kind, surfaces::InteractionKind::DataLoad);
        assert_eq!(
            submit
                .form_ui
                .as_ref()
                .and_then(|ui| ui.pre_load_interaction_id.as_ref())
                .map(|interaction_id| interaction_id.as_str()),
            Some("get-current-tag")
        );
    }

    #[test]
    fn actions_manifest_with_allowlisted_api_submit_action_stays_actionable() {
        let manifest = ExtensionManifest::new(
            "notifications.webhook",
            "Webhook Channels",
            500,
            ExtensionPlacement::Panel {
                target_page: "settings".to_string(),
                position: PanelPosition::Tab,
                tab_group: Some("Notification Channels".to_string()),
            },
            ExtensionUi::Actions {
                actions: vec!["create".to_string()],
            },
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "webhook",
            vec![manifest],
            vec![
                ActionDef::new("create", "Add Webhook")
                    .with_ui(ActionUi::Form(FormDef::new(vec![
                        FieldDef::new("name", "Name").required(),
                    ])))
                    .with_api_submit(uptrakit_extension_framework::ApiSubmitDef::new(
                        "POST",
                        "/api/v1/notifications/channels",
                        serde_json::json!({ "name": "{{name}}" }),
                    )),
            ],
        );

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].surfaces.len(), 1);
        let surface = &registrations[0].surfaces[0];
        assert!(
            surface
                .interactions
                .iter()
                .any(|interaction| interaction.interaction_id.as_str() == "create"),
            "allowlisted notification api_submit actions should remain runnable"
        );
        assert!(
            matches!(
                surface.descriptor.root_node,
                surfaces::SurfaceNode::Form { .. }
                    | surfaces::SurfaceNode::Section { .. }
                    | surfaces::SurfaceNode::Tabs { .. }
            ),
            "allowlisted api_submit action surfaces should stay actionable"
        );
    }

    #[test]
    fn actions_manifest_with_only_non_allowlisted_api_submit_actions_downgrades_to_notice() {
        let manifest = ExtensionManifest::new(
            "custom.integration",
            "Custom",
            500,
            ExtensionPlacement::Panel {
                target_page: "settings".to_string(),
                position: PanelPosition::Tab,
                tab_group: Some("Custom".to_string()),
            },
            ExtensionUi::Actions {
                actions: vec!["create".to_string()],
            },
        );

        let registrations = build_plugin_surface_registrations_from_extensions(
            "custom_provider",
            vec![manifest],
            vec![
                ActionDef::new("create", "Create")
                    .with_ui(ActionUi::Form(FormDef::new(vec![
                        FieldDef::new("name", "Name").required(),
                    ])))
                    .with_api_submit(uptrakit_extension_framework::ApiSubmitDef::new(
                        "POST",
                        "/api/v1/custom",
                        serde_json::json!({ "name": "{{name}}" }),
                    )),
            ],
        );

        let surface = &registrations[0].surfaces[0];
        assert!(
            surface.interactions.is_empty(),
            "non-allowlisted api_submit-only actions should remain non-runnable"
        );
        assert!(
            matches!(
                surface.descriptor.root_node,
                surfaces::SurfaceNode::Callout { .. } | surfaces::SurfaceNode::TextBlock { .. }
            ),
            "non-allowlisted api_submit-only surfaces should still downgrade to a notice"
        );
    }
}
