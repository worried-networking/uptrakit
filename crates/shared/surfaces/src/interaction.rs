use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{FormUiDescriptor, InteractionId, ProviderKind, SchemaContract};

pub const MIN_INTERACTION_TIMEOUT_SECONDS: u16 = 1;
pub const MAX_INTERACTION_TIMEOUT_SECONDS: u16 = 300;

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    MutationAction,
    FormSubmit,
    Workflow,
    Navigate,
    DataLoad,
    ConfirmableAction,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum InteractionTransport {
    ControllerLocal,
    ProviderProxied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStepDescriptor {
    pub step_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_ui: Option<FormUiDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_interaction_id: Option<InteractionId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub render_previous_response: bool,
    pub input_schema: SchemaContract,
    pub result_schema: SchemaContract,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionDescriptor {
    pub interaction_id: InteractionId,
    pub kind: InteractionKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_permission: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<SchemaContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_schema: Option<SchemaContract>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sensitive_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<InteractionConfirmation>,
    pub transport: InteractionTransport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflow_steps: Vec<WorkflowStepDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_ui: Option<FormUiDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_label: Option<String>,
    /// Allows same-tenant provider-origin (service-initiated) invocation of
    /// this interaction even when `required_permission` is set. Fail-closed:
    /// absent on the wire deserializes to `false`. Honored only for
    /// `Plugin`/`BuiltIn`-registered interactions — see `validate_for_provider`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub provider_invocable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionConfirmation {
    pub title: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_label: Option<String>,
    pub severity: ConfirmationSeverity,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationSeverity {
    Info,
    Warning,
    Danger,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InteractionValidationError {
    #[error(
        "interaction `{interaction_id}` timeout must be between {MIN_INTERACTION_TIMEOUT_SECONDS} and {MAX_INTERACTION_TIMEOUT_SECONDS} seconds"
    )]
    TimeoutOutOfRange { interaction_id: InteractionId },
    #[error("workflow interaction `{interaction_id}` must declare at least one workflow step")]
    WorkflowMissingSteps { interaction_id: InteractionId },
    #[error("confirmable interaction `{interaction_id}` must include confirmation metadata")]
    ConfirmableActionMissingConfirmation { interaction_id: InteractionId },
    #[error("interaction `{interaction_id}` must include a non-empty human-authored label")]
    BlankLabel { interaction_id: InteractionId },
    #[error(
        "workflow step `{step_id}` in interaction `{interaction_id}` must include a non-empty human-authored label"
    )]
    BlankWorkflowStepLabel {
        interaction_id: InteractionId,
        step_id: String,
    },
    #[error("interaction `{interaction_id}` has invalid icon: {reason}")]
    IconInvalid {
        interaction_id: InteractionId,
        reason: crate::IconNameError,
    },
    #[error("interaction `{interaction_id}` has invalid submit_label: {reason}")]
    SubmitLabelInvalid {
        interaction_id: InteractionId,
        reason: String,
    },
    #[error(
        "interaction `{interaction_id}` sets provider_invocable with a required_permission — not allowed for service-registered surfaces"
    )]
    ProviderInvocableForbiddenForServiceProviders { interaction_id: InteractionId },
}

impl InteractionDescriptor {
    /// Creates a new `InteractionDescriptor` with all optional fields set to their defaults.
    pub fn new(
        interaction_id: InteractionId,
        kind: InteractionKind,
        label: impl Into<String>,
        transport: InteractionTransport,
    ) -> Self {
        Self {
            interaction_id,
            kind,
            label: label.into(),
            transport,
            required_permission: None,
            input_schema: None,
            result_schema: None,
            sensitive_fields: vec![],
            timeout_seconds: None,
            confirmation: None,
            workflow_steps: vec![],
            form_ui: None,
            icon: None,
            submit_label: None,
            provider_invocable: false,
        }
    }

