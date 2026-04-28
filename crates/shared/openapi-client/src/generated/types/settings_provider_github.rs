// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! Request/response types for global GitHub provider settings.
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
/// Response body for `GET /api/v1/global-settings/providers/github`.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
            && let Err(err) =
                crate::generated::shared_types::validate_provider_api_base_url(api_base_url)
        {
            return Err(ValidationError {
                field: "api_base_url",
                message: err.to_string(),
            });
        }
        Ok(())
    }
}
