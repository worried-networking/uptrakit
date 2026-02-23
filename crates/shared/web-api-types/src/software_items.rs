use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::SoftwareDiscoveryState;
use uuid::Uuid;

use crate::pagination::PaginationParams;
use crate::provider_configs::CreateProviderConfigRequest;
use crate::validation::{Validate, ValidationError};

/// Create a new software item (catalog entry only — no provider coupling).
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateSoftwareItemRequest {
    /// Display name (e.g. "1Password").
    pub name: String,
    /// Whether version checking is active. Defaults to true.
    #[serde(default = "crate::default_enabled")]
    pub enabled: bool,
}

/// Partial update for a software item. Only `name` and `enabled` are updatable.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateSoftwareItemRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
}

/// Per-host provider assignment used when assigning hosts to a software item.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostSoftwareAssignment {
    pub host_id: Uuid,
    /// UUID of an existing provider config to use.
    pub provider_config_id: Option<Uuid>,
    /// Inline provider config to create (mutually exclusive with `provider_config_id`).
    pub provider_config: Option<CreateProviderConfigRequest>,
    /// Provider-specific package identifier. Defaults to `""` if omitted.
    pub package_identifier: Option<String>,
    /// Provider-specific overrides merged onto the base config at resolution time.
    pub config_override: Option<serde_json::Value>,
}

/// Assign one or more hosts to a software item, each with its own provider info.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssignHostsRequest {
    pub host_assignments: Vec<HostSoftwareAssignment>,
}

/// Update the provider info for an existing host–software-item assignment.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateHostAssignmentRequest {
    /// UUID of an existing provider config to use.
    pub provider_config_id: Option<Uuid>,
    /// Inline provider config to create (mutually exclusive with `provider_config_id`).
    pub provider_config: Option<CreateProviderConfigRequest>,
    pub package_identifier: Option<String>,
    /// Send `null` to clear the override, an object to replace it.
    pub config_override: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemResponse {
    pub id: Uuid,
    pub name: String,
    /// Distinct provider type strings from all active host assignments (for display in lists).
    pub provider_types: Vec<String>,
    pub enabled: bool,
    /// Discovery state for auto-discovered items. `None` means manually created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_state: Option<SoftwareDiscoveryState>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_checked_at: Option<OffsetDateTime>,
    pub host_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemDetailResponse {
    pub id: Uuid,
    pub name: String,
    /// Distinct provider type strings from all active host assignments.
    pub provider_types: Vec<String>,
    pub enabled: bool,
    /// Discovery state for auto-discovered items. `None` means manually created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_state: Option<SoftwareDiscoveryState>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_checked_at: Option<OffsetDateTime>,
    pub host_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
    pub hosts: Vec<SoftwareItemHostSummary>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemHostSummary {
    pub host_id: Uuid,
    pub hostname: String,
    pub friendly_name: String,
    pub provider_config_id: Uuid,
    pub provider_config_name: String,
    pub provider_type: String,
    pub package_identifier: String,
    pub config_override: Option<serde_json::Value>,
    pub installed_version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub installed_version_detected_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_updated_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub linked_at: OffsetDateTime,
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

impl std::fmt::Display for TriggerUpdateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => f.write_str("pending"),
            Self::Queued => f.write_str("queued"),
        }
    }
}

/// Release asset information for triggering an update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReleaseAssetInfoRequest {
    pub name: String,
    pub download_url: String,
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
    pub release_info: Option<ReleaseInfoRequest>,
}

/// Response when triggering a software update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerUpdateResponse {
    pub update_history_id: Uuid,
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

/// Query parameters for listing software items, extending pagination with an optional
/// discovery state filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListSoftwareItemsParams {
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
    /// Filter by discovery state. Valid values: `"pending"`, `"approved"`.
    /// Omit to return all items regardless of discovery state.
    pub discovery_state: Option<SoftwareDiscoveryState>,
}

impl ListSoftwareItemsParams {
    /// Convert the pagination fields to a [`PaginationParams`] for resolution.
    pub fn pagination(&self) -> PaginationParams {
        PaginationParams { page: self.page, per_page: self.per_page }
    }
}

