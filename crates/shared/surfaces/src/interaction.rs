use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BuiltInApiOperationId, FormUiDescriptor, InteractionId, ProviderKind, SchemaContract};

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
    DirectBuiltInApi { operation_id: BuiltInApiOperationId },
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
        "provider-authored interactions cannot use direct built-in API transport (interaction `{interaction_id}`)"
    )]
    DirectBuiltInApiForbiddenForProvider { interaction_id: InteractionId },
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
        }
    }

    /// Validates provider-specific interaction contract rules.
    ///
    /// # Errors
    /// Returns
    /// [`InteractionValidationError::DirectBuiltInApiForbiddenForProvider`]
    /// when a non-built-in provider uses `direct_built_in_api` transport.
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
    pub fn validate_for_provider(
        &self,
        provider_kind: ProviderKind,
    ) -> Result<(), InteractionValidationError> {
        if provider_kind != ProviderKind::BuiltIn
            && matches!(
                self.transport,
                InteractionTransport::DirectBuiltInApi { .. }
            )
        {
            return Err(
                InteractionValidationError::DirectBuiltInApiForbiddenForProvider {
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
}
