use serde::{Deserialize, Serialize};
use uptrakit_internal_wire::extension::{ActionDef, ExtensionManifest};
use uptrakit_shared_types::PluginTypeId;
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

/// Response for a single extension in the list.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionResponse {
    /// The extension manifest.
    #[serde(flatten)]
    pub manifest: ExtensionManifest,
    /// Resolved action catalogue for this extension's source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionDef>,
    /// Plugin type that owns this extension when it is backed by a compiled-in plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_plugin_type_id: Option<PluginTypeId>,
    /// Number of connected service instances providing this extension.
    pub provider_count: usize,
}

/// Information about a service instance providing an extension.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionProviderInfo {
    pub service_id: Uuid,
    pub service_label: String,
    pub hostname: Option<String>,
    /// Base64-encoded uncompressed P-256 public key for ECIES encryption of
    /// sensitive parameters. `None` if the service did not provide a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_public_key: Option<String>,
}

/// Request body for invoking an extension action.
#[derive(Debug, Serialize, Deserialize)]
pub struct InvokeExtensionActionRequest {
    #[serde(default)]
    pub params: serde_json::Value,
    /// ECIES sealed-box ciphertext (base64) containing sensitive parameters.
    ///
    /// Encrypted by the client with the target service's P-256 public key.
    /// The controller passes this through opaquely.
    #[serde(default)]
    pub sensitive_params: Option<SecretString>,
}

/// Query parameters for invoking an extension action.
#[derive(Debug, Deserialize)]
pub struct InvokeExtensionActionQuery {
    pub service_id: Option<Uuid>,
}