impl Validate for CreateSoftwareItemRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "name must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    fn valid_create_request() -> CreateSoftwareItemRequest {
        CreateSoftwareItemRequest {
            name: "1Password".to_string(),
            enabled: true,
        }
    }

    // ── CreateSoftwareItemRequest serialization ──────────────────────

    #[test]
    fn create_software_item_request_round_trip() {
        let req = valid_create_request();
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateSoftwareItemRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name, "1Password");
        assert!(deserialized.enabled);
    }

    #[test]
    fn create_software_item_request_default_enabled_from_json() {
        let json = serde_json::json!({ "name": "Test" });
        let req: CreateSoftwareItemRequest =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert!(req.enabled, "enabled should default to true");
    }

    // ── CreateSoftwareItemRequest validation ─────────────────────────

    #[test]
    fn validate_valid_request_passes() {
        let req = valid_create_request();
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_empty_name_fails() {
        let req = CreateSoftwareItemRequest { name: "".to_string(), enabled: true };
        let err = req.validate().expect_err("empty name should fail validation");
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_whitespace_only_name_fails() {
        let req = CreateSoftwareItemRequest { name: "   ".to_string(), enabled: true };
        let err = req
            .validate()
            .expect_err("whitespace-only name should fail validation");
        assert_eq!(err.field, "name");
    }

    // ── AssignHostsRequest round-trip ──────────────────────────────

    #[test]
    fn assign_hosts_request_round_trip() {
        use uptrakit_shared_types::ProviderType;
        let req = AssignHostsRequest {
            host_assignments: vec![
                HostSoftwareAssignment {
                    host_id: sample_uuid(),
                    provider_config_id: Some(sample_uuid()),
                    provider_config: None,
                    package_identifier: Some("1password".to_string()),
                    config_override: None,
                },
                HostSoftwareAssignment {
                    host_id: Uuid::nil(),
                    provider_config_id: None,
                    provider_config: Some(crate::provider_configs::CreateProviderConfigRequest {
                        name: "Homebrew Casks".to_string(),
                        provider_type: ProviderType::Homebrew,
                        config: serde_json::json!({"package_type": "cask"}),
                        enabled: true,
                    }),
                    package_identifier: Some("1password-cli".to_string()),
                    config_override: None,
                },
            ],
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: AssignHostsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.host_assignments.len(), 2);
        assert_eq!(deserialized.host_assignments[0].host_id, sample_uuid());
        assert_eq!(
            deserialized.host_assignments[0].package_identifier.as_deref(),
            Some("1password")
        );
        assert!(deserialized.host_assignments[1].provider_config.is_some());
    }

    // ── SoftwareItemResponse ─────────────────────────────────────────

    #[test]
    fn software_item_response_round_trip() {
        use time::macros::datetime;
        let resp = SoftwareItemResponse {
            id: sample_uuid(),
            name: "1Password".to_string(),
            provider_types: vec!["homebrew".to_string(), "github_releases".to_string()],
            enabled: true,
            discovery_state: None,
            last_checked_at: Some(datetime!(2025-06-01 12:00:00 UTC)),
            host_count: 5,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 12:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareItemResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.name, "1Password");
        assert_eq!(deserialized.host_count, 5);
        assert_eq!(deserialized.provider_types.len(), 2);
        assert!(deserialized.enabled);
    }

    #[test]
    fn software_item_response_empty_provider_types() {
        use time::macros::datetime;
        let resp = SoftwareItemResponse {
            id: sample_uuid(),
            name: "Test".to_string(),
            provider_types: vec![],
            enabled: false,
            discovery_state: None,
            last_checked_at: None,
            host_count: 0,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareItemResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.provider_types.is_empty());
        assert!(deserialized.last_checked_at.is_none());
        assert!(!deserialized.enabled);
    }

    // ── TriggerUpdateRequest / TriggerUpdateResponse ─────────────────

    #[test]
    fn trigger_update_request_round_trip() {
        let req = TriggerUpdateRequest {
            to_version: "2.0.0".to_string(),
            release_info: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: TriggerUpdateRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.to_version, "2.0.0");
        assert!(deserialized.release_info.is_none());
    }

    #[test]
    fn trigger_update_request_with_release_info() {
        let req = TriggerUpdateRequest {
            to_version: "3.0.0".to_string(),
            release_info: Some(ReleaseInfoRequest {
                tag: "v3.0.0".to_string(),
                release_url: "https://github.com/example/repo/releases/v3.0.0".to_string(),
                assets: vec![ReleaseAssetInfoRequest {
                    name: "binary.tar.gz".to_string(),
                    download_url: "https://example.com/binary.tar.gz".to_string(),
                    size: Some(1024),
                }],
            }),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: TriggerUpdateRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        let info = deserialized
            .release_info
            .expect("release_info should be present");
        assert_eq!(info.tag, "v3.0.0");
        assert_eq!(info.assets.len(), 1);
        assert_eq!(info.assets[0].size, Some(1024));
    }

    #[test]
    fn trigger_update_response_round_trip() {
        let resp = TriggerUpdateResponse {
            update_history_id: sample_uuid(),
            status: TriggerUpdateStatus::Pending,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerUpdateResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.update_history_id, sample_uuid());
        assert_eq!(deserialized.status, TriggerUpdateStatus::Pending);
    }

    #[test]
    fn trigger_update_response_queued_status() {
        let resp = TriggerUpdateResponse {
            update_history_id: sample_uuid(),
            status: TriggerUpdateStatus::Queued,
        };
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        assert_eq!(
            json_value.get("status").and_then(|v| v.as_str()),
            Some("queued")
        );
    }

    // ── TriggerVersionCheckResponse ──────────────────────────────────

    #[test]
    fn trigger_version_check_response_round_trip() {
        let resp = TriggerVersionCheckResponse {
            agents_notified: 3,
            message: "Version check triggered for 3 agents".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerVersionCheckResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.agents_notified, 3);
        assert_eq!(deserialized.message, "Version check triggered for 3 agents");
    }

    #[test]
    fn trigger_version_check_response_zero_agents() {
        let resp = TriggerVersionCheckResponse {
            agents_notified: 0,
            message: "No agents connected".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerVersionCheckResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.agents_notified, 0);
    }

    // ── TriggerUpdateStatus Display ──────────────────────────────────

    #[test]
    fn trigger_update_status_display() {
        assert_eq!(TriggerUpdateStatus::Pending.to_string(), "pending");
        assert_eq!(TriggerUpdateStatus::Queued.to_string(), "queued");
    }
}
