use serde::{Deserialize, Serialize};
use uptrakit_internal_wire::extension::ExtensionManifest;
use uuid::Uuid;

/// Response for a single extension in the list.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionResponse {
    /// The extension manifest.
    #[serde(flatten)]
    pub manifest: ExtensionManifest,
    /// Number of connected service instances providing this extension.
    pub provider_count: usize,
}

/// Information about a service instance providing an extension.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionProviderInfo {
    pub service_id: Uuid,
    pub service_label: String,
    pub hostname: Option<String>,
}

/// Request body for invoking an extension action.
#[derive(Debug, Deserialize)]
pub struct InvokeExtensionActionRequest {
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Query parameters for invoking an extension action.
#[derive(Debug, Deserialize)]
pub struct InvokeExtensionActionQuery {
    pub service_id: Option<Uuid>,
}
