use serde::{Deserialize, Serialize};
use uptrakit_shared_types::ProviderType;
use uuid::Uuid;

pub fn default_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateProviderConfigRequest {
    pub name: String,
    /// Provider type identifier (e.g. `github_releases`, `proxmox_helper_scripts`).
    pub provider_type: ProviderType,
    /// Provider-specific configuration blob.
    pub config: serde_json::Value,
    /// Whether the config is enabled. Defaults to true.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateProviderConfigRequest {
    pub name: Option<String>,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProviderConfigResponse {
    pub id: Uuid,
    pub name: String,
    pub provider_type: String,
    /// Provider-specific configuration with secrets masked.
    pub config: serde_json::Value,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}
