use serde::{Deserialize, Serialize};

pub fn default_enabled() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateSoftwareItemRequest {
    /// Display name (e.g. "Node.js").
    pub name: String,
    /// UUID of the provider config to use.
    pub provider_config_id: String,
    /// Provider-specific identifier within the source. Defaults to "" if omitted.
    pub package_identifier: Option<String>,
    /// Provider-specific overrides merged onto the base ProviderConfig at resolution time.
    pub config_override: Option<serde_json::Value>,
    /// Whether version checking is active. Defaults to true.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateSoftwareItemRequest {
    pub name: Option<String>,
    pub package_identifier: Option<String>,
    /// Provider-specific overrides. Send null to clear, an object to replace.
    pub config_override: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssignHostsRequest {
    /// List of host UUIDs to assign.
    pub host_ids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemResponse {
    pub id: String,
    pub name: String,
    pub provider_config_id: String,
    pub provider_config_name: String,
    pub provider_type: String,
    pub package_identifier: String,
    pub config_override: Option<serde_json::Value>,
    pub enabled: bool,
    pub last_checked_at: Option<String>,
    pub host_count: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemDetailResponse {
    pub id: String,
    pub name: String,
    pub provider_config_id: String,
    pub provider_config_name: String,
    pub provider_type: String,
    pub package_identifier: String,
    pub config_override: Option<serde_json::Value>,
    pub enabled: bool,
    pub last_checked_at: Option<String>,
    pub host_count: u64,
    pub created_at: String,
    pub updated_at: String,
    pub hosts: Vec<SoftwareItemHostSummary>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemHostSummary {
    pub host_id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub installed_version: Option<String>,
    pub installed_version_detected_at: Option<String>,
    pub linked_at: String,
}
