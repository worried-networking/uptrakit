use serde::{Deserialize, Serialize};
use uptrakit_internal_wire::surfaces::{self, SurfaceDescriptor};
use uuid::Uuid;

/// Query parameters for listing registered surfaces.
#[derive(Debug, Clone, Deserialize)]
pub struct ListSurfacesQuery {
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub page: Option<String>,
}

/// Surface list item returned by `/api/v1/surfaces`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceResponse {
    #[serde(flatten)]
    pub descriptor: SurfaceDescriptor,
    pub provider_count: usize,
}

/// Tenant-compatibility/availability state for a targeted provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceProviderAvailability {
    Available,
    Disconnected,
    IncompatibleTenant,
}

/// Provider information returned for a targeted surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceProviderInfo {
    pub provider_id: String,
    pub display_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_id: Option<Uuid>,
    pub availability: SurfaceProviderAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_metadata: Option<surfaces::ProviderEncryptionMetadata>,
}

/// Controller-owned rollout signal for the shared surface runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceRuntimeStatusResponse {
    pub active: bool,
}

/// Request body for invoking a surface interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeSurfaceInteractionRequest {
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
