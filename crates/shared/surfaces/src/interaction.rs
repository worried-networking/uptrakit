use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BuiltInApiOperationId, FormUiDescriptor, InteractionId, ProviderKind, SchemaContract};

pub const MIN_INTERACTION_TIMEOUT_SECONDS: u16 = 1;
pub const MAX_INTERACTION_TIMEOUT_SECONDS: u16 = 300;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationSeverity {
    Info,
    Warning,
    Danger,
}

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
}

impl InteractionDescriptor {
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

        Ok(())
    }
}
