use serde::{Deserialize, Serialize};

use crate::provider_configs::CreateProviderConfigRequest;

pub fn default_enabled() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateSoftwareItemRequest {
    /// Display name (e.g. "Node.js").
    pub name: String,
    /// UUID of the provider config to use.
    pub provider_config_id: Option<String>,
    /// Inline provider config to create (mutually exclusive with provider_config_id).
    pub provider_config: Option<CreateProviderConfigRequest>,
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
    pub last_updated_at: Option<String>,
    pub linked_at: String,
}

/// Status returned when triggering an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TriggerUpdateStatus {
    /// Agent connected, update sent.
    Pending,
    /// Agent offline, will deliver on reconnect.
    Queued,
}

/// Release asset information for triggering an update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReleaseAssetInfoRequest {
    pub name: String,
    pub download_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Release information for triggering an update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReleaseInfoRequest {
    pub tag: String,
    pub release_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<ReleaseAssetInfoRequest>,
}

/// Request body for triggering a software update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerUpdateRequest {
    /// Target version to update to.
    pub to_version: String,
    /// Optional release information (for providers that need it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_info: Option<ReleaseInfoRequest>,
}

/// Response when triggering a software update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerUpdateResponse {
    pub update_history_id: String,
    pub status: TriggerUpdateStatus,
}

/// Response when triggering a version check for a software item.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerVersionCheckResponse {
    /// Number of agents that were notified.
    pub agents_notified: u32,
    /// Human-readable status message.
    pub message: String,
}
