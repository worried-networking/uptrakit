use serde::{Deserialize, Serialize};
use uptrakit_wire::surfaces::{self, SurfaceDescriptor};
use uuid::Uuid;

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

/// Request body for invoking a surface interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
