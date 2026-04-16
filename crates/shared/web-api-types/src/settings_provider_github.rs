//! Request/response types for global GitHub provider settings.

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

/// Response body for `GET /api/v1/global-settings/providers/github`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GitHubProviderSettingsResponse {
    /// Custom API base URL override for GitHub Enterprise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    /// Whether an auth token is currently configured.
    pub has_auth_token: bool,
    /// Masked token sentinel (`***`) when a token exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
}

/// Request body for `PUT /api/v1/global-settings/providers/github`.
///
/// `auth_token` tri-state semantics:
/// - omit => keep current
/// - `"***"` => keep current
/// - `""` => clear
/// - non-empty => replace
///
/// `api_base_url` semantics:
/// - omit => keep current
/// - `""` => clear
/// - non-empty => validate and replace
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateGitHubProviderSettingsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
}

impl Validate for UpdateGitHubProviderSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(api_base_url) = self.api_base_url.as_deref()
            && !api_base_url.is_empty()
        {
            if let Err(err) = uptrakit_shared_types::validate_provider_api_base_url(api_base_url) {
                return Err(ValidationError {
                    field: "api_base_url",
                    message: err.to_string(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_provider_github_validate_accepts_empty_update() {
        let req = UpdateGitHubProviderSettingsRequest {
            auth_token: None,
            api_base_url: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn settings_provider_github_validate_accepts_clear_api_base_url() {
        let req = UpdateGitHubProviderSettingsRequest {
            auth_token: None,
            api_base_url: Some(String::new()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn settings_provider_github_validate_rejects_invalid_api_base_url() {
        let req = UpdateGitHubProviderSettingsRequest {
            auth_token: None,
            api_base_url: Some("http://localhost/api/v3".to_string()),
        };
        let err = req
            .validate()
            .expect_err("must reject private/non-https URL");
        assert_eq!(err.field, "api_base_url");
    }

    #[test]
    fn settings_provider_github_validate_accepts_valid_api_base_url() {
        let req = UpdateGitHubProviderSettingsRequest {
            auth_token: None,
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn settings_provider_github_request_serialization_omits_absent_fields() {
        let req = UpdateGitHubProviderSettingsRequest {
            auth_token: None,
            api_base_url: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert!(json.get("auth_token").is_none());
        assert!(json.get("api_base_url").is_none());
    }

    #[test]
    fn settings_provider_github_response_serialization() {
        let resp = GitHubProviderSettingsResponse {
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
            has_auth_token: true,
            auth_token: Some("***".to_string()),
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["api_base_url"], "https://ghe.example.com/api/v3");
        assert_eq!(json["has_auth_token"], true);
        assert_eq!(json["auth_token"], "***");
    }
}
