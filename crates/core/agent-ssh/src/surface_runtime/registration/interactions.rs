use std::collections::{BTreeMap, BTreeSet};

use uptrakit_internal_wire::surfaces::{
    self, FormUiDescriptor, InteractionDescriptor, InteractionId, InteractionKind,
    InteractionTransport,
};
use uptrakit_plugin_infrastructure_registry::{SurfaceActionDescriptor, SurfaceActionUi};

use super::form_adapters::{InteractionHint, InteractionRef, form_ui_from_form};

#[derive(Debug, Clone)]
struct MergedInteraction {
    hint: InteractionHint,
    form_ui: Option<FormUiDescriptor>,
    sensitive_fields: BTreeSet<String>,
}

fn merge_interaction_refs(refs: &[InteractionRef]) -> BTreeMap<String, MergedInteraction> {
    let mut merged = BTreeMap::new();
    for reference in refs {
        let entry = merged
            .entry(reference.action_id.clone())
            .or_insert_with(|| MergedInteraction {
                hint: reference.hint,
                form_ui: None,
                sensitive_fields: BTreeSet::new(),
            });
        if reference.hint == InteractionHint::DataLoad {
            entry.hint = InteractionHint::DataLoad;
        }
        if entry.form_ui.is_none() {
            entry.form_ui = reference.form_ui.clone();
        }
        entry
            .sensitive_fields
            .extend(reference.sensitive_fields.iter().cloned());
    }
    merged
}

pub(super) fn build_interactions(
    refs: &[InteractionRef],
    action_index: &BTreeMap<&str, &SurfaceActionDescriptor>,
) -> Vec<InteractionDescriptor> {
    let mut interactions = Vec::new();
    for (action_id, merged_ref) in merge_interaction_refs(refs) {
        let Ok(interaction_id) = InteractionId::new(action_id.clone()) else {
            continue;
        };
        let action = action_index.get(action_id.as_str()).copied();
        let kind = match merged_ref.hint {
            InteractionHint::DataLoad => InteractionKind::DataLoad,
            InteractionHint::Action => action_kind_for_action(action),
        };
        let confirmation = if kind == InteractionKind::ConfirmableAction {
            Some(surfaces::InteractionConfirmation {
                title: format!(
                    "Confirm {}",
                    action
                        .map(|value| value.label.as_str())
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
            sensitive_fields: merged_ref.sensitive_fields.into_iter().collect(),
            timeout_seconds,
            confirmation,
            transport: InteractionTransport::ProviderProxied,
            workflow_steps,
            form_ui: merged_ref.form_ui,
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

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_registry::{
        SurfaceFormDescriptor as PluginSurfaceFormDescriptor, SurfaceWorkflowStep,
    };

    fn action_index<'a>(
        actions: &'a [SurfaceActionDescriptor],
    ) -> BTreeMap<&'a str, &'a SurfaceActionDescriptor> {
        actions
            .iter()
            .map(|action| (action.action_id.as_str(), action))
            .collect()
    }

    #[test]
    fn data_load_refs_dominate_action_refs_when_merged() {
        let refs = vec![
            InteractionRef {
                action_id: "load-options".to_string(),
                hint: InteractionHint::Action,
                form_ui: None,
                sensitive_fields: vec![],
            },
            InteractionRef {
                action_id: "load-options".to_string(),
                hint: InteractionHint::DataLoad,
                form_ui: None,
                sensitive_fields: vec![],
            },
        ];
        let action_index: BTreeMap<&str, &SurfaceActionDescriptor> = BTreeMap::new();

        let interactions = build_interactions(&refs, &action_index);
        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].kind, InteractionKind::DataLoad);
    }

    #[test]
    fn destructive_actions_with_confirm_field_are_confirmable() {
        let action = SurfaceActionDescriptor::new("remove-host", "Remove Host")
            .destructive()
            .with_confirm_entity_field("name");
        let refs = vec![InteractionRef {
            action_id: action.action_id.clone(),
            hint: InteractionHint::Action,
            form_ui: None,
            sensitive_fields: vec![],
        }];
        let actions = vec![action];

        let interactions = build_interactions(&refs, &action_index(&actions));
        let interaction = interactions
            .first()
            .expect("interaction should be emitted for known action");
        assert_eq!(interaction.kind, InteractionKind::ConfirmableAction);
        assert!(interaction.confirmation.is_some());
    }

    #[test]
    fn wizard_actions_emit_workflow_steps_with_matching_submit_ids() {
        let action = SurfaceActionDescriptor::new("sync-host", "Sync Host").with_ui(
            SurfaceActionUi::Wizard {
                steps: vec![
                    SurfaceWorkflowStep::new(
                        "connect",
                        "Connect",
                        PluginSurfaceFormDescriptor::new(vec![]),
                    )
                    .with_submit_action("sync-connect"),
                    SurfaceWorkflowStep::new(
                        "execute",
                        "Execute",
                        PluginSurfaceFormDescriptor::new(vec![]),
                    )
                    .with_submit_action("sync-execute"),
                ],
            },
        );
        let refs = vec![InteractionRef {
            action_id: action.action_id.clone(),
            hint: InteractionHint::Action,
            form_ui: None,
            sensitive_fields: vec![],
        }];
        let actions = vec![action];

        let interactions = build_interactions(&refs, &action_index(&actions));
        let interaction = interactions
            .first()
            .expect("workflow interaction should be emitted");
        assert_eq!(interaction.kind, InteractionKind::Workflow);
        assert_eq!(interaction.workflow_steps.len(), 2);
        let submit_ids: Vec<_> = interaction
            .workflow_steps
            .iter()
            .map(|step| {
                step.submit_interaction_id
                    .as_ref()
                    .map(|id| id.as_str().to_string())
            })
            .collect();
        assert_eq!(
            submit_ids,
            vec![
                Some("sync-connect".to_string()),
                Some("sync-execute".to_string()),
            ]
        );
    }

    #[test]
    fn permission_or_none_handles_empty_and_none_values() {
        assert_eq!(permission_or_none(""), None);
        assert_eq!(permission_or_none("   "), None);
        assert_eq!(permission_or_none("none"), None);
        assert_eq!(permission_or_none(" none "), None);
        assert_eq!(
            permission_or_none("update.hosts"),
            Some("update.hosts".to_string())
        );
        assert_eq!(
            permission_or_none(" update.hosts "),
            Some("update.hosts".to_string())
        );
    }
}
