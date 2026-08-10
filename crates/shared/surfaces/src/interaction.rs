use serde::{Deserialize, Serialize};
use thiserror::Error;
use uptrakit_shared_macros::wire_safe_enum;

use crate::{FormUiDescriptor, InteractionId, ParamFieldDescriptor, ProviderKind, SchemaContract};

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

wire_safe_enum! {
    /// HTTP method a surface interaction is dispatched with (REST method model, B1).
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub enum InteractionHttpMethod {
        Get => "get",
        // `#[serde(default)]` on `InteractionDescriptor::http_method` requires
        // `InteractionHttpMethod: Default`; `#[default]` marks the variant the
        // derive picks (the macro forwards per-variant attributes verbatim).
        #[default]
        Post => "post",
        Put => "put",
        Delete => "delete",
    }
    parse_error = ParseInteractionHttpMethodError("invalid interaction http method");
}

/// Known variants for exhaustive test iteration (mirrors the
/// `ProviderEncryptionAlgorithm` precedent in `protocol.rs`).
pub const KNOWN_INTERACTION_HTTP_METHODS: &[InteractionHttpMethod] = &[
    InteractionHttpMethod::Get,
    InteractionHttpMethod::Post,
    InteractionHttpMethod::Put,
    InteractionHttpMethod::Delete,
];

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
    /// HTTP method this interaction is dispatched with. Wire default is POST
    /// (providers that predate the method model omit the field); DataLoads
    /// normalize to GET at admission regardless of the declared value.
    #[serde(default)]
    pub http_method: InteractionHttpMethod,
    pub label: String,
    /// Canonical action string (`resource:verb`) required to view/use this
    /// interaction; parsed to `Action` at admission.
    #[serde(
        default,
        alias = "required_permission",
        skip_serializing_if = "Option::is_none"
    )]
    pub required_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<SchemaContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_schema: Option<SchemaContract>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sensitive_fields: Vec<String>,
    /// Opt-in per-field param declarations (GET query typing + body validation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamFieldDescriptor>,
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
    /// this interaction even when `required_action` is set. Fail-closed:
    /// absent on the wire deserializes to `false`. Honored only for
    /// `Plugin`/`BuiltIn`-registered interactions — see `validate_for_provider`.
    /// For `Service`-kind providers the flag is additionally rejected at
    /// surface-level admission when the home surface descriptor carries
    /// `required_action` (enforced in `protocol.rs`'s registration
    /// validation, not here).
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
        "interaction `{interaction_id}` sets provider_invocable with a required_action — not allowed for service-registered surfaces"
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
            http_method: InteractionHttpMethod::default(),
            label: label.into(),
            transport,
            required_action: None,
            input_schema: None,
            result_schema: None,
            sensitive_fields: vec![],
            params: Vec::new(),
            timeout_seconds: None,
            confirmation: None,
            workflow_steps: vec![],
            form_ui: None,
            icon: None,
            submit_label: None,
            provider_invocable: false,
        }
    }

    /// Declare a non-default dispatch method (PUT/DELETE mutations).
    #[must_use]
    pub fn with_http_method(mut self, http_method: InteractionHttpMethod) -> Self {
        self.http_method = http_method;
        self
    }

    /// Declare per-field param descriptors (GET query typing + body validation).
    #[must_use]
    pub fn with_params(mut self, params: Vec<ParamFieldDescriptor>) -> Self {
        self.params = params;
        self
    }

    /// The method the framework dispatches with: DataLoads are GET-only (B1);
    /// any other kind uses the declared method.
    pub fn effective_http_method(&self) -> InteractionHttpMethod {
        if self.kind == InteractionKind::DataLoad {
            InteractionHttpMethod::Get
        } else {
            self.http_method.clone()
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
    /// that also declares `required_action`. The descriptor-level companion
    /// rule (rejecting `provider_invocable` when the *surface descriptor*
    /// carries `required_action`) lives in
    /// `protocol.rs::validate_interaction_provider_rules`, because this
    /// method never sees the surface descriptor.
    pub fn validate_for_provider(
        &self,
        provider_kind: ProviderKind,
    ) -> Result<(), InteractionValidationError> {
        if self.provider_invocable
            && self.required_action.is_some()
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
        descriptor.required_action = Some("update_hosts".to_string());
        descriptor.provider_invocable = true;
        let result = descriptor.validate_for_provider(ProviderKind::Service);
        assert!(matches!(
            result,
            Err(InteractionValidationError::ProviderInvocableForbiddenForServiceProviders { .. })
        ));
    }

    #[test]
    fn http_method_defaults_to_post_when_absent_on_wire() {
        // Old-peer shape: descriptor JSON without http_method.
        let json = serde_json::json!({
            "interaction_id": "save",
            "kind": "mutation_action",
            "label": "Save",
            "transport": { "mode": "controller_local" }
        });
        let descriptor: InteractionDescriptor = serde_json::from_value(json).expect("deserialize");
        assert_eq!(descriptor.http_method, InteractionHttpMethod::Post);
    }

    #[test]
    fn effective_http_method_normalizes_dataload_to_get() {
        let json = serde_json::json!({
            "interaction_id": "list",
            "kind": "data_load",
            "label": "List",
            "transport": { "mode": "provider_proxied" }
        });
        let descriptor: InteractionDescriptor = serde_json::from_value(json).expect("deserialize");
        assert_eq!(descriptor.http_method, InteractionHttpMethod::Post); // raw wire default
        assert_eq!(
            descriptor.effective_http_method(),
            InteractionHttpMethod::Get
        );
    }

    #[cfg(feature = "schema")]
    mod schema_tests {
        use super::*;

        fn assert_open_string_schema<T: schemars::JsonSchema>(known: &[&str]) {
            let schema = schemars::schema_for!(T);
            let value = serde_json::to_value(&schema).expect("schema to JSON");
            assert_eq!(value["type"], "string");
            assert!(
                value.get("enum").is_none(),
                "must be an open string schema, found closed enum list: {value}"
            );
            let desc = value["description"].as_str().expect("description present");
            for k in known {
                assert!(
                    desc.contains(k),
                    "known value {k} missing from description: {desc}"
                );
            }
        }

        /// Covers the `wire_safe_enum!` schemars arm: verifies that macro-generated
        /// `JsonSchema` impls produce open string schemas, not closed enum lists.
        #[test]
        fn interaction_http_method_schema_is_open_string_with_known_values() {
            assert_open_string_schema::<InteractionHttpMethod>(&["get", "post"]);
        }
    }

    #[test]
    fn http_method_round_trips_wire_string() {
        assert_eq!(
            InteractionHttpMethod::from("put".to_string()),
            InteractionHttpMethod::Put
        );
        assert_eq!(InteractionHttpMethod::Put.as_str(), "put");
        assert!(matches!(
            InteractionHttpMethod::from("patch".to_string()),
            InteractionHttpMethod::Other(_)
        ));
        "patch".parse::<InteractionHttpMethod>().unwrap_err(); // strict FromStr
    }

    #[test]
    fn validate_for_provider_accepts_provider_invocable_for_plugin_and_unpermissioned_service() {
        let mut plugin_owned = InteractionDescriptor::new(
            InteractionId::new("act").unwrap(),
            InteractionKind::DataLoad,
            "Act",
            InteractionTransport::ControllerLocal,
        );
        plugin_owned.required_action = Some("update_hosts".to_string());
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

    #[test]
    fn required_action_accepts_legacy_key_via_alias() {
        // A stale-satellite payload lands in required_action; the (legacy)
        // value then dies at the admission Action parse — never a silent None.
        let json = serde_json::json!({
            "interaction_id": "act",
            "kind": "data_load",
            "label": "Act",
            "transport": { "mode": "controller_local" },
            "required_permission": "update_hosts",
        });
        let descriptor: InteractionDescriptor =
            serde_json::from_value(json).expect("alias must deserialize");
        assert_eq!(descriptor.required_action.as_deref(), Some("update_hosts"));
    }

    #[test]
    fn required_action_rejects_dual_key_payload() {
        // serde derive: an alias shares the field's slot, so a second
        // occurrence is duplicate_field — there is no last-wins.
        let json = r#"{
            "interaction_id": "act",
            "kind": "data_load",
            "label": "Act",
            "transport": { "mode": "controller_local" },
            "required_action": "hosts:update",
            "required_permission": "update_hosts"
        }"#;
        // expect_err alone pins the semantics (dual key fails, no last-wins);
        // do not assert serde_json's message text (upstream-behavior coupling).
        serde_json::from_str::<InteractionDescriptor>(json).expect_err("dual key must fail");
    }

    #[test]
    fn required_action_serializes_under_the_new_key_only() {
        let mut descriptor = InteractionDescriptor::new(
            InteractionId::new("act").unwrap(),
            InteractionKind::DataLoad,
            "Act",
            InteractionTransport::ControllerLocal,
        );
        descriptor.required_action = Some("hosts:update".to_string());

        let value = serde_json::to_value(&descriptor).expect("serialize");
        assert!(value.get("required_action").is_some());
        assert!(value.get("required_permission").is_none());
    }
}
