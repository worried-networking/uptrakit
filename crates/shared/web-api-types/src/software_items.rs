use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::provider_configs::CreateProviderConfigRequest;
use crate::validation::{Validate, ValidationError};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateSoftwareItemRequest {
    /// Display name (e.g. "Node.js").
    pub name: String,
    /// UUID of the provider config to use.
    pub provider_config_id: Option<Uuid>,
    /// Inline provider config to create (mutually exclusive with provider_config_id).
    pub provider_config: Option<CreateProviderConfigRequest>,
    /// Provider-specific identifier within the source. Defaults to "" if omitted.
    pub package_identifier: Option<String>,
    /// Provider-specific overrides merged onto the base ProviderConfig at resolution time.
    pub config_override: Option<serde_json::Value>,
    /// Whether version checking is active. Defaults to true.
    #[serde(default = "crate::default_enabled")]
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
    pub host_ids: Vec<Uuid>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemResponse {
    pub id: Uuid,
    pub name: String,
    pub provider_config_id: Uuid,
    pub provider_config_name: String,
    pub provider_type: String,
    pub package_identifier: String,
    pub config_override: Option<serde_json::Value>,
    pub enabled: bool,
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
    pub provider_config_id: Uuid,
    pub provider_config_name: String,
    pub provider_type: String,
    pub package_identifier: String,
    pub config_override: Option<serde_json::Value>,
    pub enabled: bool,
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

impl Validate for CreateSoftwareItemRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "name must not be empty".to_string(),
            });
        }

        match (&self.provider_config_id, &self.provider_config) {
            (Some(_), Some(_)) => {
                return Err(ValidationError {
                    field: "provider_config",
                    message: "exactly one of provider_config_id or provider_config must be provided, not both".to_string(),
                });
            }
            (None, None) => {
                return Err(ValidationError {
                    field: "provider_config",
                    message:
                        "exactly one of provider_config_id or provider_config must be provided"
                            .to_string(),
                });
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::ProviderType;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    fn valid_request_with_config_id() -> CreateSoftwareItemRequest {
        CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            provider_config_id: Some(sample_uuid()),
            provider_config: None,
            package_identifier: None,
            config_override: None,
            enabled: true,
        }
    }

    fn valid_request_with_inline_config() -> CreateSoftwareItemRequest {
        CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            provider_config_id: None,
            provider_config: Some(CreateProviderConfigRequest {
                name: "GitHub Releases".to_string(),
                provider_type: ProviderType::GithubReleases,
                config: serde_json::json!({}),
                enabled: true,
            }),
            package_identifier: Some("nodejs/node".to_string()),
            config_override: None,
            enabled: true,
        }
    }

    // ── CreateSoftwareItemRequest serialization ──────────────────────

    #[test]
    fn create_software_item_request_round_trip_with_config_id() {
        let req = valid_request_with_config_id();
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateSoftwareItemRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name, "Node.js");
        assert_eq!(deserialized.provider_config_id, Some(sample_uuid()));
        assert!(deserialized.provider_config.is_none());
        assert!(deserialized.enabled);
    }

    #[test]
    fn create_software_item_request_round_trip_with_inline_config() {
        let req = valid_request_with_inline_config();
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateSoftwareItemRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.provider_config_id.is_none());
        let config = deserialized
            .provider_config
            .expect("inline provider_config should be present");
        assert_eq!(config.name, "GitHub Releases");
        assert_eq!(
            deserialized.package_identifier.as_deref(),
            Some("nodejs/node")
        );
    }

    #[test]
    fn create_software_item_request_default_enabled_from_json() {
        let json = serde_json::json!({
            "name": "Test",
            "provider_config_id": "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
        });
        let req: CreateSoftwareItemRequest =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert!(req.enabled, "enabled should default to true");
    }

    // ── CreateSoftwareItemRequest validation ─────────────────────────

    #[test]
    fn validate_valid_request_with_config_id_passes() {
        let req = valid_request_with_config_id();
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_valid_request_with_inline_config_passes() {
        let req = valid_request_with_inline_config();
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_empty_name_fails() {
        let mut req = valid_request_with_config_id();
        req.name = "".to_string();
        let err = req
            .validate()
            .expect_err("empty name should fail validation");
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_whitespace_only_name_fails() {
        let mut req = valid_request_with_config_id();
        req.name = "   ".to_string();
        let err = req
            .validate()
            .expect_err("whitespace-only name should fail validation");
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_neither_config_provided_fails() {
        let req = CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            provider_config_id: None,
            provider_config: None,
            package_identifier: None,
            config_override: None,
            enabled: true,
        };
        let err = req
            .validate()
            .expect_err("neither config should fail validation");
        assert_eq!(err.field, "provider_config");
    }

    #[test]
    fn validate_both_configs_provided_fails() {
        let req = CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            provider_config_id: Some(sample_uuid()),
            provider_config: Some(CreateProviderConfigRequest {
                name: "GitHub Releases".to_string(),
                provider_type: ProviderType::GithubReleases,
                config: serde_json::json!({}),
                enabled: true,
            }),
            package_identifier: None,
            config_override: None,
            enabled: true,
        };
        let err = req
            .validate()
            .expect_err("both configs should fail validation");
        assert_eq!(err.field, "provider_config");
    }

    // ── SoftwareItemResponse ─────────────────────────────────────────

    #[test]
    fn software_item_response_round_trip() {
        use time::macros::datetime;
        let resp = SoftwareItemResponse {
            id: sample_uuid(),
            name: "Node.js".to_string(),
            provider_config_id: sample_uuid(),
            provider_config_name: "GitHub".to_string(),
            provider_type: "github_releases".to_string(),
            package_identifier: "nodejs/node".to_string(),
            config_override: Some(serde_json::json!({"key": "value"})),
            enabled: true,
            last_checked_at: Some(datetime!(2025-06-01 12:00:00 UTC)),
            host_count: 5,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 12:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareItemResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.name, "Node.js");
        assert_eq!(deserialized.host_count, 5);
        assert!(deserialized.config_override.is_some());
        assert!(deserialized.enabled);
    }

    #[test]
    fn software_item_response_none_optional_fields() {
        use time::macros::datetime;
        let resp = SoftwareItemResponse {
            id: sample_uuid(),
            name: "Test".to_string(),
            provider_config_id: sample_uuid(),
            provider_config_name: "Config".to_string(),
            provider_type: "github_releases".to_string(),
            package_identifier: "".to_string(),
            config_override: None,
            enabled: false,
            last_checked_at: None,
            host_count: 0,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareItemResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.config_override.is_none());
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