    /// Validates provider-specific interaction contract rules.
    ///
    /// # Errors
    /// Returns [`InteractionValidationError::TimeoutOutOfRange`] when
    /// `timeout_seconds` falls outside
    /// [`MIN_INTERACTION_TIMEOUT_SECONDS`]..=[`MAX_INTERACTION_TIMEOUT_SECONDS`].
    /// Returns [`InteractionValidationError::WorkflowMissingSteps`] when
    /// a workflow interaction declares no steps.
    /// Returns
    /// [`InteractionValidationError::ConfirmableActionMissingConfirmation`]
    /// when a confirmable interaction omits confirmation metadata.
    /// Returns [`InteractionValidationError::BlankLabel`] when `label` is
    /// empty or whitespace-only.
    /// Returns [`InteractionValidationError::BlankWorkflowStepLabel`] when
    /// any workflow step has an empty or whitespace-only label.
    /// Returns [`InteractionValidationError::IconInvalid`] when `icon` is
    /// `Some` but fails kebab-case validation.
    /// Returns
    /// [`InteractionValidationError::ProviderInvocableForbiddenForServiceProviders`]
    /// when a `Service` provider sets `provider_invocable` on an interaction
    /// that also declares `required_permission`.
    pub fn validate_for_provider(
        &self,
        provider_kind: ProviderKind,
    ) -> Result<(), InteractionValidationError> {
        if self.provider_invocable
            && self.required_permission.is_some()
            && provider_kind == ProviderKind::Service
        {
            return Err(
                InteractionValidationError::ProviderInvocableForbiddenForServiceProviders {
                    interaction_id: self.interaction_id.clone(),
                },
            );
        }

        if let Some(timeout_seconds) = self.timeout_seconds
            && !(MIN_INTERACTION_TIMEOUT_SECONDS..=MAX_INTERACTION_TIMEOUT_SECONDS)
                .contains(&timeout_seconds)
        {
            return Err(InteractionValidationError::TimeoutOutOfRange {
                interaction_id: self.interaction_id.clone(),
            });
        }

        if self.kind == InteractionKind::Workflow && self.workflow_steps.is_empty() {
            return Err(InteractionValidationError::WorkflowMissingSteps {
                interaction_id: self.interaction_id.clone(),
            });
        }

        if self.kind == InteractionKind::ConfirmableAction && self.confirmation.is_none() {
            return Err(
                InteractionValidationError::ConfirmableActionMissingConfirmation {
                    interaction_id: self.interaction_id.clone(),
                },
            );
        }

        if self.label.trim().is_empty() {
            return Err(InteractionValidationError::BlankLabel {
                interaction_id: self.interaction_id.clone(),
            });
        }

        for step in &self.workflow_steps {
            if step.label.trim().is_empty() {
                return Err(InteractionValidationError::BlankWorkflowStepLabel {
                    interaction_id: self.interaction_id.clone(),
                    step_id: step.step_id.clone(),
                });
            }
        }

        if let Some(icon) = &self.icon {
            crate::validate_icon_name(icon).map_err(|reason| {
                InteractionValidationError::IconInvalid {
                    interaction_id: self.interaction_id.clone(),
                    reason,
                }
            })?;
        }

        if let Some(submit_label) = &self.submit_label {
            if submit_label.trim().is_empty() {
                return Err(InteractionValidationError::SubmitLabelInvalid {
                    interaction_id: self.interaction_id.clone(),
                    reason: "must not be empty".to_string(),
                });
            }
            if submit_label.len() > 50 {
                return Err(InteractionValidationError::SubmitLabelInvalid {
                    interaction_id: self.interaction_id.clone(),
                    reason: format!("exceeds max 50 characters ({} given)", submit_label.len()),
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_for_provider_accepts_kebab_icon() {
        let descriptor = InteractionDescriptor {
            icon: Some("trash-2".to_string()),
            ..InteractionDescriptor::new(
                InteractionId::new("act").unwrap(),
                InteractionKind::MutationAction,
                "Action",
                InteractionTransport::ControllerLocal,
            )
        };
        descriptor
            .validate_for_provider(ProviderKind::Plugin)
            .unwrap();
    }

    #[test]
    fn validate_for_provider_rejects_pascal_icon() {
        let mut descriptor = InteractionDescriptor {
            icon: Some("Trash2".to_string()),
            ..InteractionDescriptor::new(
                InteractionId::new("act").unwrap(),
                InteractionKind::MutationAction,
                "Action",
                InteractionTransport::ControllerLocal,
            )
        };
        let err = descriptor
            .validate_for_provider(ProviderKind::Plugin)
            .unwrap_err();
        assert!(matches!(
            err,
            InteractionValidationError::IconInvalid { .. }
        ));

        descriptor.icon = Some(String::new());
        let err = descriptor
            .validate_for_provider(ProviderKind::Plugin)
            .unwrap_err();
        assert!(matches!(
            err,
            InteractionValidationError::IconInvalid { .. }
        ));
    }

    #[test]
    fn validate_for_provider_accepts_missing_icon() {
        let descriptor = InteractionDescriptor::new(
            InteractionId::new("act").unwrap(),
            InteractionKind::MutationAction,
            "Action",
            InteractionTransport::ControllerLocal,
        );
        descriptor
            .validate_for_provider(ProviderKind::Plugin)
            .unwrap();
    }

    #[test]
    fn validate_for_provider_rejects_empty_submit_label() {
        let descriptor = InteractionDescriptor {
            submit_label: Some("   ".to_string()),
            ..InteractionDescriptor::new(
                InteractionId::new("act").unwrap(),
                InteractionKind::FormSubmit,
                "Save Settings",
                InteractionTransport::ProviderProxied,
            )
        };
        let err = descriptor
            .validate_for_provider(ProviderKind::Plugin)
            .unwrap_err();
        assert!(matches!(
            err,
            InteractionValidationError::SubmitLabelInvalid { .. }
        ));
    }

    #[test]
    fn validate_for_provider_rejects_submit_label_exceeding_50_chars() {
        let descriptor = InteractionDescriptor {
            submit_label: Some("a".repeat(51)),
            ..InteractionDescriptor::new(
                InteractionId::new("act").unwrap(),
                InteractionKind::FormSubmit,
                "Save",
                InteractionTransport::ProviderProxied,
            )
        };
        let err = descriptor
            .validate_for_provider(ProviderKind::Plugin)
            .unwrap_err();
        assert!(matches!(
            err,
            InteractionValidationError::SubmitLabelInvalid { .. }
        ));
    }

    #[test]
    fn validate_for_provider_accepts_valid_submit_label() {
        let descriptor = InteractionDescriptor {
            submit_label: Some("Connect".to_string()),
            ..InteractionDescriptor::new(
                InteractionId::new("act").unwrap(),
                InteractionKind::FormSubmit,
                "Save",
                InteractionTransport::ProviderProxied,
            )
        };
        descriptor
            .validate_for_provider(ProviderKind::Plugin)
            .unwrap();
    }

    #[test]
    fn provider_invocable_defaults_false_when_absent_on_wire() {
        let json = serde_json::json!({
            "interaction_id": "act",
            "kind": "data_load",
            "label": "Act",
            "transport": { "mode": "controller_local" }
        });
        let descriptor: InteractionDescriptor =
            serde_json::from_value(json).expect("deserialize without provider_invocable");
        assert!(!descriptor.provider_invocable);
        // Round-trip: default false is not serialized (skip_serializing_if).
        let value = serde_json::to_value(&descriptor).unwrap();
        assert!(value.get("provider_invocable").is_none());
    }

    #[test]
    fn validate_for_provider_rejects_provider_invocable_permissioned_service_interaction() {
        let mut descriptor = InteractionDescriptor::new(
            InteractionId::new("act").unwrap(),
            InteractionKind::DataLoad,
            "Act",
            InteractionTransport::ProviderProxied,
        );
        descriptor.required_permission = Some("update_hosts".to_string());
        descriptor.provider_invocable = true;
        let result = descriptor.validate_for_provider(ProviderKind::Service);
        assert!(matches!(
            result,
            Err(InteractionValidationError::ProviderInvocableForbiddenForServiceProviders { .. })
        ));
    }

    #[test]
    fn validate_for_provider_accepts_provider_invocable_for_plugin_and_unpermissioned_service() {
        let mut plugin_owned = InteractionDescriptor::new(
            InteractionId::new("act").unwrap(),
            InteractionKind::DataLoad,
            "Act",
            InteractionTransport::ControllerLocal,
        );
        plugin_owned.required_permission = Some("update_hosts".to_string());
        plugin_owned.provider_invocable = true;
        // matches! form — clippy::assertions_on_result_states is denied and this
        // tests mod may not carry the #![expect] header sibling mods use.
        assert!(matches!(
            plugin_owned.validate_for_provider(ProviderKind::Plugin),
            Ok(())
        ));

        let mut unpermissioned_service = InteractionDescriptor::new(
            InteractionId::new("act2").unwrap(),
            InteractionKind::DataLoad,
            "Act2",
            InteractionTransport::ProviderProxied,
        );
        unpermissioned_service.provider_invocable = true;
        assert!(matches!(
            unpermissioned_service.validate_for_provider(ProviderKind::Service),
            Ok(())
        ));
    }
}
