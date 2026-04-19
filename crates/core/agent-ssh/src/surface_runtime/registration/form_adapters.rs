use std::collections::BTreeSet;

use uptrakit_internal_wire::surfaces::{
    self, FormFieldDescriptor, FormSelectOption, FormUiDescriptor, InteractionId,
};
use uptrakit_plugin_infrastructure_registry::{
    FormFieldDescriptor as PluginFormFieldDescriptor, FormFieldType as PluginFormFieldType,
    FormSelectSourceDescriptor as PluginFormSelectSourceDescriptor, SurfaceActionDescriptor,
    SurfaceActionUi, SurfaceFormDescriptor as PluginSurfaceFormDescriptor,
    SurfaceRowCondition as PluginSurfaceRowCondition,
    SurfaceRowVisibleWhen as PluginSurfaceRowVisibleWhen,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InteractionHint {
    DataLoad,
    Action,
}

#[derive(Debug, Clone)]
pub(super) struct InteractionRef {
    pub(super) action_id: String,
    pub(super) hint: InteractionHint,
    pub(super) form_ui: Option<FormUiDescriptor>,
    pub(super) sensitive_fields: Vec<String>,
}

pub(super) fn collect_select_source_action_refs(
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

pub(super) fn action_ui_to_form_ui(ui: &SurfaceActionUi) -> Option<FormUiDescriptor> {
    match ui {
        SurfaceActionUi::Form(form) => Some(form_ui_from_form(form)),
        _ => None,
    }
}

pub(super) fn form_ui_from_form(form: &PluginSurfaceFormDescriptor) -> FormUiDescriptor {
    FormUiDescriptor {
        fields: form.fields.iter().map(field_from_extension).collect(),
        pre_load_interaction_id: form
            .pre_load_action
            .as_ref()
            .and_then(|id| InteractionId::new(id.clone()).ok()),
    }
}

pub(super) fn collect_workflow_step_refs(
    action: &SurfaceActionDescriptor,
    refs: &mut Vec<InteractionRef>,
) {
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

pub(super) fn row_visible_when_from_extension(
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

pub(super) fn sensitive_fields_for_action(action: &SurfaceActionDescriptor) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uptrakit_plugin_infrastructure_registry::SurfaceWorkflowStep as PluginSurfaceWorkflowStep;

    #[test]
    fn sensitive_fields_include_password_and_private_key_types() {
        let action = SurfaceActionDescriptor::new("configure", "Configure").with_ui(
            SurfaceActionUi::Form(PluginSurfaceFormDescriptor::new(vec![
                PluginFormFieldDescriptor::new("password", "Password")
                    .with_type(PluginFormFieldType::Password),
                PluginFormFieldDescriptor::new("private_key", "Private Key")
                    .with_type(PluginFormFieldType::SshPrivateKey),
                PluginFormFieldDescriptor::new("token", "Token").sensitive(),
            ])),
        );

        let sensitive: BTreeSet<_> = sensitive_fields_for_action(&action).into_iter().collect();
        let expected: BTreeSet<_> = ["password", "private_key", "token"]
            .into_iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(sensitive, expected);
    }

    #[test]
    fn json_value_defaults_stringify_consistently() {
        assert_eq!(
            json_value_to_string(&json!("alpha")),
            Some("alpha".to_string())
        );
        assert_eq!(json_value_to_string(&json!(true)), Some("true".to_string()));
        assert_eq!(json_value_to_string(&json!(42)), Some("42".to_string()));
        assert_eq!(
            json_value_to_string(&json!({ "a": 1 })),
            Some("{\"a\":1}".to_string())
        );
    }

    #[test]
    fn row_visible_when_mapping_preserves_supported_conditions() {
        let present_source = SurfaceActionDescriptor::new("present", "Present")
            .with_row_visible_when("machine_id", PluginSurfaceRowCondition::Present);
        let present = row_visible_when_from_extension(
            present_source
                .row_visible_when
                .as_ref()
                .expect("present row visibility condition should exist"),
        );
        assert_eq!(present.field, "machine_id");
        assert_eq!(present.condition, surfaces::SurfaceRowCondition::Present);

        let absent_source = SurfaceActionDescriptor::new("absent", "Absent")
            .with_row_visible_when("machine_id", PluginSurfaceRowCondition::Absent);
        let absent = row_visible_when_from_extension(
            absent_source
                .row_visible_when
                .as_ref()
                .expect("absent row visibility condition should exist"),
        );
        assert_eq!(absent.condition, surfaces::SurfaceRowCondition::Absent);

        let unknown_source = SurfaceActionDescriptor::new("unknown", "Unknown")
            .with_row_visible_when(
                "machine_id",
                PluginSurfaceRowCondition::Other("unsupported".to_string()),
            );
        let unknown = row_visible_when_from_extension(
            unknown_source
                .row_visible_when
                .as_ref()
                .expect("unknown row visibility condition should exist"),
        );
        assert_eq!(unknown.condition, surfaces::SurfaceRowCondition::Present);
    }

    #[test]
    fn collect_workflow_step_refs_collects_select_preload_and_submit_refs() {
        let step_form = PluginSurfaceFormDescriptor::new(vec![
            PluginFormFieldDescriptor::new("auth_password", "SSH Password")
                .with_type(PluginFormFieldType::Password),
            PluginFormFieldDescriptor::new("target", "Target").with_select_source(
                PluginFormSelectSourceDescriptor::Action {
                    action_id: "load-targets".to_string(),
                },
            ),
        ])
        .with_pre_load_action("preload-connect");

        let action = SurfaceActionDescriptor::new("sync-host", "Sync Host").with_ui(
            SurfaceActionUi::Wizard {
                steps: vec![
                    PluginSurfaceWorkflowStep::new("connect", "Connect", step_form)
                        .with_submit_action("sync-connect"),
                ],
            },
        );

        let mut refs = Vec::new();
        collect_workflow_step_refs(&action, &mut refs);

        let select_ref = refs
            .iter()
            .find(|reference| reference.action_id == "load-targets")
            .expect("select-source action ref should be collected");
        assert_eq!(select_ref.hint, InteractionHint::DataLoad);

        let preload_ref = refs
            .iter()
            .find(|reference| reference.action_id == "preload-connect")
            .expect("pre-load action ref should be collected");
        assert_eq!(preload_ref.hint, InteractionHint::DataLoad);

        let submit_ref = refs
            .iter()
            .find(|reference| reference.action_id == "sync-connect")
            .expect("submit action ref should be collected");
        assert_eq!(submit_ref.hint, InteractionHint::Action);
        assert_eq!(
            submit_ref.sensitive_fields,
            vec!["auth_password".to_string()]
        );
    }
}
