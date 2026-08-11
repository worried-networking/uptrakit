use serde::{Deserialize, Serialize};
use uptrakit_wire::surfaces::{self, SurfaceDescriptor};
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// Query parameters for listing registered surfaces.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListSurfacesQuery {
    /// Return only surfaces registered in this slot.
    #[serde(default)]
    pub slot: Option<String>,
    /// Page alias filter (`settings`, `software`, `hosts`, `surfaces`).
    #[serde(default)]
    pub page: Option<String>,
}

/// Surface list item returned by `/api/v1/surfaces`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceResponse {
    /// Flattened surface descriptor (wire-defined shape; free-form in the spec).
    #[serde(flatten)]
    #[cfg_attr(feature = "openapi", schema(value_type = serde_json::Value))]
    pub descriptor: SurfaceDescriptor,
    pub provider_count: usize,
}

/// Tenant-compatibility/availability state for a targeted provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SurfaceProviderAvailability {
    Available,
    Disconnected,
    IncompatibleTenant,
}

/// Provider information returned for a targeted surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceProviderInfo {
    pub provider_id: String,
    pub display_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_id: Option<Uuid>,
    pub availability: SurfaceProviderAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<serde_json::Value>))]
    pub encryption_metadata: Option<surfaces::ProviderEncryptionMetadata>,
}

/// Surface read payload used by frontend route rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceReadResponse {
    /// Surface descriptor (wire-defined shape; free-form in the spec).
    #[cfg_attr(feature = "openapi", schema(value_type = serde_json::Value))]
    pub descriptor: SurfaceDescriptor,
    /// Interaction descriptors (wire-defined shape; free-form in the spec).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<serde_json::Value>))]
    pub interactions: Vec<surfaces::InteractionDescriptor>,
    /// Data-source descriptors (wire-defined shape; free-form in the spec).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<serde_json::Value>))]
    pub data_sources: Vec<surfaces::DataSourceDescriptor>,
}

/// Query parameters for GET-origin surface interaction invocation.
///
/// Documentation-only: the handler reads raw query pairs (to support
/// undeclared provider-defined keys) rather than deserializing through this
/// struct directly. It exists purely to drive the OpenAPI `params(...)`
/// declaration (ADR-0025) for the method-mapped REST route family: reserved
/// keys (`page`/`per_page`) coerce to numbers; `target_provider_id` and
/// `timeout_seconds` are envelope keys stripped before provider dispatch and
/// never reach provider `params`.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
pub struct ReadSurfaceInteractionQuery {
    /// Explicit provider to target; required for multi-provider surfaces.
    #[serde(default)]
    pub target_provider_id: Option<String>,
    /// Overrides the provider's default timeout, in seconds.
    #[serde(default)]
    pub timeout_seconds: Option<u16>,
    /// Reserved typed key — coerced to a JSON number.
    #[serde(default)]
    pub page: Option<u64>,
    /// Reserved typed key — coerced to a JSON number.
    #[serde(default)]
    pub per_page: Option<u64>,
}

/// Request body for invoking a surface interaction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvokeSurfaceInteractionRequest {
    /// Interaction parameters (free-form JSON object).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = serde_json::Value))]
    pub params: serde_json::Map<String, serde_json::Value>,
    /// Sealed-box-encrypted sensitive parameters (wire-defined shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<serde_json::Value>))]
    pub encrypted_sensitive_params: Option<surfaces::EncryptedSensitiveParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_provider_id: Option<String>,
    /// Optional idempotency key. If omitted, the server generates one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Optional timeout override for this invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u16>,
}

impl Validate for InvokeSurfaceInteractionRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        // No format/length invariants beyond field types; capability/existence checks are handler-side.
        Ok(())
    }
}

impl crate::validation::sealed::Sealed for InvokeSurfaceInteractionRequest {}

impl crate::validation::RoutingEnvelope for InvokeSurfaceInteractionRequest {
    fn routing_envelope(&self) -> crate::validation::InvokeRoutingEnvelope {
        crate::validation::InvokeRoutingEnvelope {
            target_provider_id: self.target_provider_id.clone(),
            timeout_seconds: self.timeout_seconds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_surface_interaction_request_validate_is_ok() {
        InvokeSurfaceInteractionRequest::default()
            .validate()
            .expect("InvokeSurfaceInteractionRequest::default() should validate");
    }

    #[test]
    fn invoke_request_validate_is_unconditionally_ok_canary() {
        // Canary (spec 2026-08-06 item 5): validate() is unconditionally Ok
        // today, so the 403-before-semantic-400 dispatch ordering has no
        // discriminating test. The author of the FIRST real Validate rule
        // breaks this test and must then add that discriminating test (a
        // well-formed body violating the rule, sent by an unauthorized
        // caller, must 403 — see the choke-point comment in
        // web-api routes/surfaces.rs::dispatch_surface_interaction).
        let populated = InvokeSurfaceInteractionRequest {
            params: serde_json::Map::from_iter([(
                "k".to_string(),
                serde_json::Value::String("v".to_string()),
            )]),
            encrypted_sensitive_params: None,
            target_provider_id: Some("provider".to_string()),
            idempotency_key: Some("key".to_string()),
            timeout_seconds: Some(1),
        };
        populated
            .validate()
            .expect("validate() must stay unconditionally Ok until the discriminating test exists");
    }

    #[test]
    fn routing_envelope_projects_only_the_envelope_fields() {
        let req = InvokeSurfaceInteractionRequest {
            target_provider_id: Some("p1".to_string()),
            timeout_seconds: Some(30),
            ..Default::default()
        };
        let env = crate::validation::RoutingEnvelope::routing_envelope(&req);
        assert_eq!(env.target_provider_id.as_deref(), Some("p1"));
        assert_eq!(env.timeout_seconds, Some(30));
    }
}
