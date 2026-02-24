use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::ProviderType;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateProviderConfigRequest {
    pub name: String,
    /// Provider type identifier (e.g. `github_releases`, `proxmox_helper_scripts`).
    pub provider_type: ProviderType,
    /// Provider-specific configuration blob.
    pub config: serde_json::Value,
    /// Whether the config is enabled. Defaults to true.
    #[serde(default = "crate::default_enabled")]
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
    pub provider_type: ProviderType,
    /// Provider-specific configuration with secrets masked.
    pub config: serde_json::Value,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

impl Validate for CreateProviderConfigRequest {
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

    // ── CreateProviderConfigRequest ──────────────────────────────────

    #[test]
    fn create_request_round_trip() {
        let req = CreateProviderConfigRequest {
            name: "my-github".to_string(),
            provider_type: ProviderType::GithubReleases,
            config: serde_json::json!({"owner": "org", "repo": "app"}),
            enabled: true,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: CreateProviderConfigRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.name, "my-github");
        assert_eq!(de.provider_type, ProviderType::GithubReleases);
        assert!(de.enabled);
    }

    #[test]
    fn create_request_enabled_defaults_to_true() {
        let json = r#"{"name":"test","provider_type":"github_releases","config":{}}"#;
        let de: CreateProviderConfigRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert!(de.enabled, "enabled should default to true");
    }

    #[test]
    fn create_request_validate_rejects_empty_name() {
        let req = CreateProviderConfigRequest {
            name: "   ".to_string(),
            provider_type: ProviderType::GithubReleases,
            config: serde_json::json!({}),
            enabled: true,
        };
        let err = req
            .validate()
            .expect_err("should reject whitespace-only name");
        assert_eq!(err.field, "name");
    }

    #[test]
    fn create_request_validate_accepts_valid() {
        let req = CreateProviderConfigRequest {
            name: "my-provider".to_string(),
            provider_type: ProviderType::GithubReleases,
            config: serde_json::json!({}),
            enabled: true,
        };
        assert!(req.validate().is_ok());
    }

    // ── UpdateProviderConfigRequest ──────────────────────────────────

    #[test]
    fn update_request_round_trip_all_fields() {
        let req = UpdateProviderConfigRequest {
            name: Some("renamed".to_string()),
            config: Some(serde_json::json!({"key": "val"})),
            enabled: Some(false),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdateProviderConfigRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.name.as_deref(), Some("renamed"));
        assert_eq!(de.enabled, Some(false));
    }

    #[test]
    fn update_request_round_trip_none_fields() {
        let req = UpdateProviderConfigRequest {
            name: None,
            config: None,
            enabled: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdateProviderConfigRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(de.name.is_none());
        assert!(de.config.is_none());
        assert!(de.enabled.is_none());
    }

    // ── ProviderConfigResponse ───────────────────────────────────────

    #[test]
    fn response_round_trip() {
        use time::macros::datetime;
        let resp = ProviderConfigResponse {
            id: sample_uuid(),
            name: "docker-hub".to_string(),
            provider_type: ProviderType::Docker,
            config: serde_json::json!({}),
            enabled: true,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: ProviderConfigResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.id, sample_uuid());
        assert_eq!(de.name, "docker-hub");
        assert_eq!(de.provider_type, ProviderType::Docker);
        assert!(de.enabled);
    }
}
